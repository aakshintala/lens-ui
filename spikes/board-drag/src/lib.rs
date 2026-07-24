//! Spike (B-4c): reverse hit-test for board drag-drop.
//!
//! ## The unknown this de-risks
//!
//! The board packer (`lens_core::pack`) is **forward-only**: an ordinal list packs into
//! pixel positions via shortest-column backfill. There is no closed-form inverse, and —
//! the sharp edge — ordinal order is **not spatially monotonic**: a later card backfills a
//! short column beside a tall group and can sit *physically above* an earlier one. So
//! "the user dropped here" → "insert at ordinal N" cannot be a formula; it must be a
//! **scan over the already-placed tiles**. This spike implements that scan and pins the
//! non-monotonic case in a test so the B-4c design starts from a proven resolver.
//!
//! ## What it deliberately does NOT cover
//!
//! - gpui wiring (`on_drag`/`on_drop`): mechanics are confirmed at the source level
//!   (hitbox-based dispatch, orthogonal to absolute positioning) — proof belongs in the
//!   B-4c real-window build, not here.
//! - Coordinate transforms from window→content space: in production, binding `on_drop` to
//!   the `content` element makes `cursor − bounds.origin` content-local for free (bounds
//!   are painted, already scrolled/offset). This spike works directly in content space.
//! - The insertion **convention** on masonry (nearest-tile vs marker vs reflow-preview) is
//!   a design decision for the brainstorm; this spike implements the simplest defensible
//!   one (nearest tile + reading-order side) so there is something concrete to react to.

use lens_core::pack::{self, CARD_H, CARD_W, CELL_W, GAP, HEADER, Item, Kind};

/// Spike-local id (production uses `BoardItemId`).
pub type Id = &'static str;

/// A top-level board entry (mirrors the `board_tree` shape `pack_and_render` consumes).
#[derive(Clone, Debug)]
pub enum TopItem {
    Card(Id),
    Group {
        id: Id,
        members: Vec<Id>,
        collapsed: bool,
    },
}

impl TopItem {
    fn to_pack_item(&self) -> Item {
        match self {
            TopItem::Card(_) => Item::card(),
            TopItem::Group {
                members, collapsed, ..
            } => {
                if *collapsed {
                    Item::group_collapsed(members.len())
                } else {
                    Item::group(members.len())
                }
            }
        }
    }
}

/// Where a drop resolves. Mirrors the args `BoardLayout::move_item` consumes
/// (`new_parent`, `new_ordinal`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTarget {
    /// `None` = top level; `Some(group id)` = drop into that group's member list.
    pub parent: Option<Id>,
    /// Insertion index in the parent's sibling list, **including** the dragged item if it
    /// is already a sibling ("insert before whatever currently sits at this ordinal").
    /// This is the *spatial* answer. `move_item` wants the index in the list with the
    /// dragged item removed — see [`to_move_ordinal`], which is a separate, testable step.
    pub ordinal: usize,
}

/// Translate the spatial insert-before ordinal into the index `move_item` expects (the
/// dragged item is removed from its sibling list *before* the insert). Only shifts when the
/// drag stays within the same parent and the dragged item sat before the target; a
/// cross-parent move needs no shift. Kept separate from [`resolve_drop`] because it is a
/// pure index convention, not geometry — and getting the two tangled is exactly the class
/// of off-by-one this spike exists to prevent.
pub fn to_move_ordinal(spatial_ordinal: usize, dragged_sibling_index: Option<usize>) -> usize {
    match dragged_sibling_index {
        Some(i) if i < spatial_ordinal => spatial_ordinal - 1,
        _ => spatial_ordinal,
    }
}

