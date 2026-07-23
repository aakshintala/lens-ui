//! Pure board packer (B-2): grid-snap first-fit with hole-backfill, ordinal
//! order. Ported verbatim from the GO spike `spikes/board-container/src/packer.rs`
//! (itself a port of `pack()`/`foot()` in the pixel SSOT
//! `docs/design/renders/board-home.html`). No gpui — pure, deterministic, testable.
//! Geometry constants duplicate `card::model` px values across the layer boundary
//! intentionally (lens-core stays gpui-free). Tune on device (spec §8).

// Geometry constants — mockup values from the SSOT (§2.1).
pub const CARD_W: f32 = 280.0;
pub const CARD_H: f32 = 160.0;
pub const HEADER: f32 = 24.0;
pub const GAP: f32 = 24.0;
pub const INSET: f32 = 5.0;

pub const CELL_W: f32 = CARD_W + GAP; // 296
pub const CELL_H: f32 = CARD_H + HEADER + GAP; // 200 — [header-lane][card-body][gap]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A loose 1×1 card.
    Card,
    /// A group tile spanning `fc × fr` cells with `members` full-size cards.
    Group { members: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub kind: Kind,
    pub fc: usize,
    pub fr: usize,
}

impl Item {
    pub fn card() -> Self {
        Item {
            kind: Kind::Card,
            fc: 1,
            fr: 1,
        }
    }

    pub fn group(members: usize) -> Self {
        let (fc, fr) = foot(members);
        Item {
            kind: Kind::Group { members },
            fc,
            fr,
        }
    }

    /// A collapsed group: a 1×1 tile (§7) — the footprint is overridden to a single
    /// cell regardless of member count (the collapsed body shows a status rollup, not
    /// the members). `members` is retained for `Kind` symmetry.
    pub fn group_collapsed(members: usize) -> Self {
        Item {
            kind: Kind::Group { members },
            fc: 1,
            fr: 1,
        }
    }
}

/// Reshape a group whose natural width (`fc`) exceeds the container's `cols`: clamp columns
/// to `cols` and re-derive rows from member count (`fr' = ⌈members / fc'⌉`) so it packs and
/// renders as a 1×N (or k×N) stack. Loose cards, collapsed groups (fc already 1), and groups
/// that already fit are returned unchanged.
fn reshape_to_cols(it: &Item, cols: usize) -> Item {
    match it.kind {
        Kind::Group { members } if it.fc > cols => {
            let fc = it.fc.min(cols).max(1);
            let fr = members.div_ceil(fc).max(1);
            Item {
                kind: it.kind,
                fc,
                fr,
            }
        }
        _ => *it,
    }
}

/// `foot(n)`: n≤3 → n×1; n≥4 → ⌈√n⌉ cols × ⌈n/c⌉ rows (§2.2).
pub fn foot(n: usize) -> (usize, usize) {
    if n <= 3 {
        return (n.max(1), 1);
    }
    let c = (n as f64).sqrt().ceil() as usize;
    (c, n.div_ceil(c))
}

/// A packed tile: the source item, its column (`gx`, horizontal grid) and pixel top (`py`).
/// Vertical placement is pixel-masonry (variable tile heights, uniform GAP between every tile),
/// NOT a snapped row grid — a group is simply a taller tile (header + members), so its extra
/// header height never inflates the gaps around it.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub item: Item,
    pub item_index: usize,
    pub gx: usize,
    pub py: f32,
}

pub struct Packing {
    pub tiles: Vec<Placed>,
    pub content_height: f32,
}

