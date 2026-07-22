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
pub const GAP: f32 = 16.0;
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

/// `foot(n)`: n≤3 → n×1; n≥4 → ⌈√n⌉ cols × ⌈n/c⌉ rows (§2.2).
pub fn foot(n: usize) -> (usize, usize) {
    if n <= 3 {
        return (n.max(1), 1);
    }
    let c = (n as f64).sqrt().ceil() as usize;
    (c, n.div_ceil(c))
}

/// A packed tile: the source item + its top-left grid cell.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub item: Item,
    pub item_index: usize,
    pub gx: usize,
    pub gy: usize,
}

pub struct Packing {
    pub tiles: Vec<Placed>,
    pub content_height: f32,
}

/// Grid-snap first-fit, ordinal order, hole-backfill (§2.3). Direct port of the
/// SSOT `pack()` — scan rows top→bottom, cols left→right, place at first free
/// `fc × fr` block, mark occupied.
pub fn pack(items: &[Item], cols: usize) -> Packing {
    let cols = cols.max(1);
    let mut occ: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::with_capacity(items.len());
    let mut max_r = 0usize;

    let ensure = |occ: &mut Vec<Vec<bool>>, r: usize| {
        while occ.len() <= r {
            occ.push(vec![false; cols]);
        }
    };

    for (item_index, it) in items.iter().enumerate() {
        let (fc, fr) = (it.fc.max(1).min(cols), it.fr.max(1));
        let mut placed = false;
        let mut r = 0usize;
        while !placed {
            ensure(&mut occ, r + fr - 1);
            for c in 0..=(cols - fc) {
                if free(&occ, r, c, fc, fr) {
                    mark(&mut occ, r, c, fc, fr);
                    out.push(Placed {
                        item: *it,
                        item_index,
                        gx: c,
                        gy: r,
                    });
                    max_r = max_r.max(r + fr);
                    placed = true;
                    break;
                }
            }
            r += 1;
        }
    }

    let content_height = if max_r == 0 {
        0.0
    } else {
        max_r as f32 * CELL_H - GAP
    };
    Packing {
        tiles: out,
        content_height,
    }
}

fn free(occ: &[Vec<bool>], r: usize, c: usize, w: usize, h: usize) -> bool {
    for i in 0..h {
        for j in 0..w {
            if occ.get(r + i).and_then(|row| row.get(c + j)).copied() == Some(true) {
                return false;
            }
        }
    }
    true
}

fn mark(occ: &mut [Vec<bool>], r: usize, c: usize, w: usize, h: usize) {
    for i in 0..h {
        for j in 0..w {
            occ[r + i][c + j] = true;
        }
    }
}

/// Cols that fit in `avail_width` (§2.3: `floor((avail+GAP)/CELL_W)`).
pub fn cols_for_width(avail_width: f32) -> usize {
    (((avail_width + GAP) / CELL_W).floor() as usize).max(1)
}

impl Placed {
    /// Pixel top of the tile's **cell** (header-lane top), pre-inset.
    pub fn cell_top(&self) -> f32 {
        self.gy as f32 * CELL_H
    }

    /// Pixel bottom of the tile's occupied cells (for culling y-intersection).
    pub fn cell_bottom(&self) -> f32 {
        (self.gy + self.item.fr) as f32 * CELL_H - GAP
    }

    /// Left of the tile's cell block.
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

    #[test]
    fn hole_backfill() {
        // A 2×2 group then two 1×1 cards in 3 cols: the cards backfill the holes
        // beside the group (col 2, rows 0 and 1) — ordinal order kept.
        let items = [Item::group(4), Item::card(), Item::card()];
        let p = pack(&items, 3);
        assert_eq!((p.tiles[0].gx, p.tiles[0].gy), (0, 0)); // group top-left
        assert_eq!((p.tiles[1].gx, p.tiles[1].gy), (2, 0)); // card fills hole
        assert_eq!((p.tiles[2].gx, p.tiles[2].gy), (2, 1)); // card fills hole
        assert_eq!(p.content_height, 2.0 * CELL_H - GAP);
    }

    #[test]
    fn single_col_stacks() {
        let items = [Item::card(), Item::card(), Item::card()];
        let p = pack(&items, 1);
        assert_eq!((p.tiles[0].gy, p.tiles[1].gy, p.tiles[2].gy), (0, 1, 2));
    }

    #[test]
    fn cols_for_width_anchors() {
        assert_eq!(cols_for_width(0.0), 1); // clamped to ≥1
        assert_eq!(cols_for_width(940.0), 3); // SSOT window
        assert_eq!(cols_for_width(CELL_W - GAP), 1); // exactly one card wide
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
        assert_eq!((p.tiles[0].gx, p.tiles[0].gy), (0, 0));
        assert_eq!(p.content_height, CELL_H - GAP);
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
        assert_eq!((t.gx, t.gy), (0, 0));
        // one cell tall: CELL_H minus the trailing gap.
        assert_eq!(packing.content_height, CELL_H - GAP);
    }

    #[test]
    fn intersects_band_culls_outside() {
        // Row-0 loose card occupies cells [0, CELL_H-GAP]; a band far below misses.
        let p = pack(&[Item::card()], 3);
        let t = p.tiles[0];
        assert!(t.intersects_band(0.0, CELL_H)); // visible band covers row 0
        assert!(!t.intersects_band(CELL_H * 5.0, CELL_H * 6.0)); // far below → culled
    }
}