/// Resolve a content-space cursor to a drop target over the packed board.
///
/// Order of resolution:
/// 1. If the cursor is inside an **expanded group's body** (below its header), the drop is
///    *into* that group — its members form a clean row-major grid, so the ordinal is
///    spatially monotonic there (the easy case).
/// 2. Otherwise it is a **top-level** drop: pick the nearest top-level tile by rect
///    distance and insert before/after it by which side of its center the cursor is on
///    (reading order, y-dominant for a vertically-scrolling board). This is the scan that
///    stands in for the missing inverse.
pub fn resolve_drop(board: &[TopItem], cols: usize, cursor: (f32, f32)) -> DropTarget {
    let (cx, cy) = cursor;
    let items: Vec<Item> = board.iter().map(TopItem::to_pack_item).collect();
    let packing = pack::pack(&items, cols);

    // (1) into an expanded group's body?
    for placed in &packing.tiles {
        let Kind::Group { members } = placed.item.kind else {
            continue;
        };
        // Collapsed groups render as a 1×1 rollup with no member drop zone; a drop on one
        // resolves as a top-level tile (handled below). `board[item_index]` is safe: pack
        // preserves item order 1:1 with the input.
        let is_collapsed = matches!(
            &board[placed.item_index],
            TopItem::Group {
                collapsed: true,
                ..
            }
        );
        if is_collapsed || members == 0 {
            continue;
        }
        let (x0, y0) = (placed.cell_left(), placed.cell_top());
        let fc = placed.item.fc.max(1);
        let fr = placed.item.fr.max(1);
        let block_w = fc as f32 * CELL_W - GAP;
        let block_h = HEADER + fr as f32 * CARD_H + (fr as f32 - 1.0) * GAP;
        let body_top = y0 + HEADER;
        let in_body = cx >= x0 && cx <= x0 + block_w && cy >= body_top && cy <= y0 + block_h;
        if !in_body {
            continue;
        }
        let group_id = match &board[placed.item_index] {
            TopItem::Group { id, .. } => *id,
            TopItem::Card(_) => unreachable!("Kind::Group came from a TopItem::Group"),
        };
        let ordinal = member_ordinal(cx, cy, x0, y0, fc, members);
        return DropTarget {
            parent: Some(group_id),
            ordinal,
        };
    }

    // (2) top-level: nearest tile + reading-order side.
    let ordinal = top_level_ordinal(board, &packing, cx, cy);
    DropTarget {
        parent: None,
        ordinal,
    }
}

/// Insertion ordinal within an expanded group's member grid (row-major, tight
/// `CARD_H + GAP` stride — matches `absolute_group`). Members are a real grid here, so the
/// ordinal *is* spatially monotonic: locate the cell, then bias one past it when the cursor
/// is right of the cell's horizontal center (reading order). Clamped to `[0, members]`.
fn member_ordinal(cx: f32, cy: f32, x0: f32, y0: f32, fc: usize, members: usize) -> usize {
    let body_top = y0 + HEADER;
    let rows = members.div_ceil(fc);
    let col = (((cx - x0) / CELL_W).floor() as isize).clamp(0, fc as isize - 1) as usize;
    let row = (((cy - body_top) / (CARD_H + GAP)).floor() as isize).clamp(0, rows as isize - 1)
        as usize;
    let raw = row * fc + col;
    // A partially-filled last row leaves empty trailing cells. A cursor in one is spatially
    // *past* the last member, so it appends — computing before/after against the phantom
    // cell's own center (against a clamped-back real member in a different column) would
    // wrongly resolve to *before* that member (codex review, 2026-07-23).
    if raw >= members {
        return members;
    }
    let cell_center_x = x0 + col as f32 * CELL_W + CARD_W / 2.0;
    let after = cx > cell_center_x;
    (raw + usize::from(after)).min(members)
}

/// Nearest top-level tile by clamped rect distance, then insert before/after by the cursor's
/// side of that tile's center (y-dominant). Returns the spatial insert-before ordinal in the
/// full top-level list. Empty board → 0.
fn top_level_ordinal(board: &[TopItem], packing: &pack::Packing, cx: f32, cy: f32) -> usize {
    let mut best: Option<(f32, usize)> = None; // (distance², top-level ordinal)
    for placed in &packing.tiles {
        let (x0, y0) = (placed.cell_left(), placed.cell_top());
        let (w, h) = tile_size(placed);
        let d2 = rect_dist_sq(cx, cy, x0, y0, w, h);
        let ord = placed.item_index; // pack preserves input order 1:1 → item_index == ordinal
        if best.is_none_or(|(bd, _)| d2 < bd) {
            best = Some((d2, ord));
        }
    }
    let Some((_, k)) = best else {
        return 0; // empty board
    };
    // Side: before if the cursor is above the tile's vertical center, else after.
    let placed = &packing.tiles[k];
    let y0 = placed.cell_top();
    let (_w, h) = tile_size(placed);
    let center_y = y0 + h / 2.0;
    let after = cy > center_y;
    let _ = board; // (kept in signature for parity with production, which reads ids here)
    let ord = k + usize::from(after);
    ord.min(packing.tiles.len())
}