impl Packing {
    /// Number of columns actually occupied by placed tiles (`max(gx + fc)`), ≥ 1.
    /// May be less than the pack `cols` when few tiles were placed — centering keys
    /// on this so a MAX_COLS-capped board with a handful of sessions centers on the
    /// occupied width instead of reserving empty trailing columns. `fc` is guarded to
    /// ≥ 1 against a degenerate hand-built zero footprint.
    pub fn used_cols(&self) -> usize {
        self.tiles
            .iter()
            .map(|t| t.gx + t.item.fc.max(1))
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

/// Rendered pixel height of a tile: a loose card is `CARD_H`; a group is its one header lane
/// plus `fr` member rows separated by GAP (`HEADER + fr·CARD_H + (fr−1)·GAP`); a collapsed
/// group (fr = 1) is `HEADER + CARD_H`. SINGLE source of tile height — pack() stacks by it and
/// `absolute_group`'s box height must match it.
pub fn item_height(it: &Item) -> f32 {
    match it.kind {
        Kind::Card => CARD_H,
        Kind::Group { .. } => {
            let fr = it.fr.max(1) as f32;
            HEADER + fr * CARD_H + (fr - 1.0) * GAP
        }
    }
}

/// Pixel-masonry packer: keep the horizontal column grid (`fc` consecutive columns; groups may
/// span 2×2 etc.), but place each tile at the lowest pixel top across its columns (+ GAP), so
/// loose tiles backfill the short columns beside a tall group and every vertical gap is exactly
/// GAP. Leftmost column wins on ties → ordinal-stable placement.
pub fn pack(items: &[Item], cols: usize) -> Packing {
    let cols = cols.max(1);
    let mut col_bottom = vec![0.0f32; cols];
    let mut out = Vec::with_capacity(items.len());

    for (item_index, it) in items.iter().enumerate() {
        // Reshape a group wider than the container: clamp columns to `cols` and re-derive rows
        // from member count so a 2×2 becomes a 1×N stack instead of spilling past the container
        // (narrow rail / shrunk window). The reshaped footprint is stored for the renderer.
        let it = reshape_to_cols(it, cols);
        let fc = it.fc.max(1).min(cols);
        let h = item_height(&it);

        let mut best_gx = 0usize;
        let mut best_top = f32::INFINITY;
        for gx in 0..=(cols - fc) {
            let bottom = (gx..gx + fc).map(|c| col_bottom[c]).fold(0.0, f32::max);
            let top = if bottom > 0.0 { bottom + GAP } else { 0.0 };
            if top < best_top {
                best_top = top;
                best_gx = gx;
            }
        }
        let py = best_top;
        for bottom in &mut col_bottom[best_gx..best_gx + fc] {
            *bottom = py + h;
        }
        out.push(Placed {
            item: it,
            item_index,
            gx: best_gx,
            py,
        });
    }

    let content_height = col_bottom.iter().copied().fold(0.0, f32::max);
    Packing {
        tiles: out,
        content_height,
    }
}

/// Cols that fit in `avail_width` (§2.3: `floor((avail+GAP)/CELL_W)`).
pub fn cols_for_width(avail_width: f32) -> usize {
    (((avail_width + GAP) / CELL_W).floor() as usize).max(1)
}

/// Content-width cap as a max column count that grows in steps with the viewport's
/// logical width, then plateaus. Cards are a fixed `CARD_W`, so "max columns" is
/// equivalently a max content px-width (`max_cols·CELL_W − GAP`); this damps it so a
/// wide/ultrawide screen earns *some* extra columns but never fans a handful of
/// sessions edge-to-edge — the packed block is centered in the leftover width (§8).
/// `logical_w` is the full viewport width in gpui logical points; breakpoints are
/// tuned on-device against 1800/2056/3840-wide screens. The `6` plateau is the
/// deliberate ultrawide ceiling. Callers apply it via
/// `cols_for_width(avail).min(max_cols_for_width(viewport_w))`.
pub fn max_cols_for_width(logical_w: f32) -> usize {
    if logical_w >= 3400.0 {
        6
    } else if logical_w >= 2000.0 {
        5
    } else if logical_w >= 1400.0 {
        4
    } else {
        3
    }
}

impl Placed {
    /// Pixel top of the tile (group box top / loose card top).
    pub fn cell_top(&self) -> f32 {
        self.py
    }

    /// Pixel bottom of the tile (top + rendered height) — for culling y-intersection.
    pub fn cell_bottom(&self) -> f32 {
        self.py + item_height(&self.item)
    }

    /// Left of the tile's column block.
    pub fn cell_left(&self) -> f32 {
        self.gx as f32 * CELL_W
    }

    /// Does this tile's y-range intersect the visible band `[lo, hi]`? The cull
    /// predicate (spec §4 unknown 2): tiles that fail this are absent from the
    /// child vec, so gpui never builds them.
    pub fn intersects_band(&self, lo: f32, hi: f32) -> bool {
        self.cell_bottom() >= lo && self.cell_top() <= hi
    }
}

use crate::domain::ids::BoardItemId;

#[derive(Clone, Debug)]
pub struct DropTile {
    pub placed: Placed,
    pub id: BoardItemId,
    /// True iff collapsed group — no member drop zone (§4.2 step 3).
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraggedKind {
    Card,
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTarget {
    pub parent: Option<BoardItemId>,
    pub ordinal: usize,
}

pub fn to_move_ordinal(spatial_ordinal: usize, dragged_sibling_index: Option<usize>) -> usize {
    match dragged_sibling_index {
        Some(i) if i < spatial_ordinal => spatial_ordinal - 1,
        _ => spatial_ordinal,
    }
}

/// `snapshot` = frozen S: packed tiles with the dragged item already removed (§4.1).
pub fn resolve_drop(
    snapshot: &[DropTile],
    cursor: (f32, f32),
    dragged: DraggedKind,
) -> DropTarget {
    let (cx, cy) = cursor;
    // (1) into expanded group body — card drags only (§6).
    if dragged == DraggedKind::Card {
        for tile in snapshot {
            let Kind::Group { members } = tile.placed.item.kind else {
                continue;
            };
            if tile.collapsed || members == 0 {
                continue;
            }
            let (x0, y0) = (tile.placed.cell_left(), tile.placed.cell_top());
            let fc = tile.placed.item.fc.max(1);
            let fr = tile.placed.item.fr.max(1);
            let block_w = fc as f32 * CELL_W - GAP;
            let block_h = HEADER + fr as f32 * CARD_H + (fr as f32 - 1.0) * GAP;
            let body_top = y0 + HEADER;
            let in_body = cx >= x0 && cx <= x0 + block_w && cy >= body_top && cy <= y0 + block_h;
            if !in_body {
                continue;
            }
            return DropTarget {
                parent: Some(tile.id.clone()),
                ordinal: member_ordinal(cx, cy, x0, y0, fc, members),
            };
        }
    }
    // (2) top-level nearest + reading-order side.
    DropTarget {
        parent: None,
        ordinal: top_level_ordinal(snapshot, cx, cy),
    }
}

fn member_ordinal(cx: f32, cy: f32, x0: f32, y0: f32, fc: usize, members: usize) -> usize {
    let body_top = y0 + HEADER;
    let rows = members.div_ceil(fc);
    let col = (((cx - x0) / CELL_W).floor() as isize)
        .clamp(0, fc as isize - 1) as usize;
    let row = (((cy - body_top) / (CARD_H + GAP)).floor() as isize)
        .clamp(0, rows as isize - 1) as usize;
    let raw = row * fc + col;
    if raw >= members {
        return members; // empty trailing cell → append (codex 2026-07-23)
    }
    let cell_center_x = x0 + col as f32 * CELL_W + CARD_W / 2.0;
    let after = cx > cell_center_x;
    (raw + usize::from(after)).min(members)
}

fn top_level_ordinal(snapshot: &[DropTile], cx: f32, cy: f32) -> usize {
    let mut best: Option<(f32, usize)> = None;
    for (i, tile) in snapshot.iter().enumerate() {
        let (x0, y0) = (tile.placed.cell_left(), tile.placed.cell_top());
        let (w, h) = tile_size(&tile.placed);
        let d2 = rect_dist_sq(cx, cy, x0, y0, w, h);
        if best.is_none_or(|(bd, _)| d2 < bd) {
            best = Some((d2, i));
        }
    }
    let Some((_, k)) = best else {
        return 0;
    };
    let placed = &snapshot[k].placed;
    let y0 = placed.cell_top();
    let (_w, h) = tile_size(placed);
    let after = cy > y0 + h / 2.0;
    (k + usize::from(after)).min(snapshot.len())
}

fn tile_size(placed: &Placed) -> (f32, f32) {
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

fn rect_dist_sq(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32) -> f32 {
    let dx = (x - px).max(0.0).max(px - (x + w));
    let dy = (y - py).max(0.0).max(py - (y + h));
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::BoardItemId;

    fn bid(s: &str) -> BoardItemId {
        BoardItemId::new(s)
    }

    /// Pack items, zip parallel ids + collapsed flags → DropTile snapshot (S).
    fn snap(items: &[Item], ids: &[&str], collapsed: &[bool], cols: usize) -> Vec<DropTile> {
        let packing = pack(items, cols);
        packing
            .tiles
            .into_iter()
            .map(|placed| DropTile {
                id: bid(ids[placed.item_index]),
                collapsed: collapsed[placed.item_index],
                placed,
            })
            .collect()
    }

    #[test]
    fn drop_above_center_inserts_before() {
        let items = [Item::card(), Item::card(), Item::card()];
        let s = snap(&items, &["a", "b", "c"], &[false; 3], 1);
        let py1 = CARD_H + GAP;
        let t = resolve_drop(&s, (CARD_W / 2.0, py1 + 4.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: None,
            ordinal: 1
        });
    }

    #[test]
    fn drop_below_center_inserts_after() {
        let items = [Item::card(), Item::card(), Item::card()];
        let s = snap(&items, &["a", "b", "c"], &[false; 3], 1);
        let py1 = CARD_H + GAP;
        let t = resolve_drop(&s, (CARD_W / 2.0, py1 + CARD_H - 4.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: None,
            ordinal: 2
        });
    }

    #[test]
    fn drop_below_everything_resolves_after_nearest() {
        let items = [Item::card(), Item::card()];
        let s = snap(&items, &["a", "b"], &[false; 2], 1);
        let t = resolve_drop(&s, (CARD_W / 2.0, 10_000.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: None,
            ordinal: 2
        });
    }

    #[test]
    fn ordinal_is_not_spatial_order_under_backfill() {
        let items = [Item::group(4), Item::card(), Item::card()];
        let s = snap(&items, &["g", "x", "y"], &[false; 3], 3);
        let x_col2 = 2.0 * CELL_W + CARD_W / 2.0;
        let t = resolve_drop(&s, (x_col2, 4.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: None,
            ordinal: 1
        });
        let py_y = CARD_H + GAP;
        let t_low = resolve_drop(&s, (x_col2, py_y + 4.0), DraggedKind::Card);
        assert_eq!(t_low, DropTarget {
            parent: None,
            ordinal: 2
        });
    }

    #[test]
    fn drop_into_expanded_group_body_targets_member_slot() {
        let items = [Item::group(4)];
        let s = snap(&items, &["g"], &[false], 3);
        let t = resolve_drop(&s, (4.0, HEADER + CARD_H / 2.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: Some(bid("g")),
            ordinal: 0
        });
        let x_m1 = CELL_W + CARD_W - 4.0;
        let t2 = resolve_drop(&s, (x_m1, HEADER + CARD_H / 2.0), DraggedKind::Card);
        assert_eq!(t2, DropTarget {
            parent: Some(bid("g")),
            ordinal: 2
        });
    }

    #[test]
    fn empty_trailing_cell_in_partial_last_row_appends() {
        let items = [Item::group(5)];
        let s = snap(&items, &["g"], &[false], 3);
        let t = resolve_drop(&s, (618.0, 218.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: Some(bid("g")),
            ordinal: 5
        });
        let m4_center_x = CELL_W + CARD_W / 2.0;
        let y_row1 = HEADER + (CARD_H + GAP) + CARD_H / 2.0;
        assert_eq!(
            resolve_drop(&s, (m4_center_x - 20.0, y_row1), DraggedKind::Card),
            DropTarget {
                parent: Some(bid("g")),
                ordinal: 4
            }
        );
        assert_eq!(
            resolve_drop(&s, (m4_center_x + 20.0, y_row1), DraggedKind::Card),
            DropTarget {
                parent: Some(bid("g")),
                ordinal: 5
            }
        );
    }

    #[test]
    fn drop_on_group_header_is_top_level_not_into_group() {
        let items = [Item::card(), Item::group(2)];
        let s = snap(&items, &["a", "g"], &[false; 2], 3);
        let gx = 1.0 * CELL_W;
        let t = resolve_drop(&s, (gx + 10.0, 4.0), DraggedKind::Card);
        assert_eq!(t.parent, None, "header drop must not enter the group");
    }

    #[test]
    fn collapsed_group_has_no_member_drop_zone() {
        let items = [Item::group_collapsed(3)];
        let s = snap(&items, &["g"], &[true], 3);
        let t = resolve_drop(
            &s,
            (CARD_W / 2.0, HEADER + CARD_H / 2.0),
            DraggedKind::Card,
        );
        assert_eq!(t.parent, None);
    }

    #[test]
    fn dragged_group_over_group_body_falls_through_to_top_level() {
        let items = [Item::group(4), Item::group(2)];
        let s = snap(&items, &["g0", "g1"], &[false; 2], 3);
        let t = resolve_drop(
            &s,
            (4.0, HEADER + CARD_H / 2.0),
            DraggedKind::Group,
        );
        assert_eq!(t.parent, None, "group drag must not nest under another group");
    }

    #[test]
    fn move_ordinal_shifts_when_dragged_precedes_target() {
        assert_eq!(to_move_ordinal(3, Some(1)), 2);
        assert_eq!(to_move_ordinal(1, Some(1)), 1);
        assert_eq!(to_move_ordinal(0, Some(3)), 0);
        assert_eq!(to_move_ordinal(3, None), 3);
    }

    #[test]
    fn empty_board_resolves_to_zero() {
        let s: [DropTile; 0] = [];
        let t = resolve_drop(&s, (100.0, 100.0), DraggedKind::Card);
        assert_eq!(t, DropTarget {
            parent: None,
            ordinal: 0
        });
    }

    #[test]
    fn foot_anchors() {
        assert_eq!(foot(1), (1, 1));
        assert_eq!(foot(2), (2, 1));
        assert_eq!(foot(3), (3, 1));
        assert_eq!(foot(4), (2, 2));
        assert_eq!(foot(6), (3, 2));
        assert_eq!(foot(9), (3, 3));
    }

    fn group_h(fr: usize) -> f32 {
        HEADER + fr as f32 * CARD_H + (fr as f32 - 1.0) * GAP
    }

    #[test]
    fn masonry_backfill() {
        // A 2×2 group then two loose cards in 3 cols: the cards backfill the short 3rd
        // column beside the group, stacked with a uniform GAP; the tall group sets height.
        let p = pack(&[Item::group(4), Item::card(), Item::card()], 3);
        assert_eq!(p.tiles[0].gx, 0); // group top-left
        assert_eq!(p.tiles[0].py, 0.0);
        assert_eq!(p.tiles[1].gx, 2); // card fills col 2
        assert_eq!(p.tiles[1].py, 0.0);
        assert_eq!(p.tiles[2].gx, 2); // stacked one GAP below the first card
        assert_eq!(p.tiles[2].py, CARD_H + GAP);
        assert_eq!(p.content_height, group_h(2));
    }

    #[test]
    fn single_col_stacks_by_pixel_height() {
        let p = pack(&[Item::card(), Item::card(), Item::card()], 1);
        assert_eq!(p.tiles[0].py, 0.0);
        assert_eq!(p.tiles[1].py, CARD_H + GAP);
        assert_eq!(p.tiles[2].py, 2.0 * (CARD_H + GAP));
        assert_eq!(p.content_height, 3.0 * CARD_H + 2.0 * GAP);
    }

    #[test]
    fn cols_for_width_anchors() {
        assert_eq!(cols_for_width(0.0), 1); // clamped to ≥1
        assert_eq!(cols_for_width(940.0), 3); // SSOT window
        assert_eq!(cols_for_width(CELL_W - GAP), 1); // exactly one card wide
    }

    #[test]
    fn max_cols_for_width_breakpoints() {
        // The three on-device screens (logical points) + the plateau/floor.
        assert_eq!(max_cols_for_width(1800.0), 4); // 14" MBP "More Space"
        assert_eq!(max_cols_for_width(2056.0), 5); // 16" MBP "More Space"
        assert_eq!(max_cols_for_width(3840.0), 6); // 4K external
        assert_eq!(max_cols_for_width(1399.0), 3); // below first step → floor
        assert_eq!(max_cols_for_width(6000.0), 6); // ultrawide plateaus at the ceiling
        // Exact breakpoint edges are inclusive (>=).
        assert_eq!(max_cols_for_width(1400.0), 4);
        assert_eq!(max_cols_for_width(2000.0), 5);
        assert_eq!(max_cols_for_width(3400.0), 6);
    }

    #[test]
    fn used_cols_reflects_occupancy_not_capacity() {
        // Three loose cards in a 6-col pack occupy only 3 columns → centering keys on 3.
        let p = pack(&[Item::card(), Item::card(), Item::card()], 6);
        assert_eq!(p.used_cols(), 3);
        // A 2×2 group + one backfill card spans through column 2 (gx 2 + fc 1).
        let g = pack(&[Item::group(4), Item::card()], 6);
        assert_eq!(g.used_cols(), 3);
        // Empty packing is a safe 1.
        assert_eq!(pack(&[], 4).used_cols(), 1);
    }

    #[test]
    fn pack_clamps_degenerate_footprint() {
        // A hand-built zero footprint must not panic and packs as a 1×1.
        let items = [Item {
            kind: Kind::Card,
            fc: 0,
            fr: 0,
        }];
        let p = pack(&items, 3);
        assert_eq!(p.tiles.len(), 1);
        assert_eq!(p.tiles[0].gx, 0);
        assert_eq!(p.tiles[0].py, 0.0);
        assert_eq!(p.content_height, CARD_H);
    }

    #[test]
    fn group_reflows_to_narrow_container() {
        // A 4-member group is 2×2 at full width, but in a 1-col container (focused rail
        // or a shrunk window) it must reshape to 1×4 — NOT render 2-wide and spill/clip
        // past the container (Issue 2). The stored item carries the reshaped footprint so
        // `absolute_group` places members by `i % fc` into the narrow stack.
        let p = pack(&[Item::group(4)], 1);
        let t = p.tiles[0];
        assert_eq!((t.item.fc, t.item.fr), (1, 4));
        assert_eq!(t.gx, 0);
        assert_eq!(t.py, 0.0);
        assert_eq!(p.content_height, group_h(4)); // 1×4 tight stack
    }

    #[test]
    fn focused_rail_group_reflows_to_one_col() {
        // Spec §7 / B-4b carried minor: focused rail is pack_and_render at cols=1.
        // A natural 2×2 (4 members) must reshape to 1×4 and store the clamped fc on
        // Placed.item so absolute_group lays members into the narrow stack.
        let p = pack(&[Item::group(4)], 1);
        let t = &p.tiles[0];
        assert_eq!((t.item.fc, t.item.fr), (1, 4));
        assert_eq!(t.gx, 0);
        assert_eq!(t.py, 0.0);
        // Height = HEADER + 4*CARD_H + 3*GAP (tight 1×4 stack).
        assert_eq!(
            p.content_height,
            HEADER + 4.0 * CARD_H + 3.0 * GAP
        );
    }

    #[test]
    fn group_reflows_to_two_cols() {
        // A 9-member group is 3×3 at full width; in a 2-col container it reshapes to
        // 2×5 (⌈9/2⌉ = 5 rows), the last row half-full.
        let p = pack(&[Item::group(9)], 2);
        let t = p.tiles[0];
        assert_eq!((t.item.fc, t.item.fr), (2, 5));
    }

    #[test]
    fn group_keeps_footprint_when_it_fits() {
        // Wide enough: the natural 2×2 footprint is preserved (no spurious reshape).
        let p = pack(&[Item::group(4)], 4);
        let t = p.tiles[0];
        assert_eq!((t.item.fc, t.item.fr), (2, 2));
    }

    #[test]
    fn collapsed_group_packs_one_by_one() {
        // A collapsed group is a 1×1 tile no matter how many members it has (§7).
        let items = [Item::group_collapsed(9)];
        let packing = pack(&items, 3);
        assert_eq!(packing.tiles.len(), 1);
        let t = packing.tiles[0];
        assert_eq!((t.item.fc, t.item.fr), (1, 1));
        assert!(matches!(t.item.kind, Kind::Group { members: 9 }));
        assert_eq!(t.gx, 0);
        assert_eq!(t.py, 0.0);
        // header lane + one card body.
        assert_eq!(packing.content_height, HEADER + CARD_H);
    }

    #[test]
    fn intersects_band_culls_outside() {
        // A loose card spans [0, CARD_H]; a band far below misses it.
        let p = pack(&[Item::card()], 3);
        let t = p.tiles[0];
        assert!(t.intersects_band(0.0, CARD_H)); // visible band covers it
        assert!(!t.intersects_band(CARD_H * 5.0, CARD_H * 6.0)); // far below → culled
    }
}
