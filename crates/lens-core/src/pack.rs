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

#[cfg(test)]
mod tests {
    use super::*;

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