/// Rendered (w, h) of a placed tile — loose card or (expanded/collapsed) group box.
fn tile_size(placed: &pack::Placed) -> (f32, f32) {
    match placed.item.kind {
        Kind::Card => (CARD_W, CARD_H),
        Kind::Group { .. } => {
            let fc = placed.item.fc.max(1);
            let fr = placed.item.fr.max(1);
            (
                fc as f32 * CELL_W - GAP,
                HEADER + fr as f32 * CARD_H + (fr as f32 - 1.0) * GAP,
            )
        }
    }
}

/// Squared distance from a point to a rect (0 inside). Squared to avoid a sqrt in the scan.
fn rect_dist_sq(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32) -> f32 {
    let dx = (x - px).max(0.0).max(px - (x + w));
    let dy = (y - py).max(0.0).max(py - (y + h));
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: Id) -> TopItem {
        TopItem::Card(id)
    }
    fn group(id: Id, members: &[Id]) -> TopItem {
        TopItem::Group {
            id,
            members: members.to_vec(),
            collapsed: false,
        }
    }

    // --- top-level reorder, simple single column -----------------------------------------

    #[test]
    fn drop_above_center_inserts_before() {
        // 3 cards stacked in 1 col: py = 0, CARD_H+GAP, 2*(CARD_H+GAP).
        let board = [card("a"), card("b"), card("c")];
        // Cursor near the top of card "b" (its center is py1 + CARD_H/2).
        let py1 = CARD_H + GAP;
        let t = resolve_drop(&board, 1, (CARD_W / 2.0, py1 + 4.0));
        assert_eq!(t, DropTarget { parent: None, ordinal: 1 }); // before "b"
    }

    #[test]
    fn drop_below_center_inserts_after() {
        let board = [card("a"), card("b"), card("c")];
        let py1 = CARD_H + GAP;
        let t = resolve_drop(&board, 1, (CARD_W / 2.0, py1 + CARD_H - 4.0));
        assert_eq!(t, DropTarget { parent: None, ordinal: 2 }); // after "b"
    }

    #[test]
    fn drop_below_everything_appends() {
        let board = [card("a"), card("b")];
        let t = resolve_drop(&board, 1, (CARD_W / 2.0, 10_000.0));
        assert_eq!(t.parent, None);
        assert_eq!(t.ordinal, 2); // end of list
    }

    // --- THE crux: masonry non-monotonicity ----------------------------------------------

    #[test]
    fn ordinal_is_not_spatial_order_under_backfill() {
        // group(4) [2×2] at ordinal 0, then two loose cards. In 3 cols the cards backfill
        // column 2 beside the group: card "x" (ord 1) at py=0, card "y" (ord 2) below it.
        // So the tile that is spatially HIGHEST-right (ord 1) is NOT ordinal 0 — proving
        // the resolver must read placement, not assume reading-order == ordinal.
        let board = [group("g", &["m0", "m1", "m2", "m3"]), card("x"), card("y")];
        // Cursor over card "x": column 2 (x ≈ 2*CELL_W), near its top.
        let x_col2 = 2.0 * CELL_W + CARD_W / 2.0;
        let t = resolve_drop(&board, 3, (x_col2, 4.0));
        // Nearest tile is "x" (ordinal 1), cursor above its center → insert before it.
        assert_eq!(t, DropTarget { parent: None, ordinal: 1 });

        // A cursor at the same x but low (over card "y", ordinal 2) must resolve to 2/3 —
        // NOT to "adjacent to the group" despite the group being the ordinal-0 neighbor.
        let py_y = CARD_H + GAP;
        let t_low = resolve_drop(&board, 3, (x_col2, py_y + 4.0));
        assert_eq!(t_low, DropTarget { parent: None, ordinal: 2 }); // before "y"
    }

    // --- into a group body ---------------------------------------------------------------

    #[test]
    fn drop_into_expanded_group_body_targets_member_slot() {
        let board = [group("g", &["m0", "m1", "m2", "m3"])]; // 2×2 at origin
        // Member m0 body cell: origin (0, HEADER); center x = CARD_W/2.
        // Cursor just left of m0 center → before m0 (ordinal 0).
        let t = resolve_drop(&board, 3, (4.0, HEADER + CARD_H / 2.0));
        assert_eq!(t, DropTarget { parent: Some("g"), ordinal: 0 });

        // Cursor right of m1 center (col 1, row 0) → after m1 (ordinal 2).
        let x_m1 = CELL_W + CARD_W - 4.0;
        let t2 = resolve_drop(&board, 3, (x_m1, HEADER + CARD_H / 2.0));
        assert_eq!(t2, DropTarget { parent: Some("g"), ordinal: 2 });
    }

    #[test]
    fn empty_trailing_cell_in_partial_last_row_appends() {
        // 5-member group in 3 cols → 3×2, last row half-full: m3(r1c0) m4(r1c1) [empty r1c2].
        // A cursor over that empty r1c2 cell is spatially past m4 → append (ordinal 5), NOT
        // "before m4" (the clamp-to-last-member + phantom-center bug codex caught 2026-07-23).
        let board = [group("g", &["m0", "m1", "m2", "m3", "m4"])];
        let t = resolve_drop(&board, 3, (618.0, 218.0)); // codex's exact failing input
        assert_eq!(t, DropTarget { parent: Some("g"), ordinal: 5 });

        // The last *real* member in a partial row still resolves before/after by its own
        // center: cursor left of m4 (r1c1) → before it (4); right of m4 → after it (5).
        let m4_center_x = CELL_W + CARD_W / 2.0;
        let y_row1 = HEADER + (CARD_H + GAP) + CARD_H / 2.0;
        let left = resolve_drop(&board, 3, (m4_center_x - 20.0, y_row1));
        assert_eq!(left, DropTarget { parent: Some("g"), ordinal: 4 });
        let right = resolve_drop(&board, 3, (m4_center_x + 20.0, y_row1));
        assert_eq!(right, DropTarget { parent: Some("g"), ordinal: 5 });
    }

    #[test]
    fn drop_on_group_header_is_top_level_not_into_group() {
        let board = [card("a"), group("g", &["m0", "m1"])];
        // Header band of the group sits at py of the group tile. In 3 cols, card "a" is at
        // (0,0) col0; the group (fc=2) backfills... actually pack places group first? No —
        // input order: card "a" ord 0 then group ord 1. Card at col0 py0; group(2) → 2×1,
        // needs 2 cols, placed at gx=1? shortest-col: col0 bottom=CARD_H, cols1..2 =0 → gx1.
        // Group header y = 0. Cursor on header (y < HEADER) → top-level, not into group.
        let gx = 1.0 * CELL_W;
        let t = resolve_drop(&board, 3, (gx + 10.0, 4.0));
        assert_eq!(t.parent, None, "header drop must not enter the group");
    }

    #[test]
    fn collapsed_group_has_no_member_drop_zone() {
        let board = [TopItem::Group {
            id: "g",
            members: vec!["m0", "m1", "m2"],
            collapsed: true,
        }];
        // Anywhere over the 1×1 collapsed tile resolves top-level (can't drop into it).
        let t = resolve_drop(&board, 3, (CARD_W / 2.0, HEADER + CARD_H / 2.0));
        assert_eq!(t.parent, None);
    }

    // --- move_ordinal index convention ---------------------------------------------------

    #[test]
    fn move_ordinal_shifts_when_dragged_precedes_target() {
        // Dragged sits at sibling index 1, dropped before ordinal 3 (same parent):
        // after removal the target index is 2.
        assert_eq!(to_move_ordinal(3, Some(1)), 2);
        // Dropped before its own position or earlier → no shift.
        assert_eq!(to_move_ordinal(1, Some(1)), 1);
        assert_eq!(to_move_ordinal(0, Some(3)), 0);
        // Cross-parent (dragged not a sibling) → never shifts.
        assert_eq!(to_move_ordinal(3, None), 3);
    }

    #[test]
    fn empty_board_resolves_to_zero() {
        let board: [TopItem; 0] = [];
        let t = resolve_drop(&board, 3, (100.0, 100.0));
        assert_eq!(t, DropTarget { parent: None, ordinal: 0 });
    }
}
