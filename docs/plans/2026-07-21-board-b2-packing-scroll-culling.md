# Board B-2 — Packing, Scroll & Culling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the placeholder board grid with the real adaptive-packing masonry engine — pure packer in `lens-core`, an ordered `board_tree` walk, a custom scrollable/culling container in `lens-ui`, and a container-driven visibility gate that retires the paint-time gate and fixes the scroll/re-entry freeze at the root.

**Architecture:** The layout math (grid-snap first-fit `pack()`, `foot(n)`, cull band) is pure and lives in `lens-core::pack` — ported verbatim from the GO spike (`spikes/board-container/`). `BoardLayout::board_tree` gives an ordered, group-aware walk. `lens-ui::board::BoardView` builds a `BoardLayout` from `FleetStore` (an **explicitly provisional, B-4-replaced** adapter — see basis B below), walks it, packs tiles, and renders absolutely-positioned cards inside an `overflow_scroll` surface, building only tiles in the visible band. The container is the **sole visibility authority**: each frame it computes the visible session set and pushes `set_visible(bool)` to card views via `App::defer`, starting/stopping their anim timers — this retires the per-card paint-time `last_bounds` gate and the focus↔board edge-trigger recovery, fixing both the scroll-into-view and re-entry freezes.

**Tech Stack:** Rust, gpui 0.2.2 (`div`, `overflow_scroll`, `track_scroll`, `ScrollHandle`, `.cached()`, `cx.defer`), `#[gpui::test]` real-bounds harness (`VisualTestContext`).

## Global Constraints

- **Basis B (locked with the user, 2026-07-21):** B-2 does **not** wire `SqliteBoardStore` into `lens-ui`. The `BoardLayout` the packer walks is derived in-memory from `FleetStore` by a **provisional adapter** that B-4 deletes when it lands the persisted store→replica seam alongside the first board writes. The adapter must stay explicitly marked `B-4-REPLACED STUB`; the two guardrails that keep it from calcifying into "FleetStore is the placement authority" are (1) the read-APIs take `&BoardLayout` so consumers are blind to the source, and (2) the comment. Rationale: persistence earns its keep at the first write (B-4); reading the store now reads a tree whose entire content the adapter can regenerate.
- **`lens-core` is pure** — no gpui types in `pack.rs` or `domain/board.rs`. Layout constants are plain `f32`.
- **Geometry constants are the spike/mockup values** (tune on device later, spec §8): `CARD_W=280`, `CARD_H=160`, `HEADER=24`, `GAP=16`, `INSET=5`; `CELL_W=296`, `CELL_H=200`. Overdraw margin = `1 × CELL_H` (200px), resolved in spec §8.
- **`.cached()` dirty-tracking landmine** ([[viewport-reentry-freeze]]): never read a sibling card entity inside `BoardView::render`'s accessed-entity window. All `set_visible` writes go through `cx.defer` (off the render path).
- **Init subtlety** (spike §3): card views MUST init `visible=false`. If they init visible, the container's first `set_visible(true)` early-returns and the timer never spawns.
- **Cards keyed by `SessionId`** throughout `lens-ui`; the ephemeral `ConnectionId` is a placeholder (`conn_ephemeral`) — real conn scoping is B-5.
- **Depth-1 groups committed/tested; deeper recursive-by-construction, PROVISIONAL** (spec §1). At runtime under basis B there are **zero** groups (none creatable until B-4); the group render arm exists so B-4 doesn't panic and B-3 has an arm to fill with chrome.
- **Gate scope** ([[xtask-gate-scope]]): `lens-core` and `lens-ui` are already in the `xtask gate` `-p` lists — no gate-list edit needed. Never pipe the gate through `tail`.
- **Commits** end with the trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **Review diversity** (CLAUDE.md): every task gets ≥1 cross-family review before it's considered done.

---

## File Structure

- **Create `crates/lens-core/src/pack.rs`** — pure packer: `foot`, `pack`, `cols_for_width`, `Item`/`Kind`/`Placed`/`Packing`, geometry consts, `Placed::{cell_top,cell_bottom,cell_left,intersects_band}`. Ported verbatim from `spikes/board-container/src/packer.rs` + one cull helper. (Task 1)
- **Modify `crates/lens-core/src/lib.rs`** — add `pub mod pack;`. (Task 1)
- **Modify `crates/lens-core/src/domain/board.rs`** — add `BoardNode` enum + `board_tree` + `nodes_under` + `BoardNode::leaf_sessions`. (Task 2)
- **Create `crates/lens-ui/src/board/layout_adapter.rs`** — `build_ephemeral_layout(&FleetStore) -> BoardLayout` (the B-4-REPLACED stub). (Task 3)
- **Modify `crates/lens-ui/src/board/mod.rs`** — scroll handles + a single `pack_and_render` container helper (masonry + scroll + cull), used by both board and focused-rail modes; the unified visibility gate; retire `recover_viewport_gates_on_reentry` + `last_mode`. (Tasks 4, 5)
- **Modify `crates/lens-ui/src/card/view.rs`** — add `visible` field (init false) + `set_visible`; replace the `last_bounds`-derived `visible` with the field; remove `invalidate_viewport_gate`. (Task 5)
- **Modify `crates/lens-ui/tests/acceptance_shell.rs`** — rewrite the two freeze tests to the container-gating contract. (Task 5)

---

## Task 1: Port the pure packer into `lens-core`

**Files:**
- Create: `crates/lens-core/src/pack.rs`
- Modify: `crates/lens-core/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `pack.rs`

**Interfaces:**
- Consumes: nothing (leaf module).
- Produces:
  - consts `CARD_W CARD_H HEADER GAP INSET CELL_W CELL_H: f32`
  - `enum Kind { Card, Group { members: usize } }`
  - `struct Item { kind: Kind, fc: usize, fr: usize }` with `Item::card()`, `Item::group(members: usize)`
  - `fn foot(n: usize) -> (usize, usize)`
  - `struct Placed { item: Item, item_index: usize, gx: usize, gy: usize }` with `cell_top(&self)->f32`, `cell_bottom(&self)->f32`, `cell_left(&self)->f32`, `intersects_band(&self, lo: f32, hi: f32)->bool`
  - `struct Packing { tiles: Vec<Placed>, content_height: f32 }`
  - `fn pack(items: &[Item], cols: usize) -> Packing`
  - `fn cols_for_width(avail_width: f32) -> usize`

- [ ] **Step 1: Write `pack.rs` with the ported algorithm + tests**

Port `spikes/board-container/src/packer.rs` verbatim (it is already the reference), swapping the header doc-comment to point at this plan, and add the `intersects_band` cull helper + its test. Full file:

```rust
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
        Item { kind: Kind::Card, fc: 1, fr: 1 }
    }

    pub fn group(members: usize) -> Self {
        let (fc, fr) = foot(members);
        Item { kind: Kind::Group { members }, fc, fr }
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
        let (fc, fr) = (it.fc.min(cols), it.fr);
        let mut placed = false;
        let mut r = 0usize;
        while !placed {
            ensure(&mut occ, r + fr - 1);
            for c in 0..=(cols - fc) {
                if free(&occ, r, c, fc, fr) {
                    mark(&mut occ, r, c, fc, fr);
                    out.push(Placed { item: *it, item_index, gx: c, gy: r });
                    max_r = max_r.max(r + fr);
                    placed = true;
                    break;
                }
            }
            r += 1;
        }
    }

    let content_height = if max_r == 0 { 0.0 } else { max_r as f32 * CELL_H - GAP };
    Packing { tiles: out, content_height }
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
    fn intersects_band_culls_outside() {
        // Row-0 loose card occupies cells [0, CELL_H-GAP]; a band above it misses.
        let p = pack(&[Item::card()], 3);
        let t = p.tiles[0];
        assert!(t.intersects_band(0.0, CELL_H)); // visible band covers row 0
        assert!(!t.intersects_band(CELL_H * 5.0, CELL_H * 6.0)); // far below → culled
    }
}
```

- [ ] **Step 2: Wire the module**

In `crates/lens-core/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod pack;
```

- [ ] **Step 3: Run the tests — expect PASS**

Run: `cargo test -p lens-core pack::`
Expected: `foot_anchors`, `hole_backfill`, `single_col_stacks`, `cols_for_width_anchors`, `intersects_band_culls_outside` all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core/src/pack.rs crates/lens-core/src/lib.rs
git commit -m "feat(board): B-2 pure packer (foot/pack/cull) → lens-core"
```

---

## Task 2: `board_tree` ordered-walk read-API on `BoardLayout`

**Files:**
- Modify: `crates/lens-core/src/domain/board.rs`
- Test: inline `#[cfg(test)] mod tests` in `domain/board.rs`

**Interfaces:**
- Consumes: `BoardLayout::children(board_id, parent)` (exists), `BoardItem`, `BoardItemKind`, `BoardError`, `SessionId`.
- Produces:
  - `enum BoardNode<'a> { Card(&'a BoardItem), Group { item: &'a BoardItem, members: Vec<BoardNode<'a>> } }`
  - `BoardNode::leaf_sessions(&self) -> Vec<&'a SessionId>`
  - `BoardLayout::board_tree(&self, board_id: &BoardId) -> Result<Vec<BoardNode<'_>>, BoardError>`

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` in `crates/lens-core/src/domain/board.rs`. These reuse the existing test helpers already in that module (`layout_with_default_board`, `card_id`, `sess`, `conn`) — confirm their names by reading the module; if they differ, adapt the calls (do not redefine them).

```rust
    fn group_item(id: &str, ordinal: i32, archived: bool) -> BoardItem {
        BoardItem {
            id: BoardItemId::new(id),
            board_id: BoardId::new(DEFAULT_BOARD_ID),
            parent_item_id: None,
            ordinal,
            kind: BoardItemKind::Group {
                name: id.into(),
                color_token: None,
                collapsed: false,
                archived,
            },
            created_at: 1_700_000_000_000,
        }
    }

    fn card_item(id: &str, session: &str, parent: Option<&str>, ordinal: i32) -> BoardItem {
        BoardItem {
            id: BoardItemId::new(id),
            board_id: BoardId::new(DEFAULT_BOARD_ID),
            parent_item_id: parent.map(BoardItemId::new),
            ordinal,
            kind: BoardItemKind::Card { conn: conn(), session: sess(session) },
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn board_tree_orders_loose_cards_by_ordinal() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            card_item("c2", "s2", None, 2),
            card_item("c1", "s1", None, 1),
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(sessions, vec![&sess("s1"), &sess("s2")]);
    }

    #[test]
    fn board_tree_nests_group_members() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            group_item("g1", 1, false),
            card_item("c1", "s1", Some("g1"), 1),
            card_item("c2", "s2", Some("g1"), 2),
            card_item("c3", "s3", None, 2), // loose after the group
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        assert_eq!(nodes.len(), 2); // group node + loose card
        match &nodes[0] {
            BoardNode::Group { members, .. } => {
                assert_eq!(members.len(), 2);
                assert_eq!(nodes[0].leaf_sessions(), vec![&sess("s1"), &sess("s2")]);
            }
            _ => panic!("first node must be the group"),
        }
        assert!(matches!(nodes[1], BoardNode::Card(_)));
    }

    #[test]
    fn board_tree_skips_archived_group() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            group_item("g_arch", 1, true),
            card_item("c1", "s1", Some("g_arch"), 1), // under archived group → skipped
            card_item("c2", "s2", None, 2),
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(sessions, vec![&sess("s2")]); // archived subtree absent
    }

    #[test]
    fn board_tree_unknown_board_errs() {
        let layout = layout_with_default_board();
        let err = layout.board_tree(&BoardId::new("nope")).unwrap_err();
        assert_eq!(err, BoardError::BoardNotFound);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core board_tree`
Expected: FAIL — `no method named board_tree` / `cannot find type BoardNode`.

- [ ] **Step 3: Implement `BoardNode` + `board_tree`**

Add to `crates/lens-core/src/domain/board.rs`. Place the `BoardNode` enum near the other public types (after `BoardItemKind`), and the methods inside `impl BoardLayout` (next to `children`):

```rust
/// A node in the ordered board walk (`board_tree`). Recursive so nested groups
/// work by construction — depth-1 is committed/tested; deeper is PROVISIONAL
/// until B-5 makes nested groups reachable (spec §1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardNode<'a> {
    /// A loose card item (`kind == Card`).
    Card(&'a BoardItem),
    /// A group and its ordered child nodes.
    Group {
        item: &'a BoardItem,
        members: Vec<BoardNode<'a>>,
    },
}

impl<'a> BoardNode<'a> {
    /// All leaf card sessions under this node, in walk order (a loose card → 1;
    /// a group → its members flattened). Powers the packer member count and the
    /// per-tile session list the renderer looks card views up by.
    pub fn leaf_sessions(&self) -> Vec<&'a SessionId> {
        match self {
            BoardNode::Card(item) => match &item.kind {
                BoardItemKind::Card { session, .. } => vec![session],
                BoardItemKind::Group { .. } => vec![],
            },
            BoardNode::Group { members, .. } => {
                members.iter().flat_map(|m| m.leaf_sessions()).collect()
            }
        }
    }
}
```

Inside `impl BoardLayout` (add after `children`):

```rust
    /// Ordered walk of a board's item forest — the packer input (§4). Top-level
    /// items in ordinal order; each group recurses into its children. **Archived
    /// groups (and their subtrees) are skipped** — they belong to the Archive
    /// surface (B-6). Depth-1 committed; deeper recursive-by-construction (§1).
    pub fn board_tree(&self, board_id: &BoardId) -> Result<Vec<BoardNode<'_>>, BoardError> {
        if !self.boards.iter().any(|b| &b.id == board_id) {
            return Err(BoardError::BoardNotFound);
        }
        Ok(self.nodes_under(board_id, None))
    }

    fn nodes_under(
        &self,
        board_id: &BoardId,
        parent: Option<&BoardItemId>,
    ) -> Vec<BoardNode<'_>> {
        self.children(board_id, parent)
            .into_iter()
            .filter_map(|item| match &item.kind {
                BoardItemKind::Card { .. } => Some(BoardNode::Card(item)),
                BoardItemKind::Group { archived: true, .. } => None, // → Archive (B-6)
                BoardItemKind::Group { .. } => Some(BoardNode::Group {
                    item,
                    members: self.nodes_under(board_id, Some(&item.id)),
                }),
            })
            .collect()
    }
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test -p lens-core board_tree`
Expected: all four `board_tree_*` tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/domain/board.rs
git commit -m "feat(board): B-2 board_tree ordered group-aware walk on BoardLayout"
```

---

## Task 3: Ephemeral `BoardLayout` adapter (basis-B stub)

**Files:**
- Create: `crates/lens-ui/src/board/layout_adapter.rs`
- Modify: `crates/lens-ui/src/board/mod.rs` (add `mod layout_adapter;`)
- Test: inline `#[cfg(test)] mod tests` in `layout_adapter.rs`

**Interfaces:**
- Consumes: `FleetStore` (`.cards: HashMap<SessionId, Entity<SessionCard>>`), `lens_core::domain::board::{BoardLayout, Board, BoardItem, BoardItemKind, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME}`, `lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId}`.
- Produces: `pub fn build_ephemeral_layout(fleet: &FleetStore) -> BoardLayout` — a group-less, deterministically-ordered loose-card layout.

- [ ] **Step 1: Write the failing test**

Create `crates/lens-ui/src/board/layout_adapter.rs`:

```rust
//! B-4-REPLACED STUB — provisional board tree derived from live `FleetStore`.
//!
//! Basis B (plan `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md`):
//! B-2 does NOT wire the persisted `SqliteBoardStore` into `lens-ui`. Until B-4
//! lands the store→replica seam alongside the first board writes, the packer
//! walks a `BoardLayout` fabricated here from the fleet's live cards — all loose,
//! deterministic order. This temporarily makes placement FleetStore-derived; the
//! two guardrails against that calcifying (per the plan's Global Constraints) are
//! that `board_tree` takes `&BoardLayout` (consumers are blind to the source) and
//! THIS comment. B-4 deletes this file and swaps in the real replica.

use crate::fleet::store::FleetStore;
use lens_core::domain::board::{
    Board, BoardItem, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME,
};
use lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};

/// Build a loose-card `BoardLayout` from the fleet's current cards, ordered
/// deterministically by session-id string (matches the retired placeholder's
/// order). No groups — none are creatable until B-4. `created_at`/ordinals are
/// synthetic (nothing is persisted).
pub fn build_ephemeral_layout(fleet: &FleetStore) -> BoardLayout {
    const EPOCH: i64 = 0;
    let board_id = BoardId::new(DEFAULT_BOARD_ID);
    let conn = ConnectionId::new("conn_ephemeral");

    let mut sessions: Vec<SessionId> = fleet.cards.keys().cloned().collect();
    sessions.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let items = sessions
        .into_iter()
        .enumerate()
        .map(|(i, session)| BoardItem {
            id: BoardItemId::new(format!("eph_{}", session.as_str())),
            board_id: board_id.clone(),
            parent_item_id: None,
            ordinal: i as i32,
            kind: BoardItemKind::Card { conn: conn.clone(), session },
            created_at: EPOCH,
        })
        .collect();

    BoardLayout {
        boards: vec![Board {
            id: board_id,
            name: DEFAULT_BOARD_NAME.into(),
            ordinal: 0,
            created_at: EPOCH,
            updated_at: EPOCH,
        }],
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use crate::clock::UiClock;
    use lens_core::domain::board::BoardNode;
    use std::sync::Arc;

    #[gpui::test]
    fn ephemeral_layout_is_ordered_loose_cards(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let (layout, board) = cx.update(|cx| {
            let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
            fleet.update(cx, |f, cx| {
                // Insert out of lexical order to prove the sort.
                f.spawn_fake_session(SessionId::new("s2"), cx);
                f.spawn_fake_session(SessionId::new("s1"), cx);
                f.spawn_fake_session(SessionId::new("s3"), cx);
            });
            let layout = build_ephemeral_layout(fleet.read(cx));
            let board = layout.default_board_id().unwrap().clone();
            (layout, board)
        });

        let nodes = layout.board_tree(&board).unwrap();
        assert_eq!(nodes.len(), 3);
        assert!(nodes.iter().all(|n| matches!(n, BoardNode::Card(_))));
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(
            sessions,
            vec![&SessionId::new("s1"), &SessionId::new("s2"), &SessionId::new("s3")]
        );
    }
}
```

Note: confirm `FleetStore::spawn_fake_session(SessionId, &mut Context)` is the real signature (the acceptance test uses `f.spawn_fake_session(id.clone(), cx)`); if it differs, adapt the test's session-seeding calls.

- [ ] **Step 2: Register the module**

In `crates/lens-ui/src/board/mod.rs`, add near the top (below the existing `use` block):

```rust
mod layout_adapter;
```

- [ ] **Step 3: Run the test to verify it fails, then passes**

Run: `cargo test -p lens-ui ephemeral_layout_is_ordered_loose_cards`
Expected: PASS once the module compiles (the code above is the implementation). If it fails to compile on `spawn_fake_session` arity, fix the call per the note in Step 1.

- [ ] **Step 4: Commit**

```bash
git add crates/lens-ui/src/board/layout_adapter.rs crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): B-2 ephemeral FleetStore→BoardLayout adapter (B-4-replaced stub)"
```

---

## Task 4: Scroll container + culling render

Replaces the placeholder `render_board_grid`/`render_shrunk_boards` flex layouts with the absolute-masonry scroll container. Board mode = container at N cols; focused rail = the **same** container at 1 col (spec §5). This task delivers the visual masonry + scroll + culling; the **timer/visibility gate stays the old paint-time gate for now** (Task 5 swaps it). Off-screen timers still run after this task — that is Task 5's fix.

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs`
- Test: `crates/lens-ui/tests/acceptance_shell.rs` (add one culling test) + existing `shell_skeleton_acceptance` must still pass.

**Interfaces:**
- Consumes: `lens_core::pack::{self, Item, Kind, Placed, CELL_W, CELL_H, CARD_W, CARD_H, HEADER, GAP, INSET, cols_for_width, pack}`, `board::layout_adapter::build_ephemeral_layout`, `lens_core::domain::board::BoardNode`, `gpui::ScrollHandle`.
- Produces (on `BoardView`):
  - fields `board_scroll: ScrollHandle`, `rail_scroll: ScrollHandle`
  - `fn pack_and_render(&mut self, avail_width: f32, viewport_h: f32, scroll: ScrollHandle, cx: &mut Context<Self>) -> (gpui::AnyElement, Vec<SessionId>)` — returns the scroll element and the visible-band session ids (consumed by Task 5's gate; in Task 4 the second value is stored for the culling test via `last_built`).
  - field `last_built: Vec<SessionId>` + `pub fn visible_session_ids_for_test(&self) -> Vec<SessionId>`
  - const `RAIL_W: f32 = 286.0`, `NAV_RAIL_W: f32 = 48.0`

- [ ] **Step 1: Add the fields + imports**

In `crates/lens-ui/src/board/mod.rs`, extend the imports and the `BoardView` struct. Add to the `gpui` import list: `AnyElement, ScrollHandle`. Add:

```rust
use lens_core::domain::board::BoardNode;
use lens_core::pack::{self, CARD_H, CARD_W, CELL_H, CELL_W, GAP, HEADER, INSET, Item};
use std::collections::HashSet;
```

Constants near the top of the module:

```rust
/// Width of the left nav rail (unchanged placeholder).
const NAV_RAIL_W: f32 = 48.0;
/// Width of the focused-mode session rail (spec §5; `.boards` strip = 286px).
const RAIL_W: f32 = 286.0;
```

Struct fields (add to `BoardView`):

```rust
    /// Scroll position of the board masonry surface (spec §4 unknown 1).
    board_scroll: ScrollHandle,
    /// Scroll position of the focused-mode rail (same container at 1 col, §5).
    rail_scroll: ScrollHandle,
    /// Session ids whose tiles were in the visible band at the last render —
    /// the cull result (test hook + Task 5's gate input).
    last_built: Vec<SessionId>,
```

Initialise them in **both** constructors — `mount` (the returned `Self { .. }`) — with:

```rust
            board_scroll: ScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            last_built: Vec::new(),
```

- [ ] **Step 2: Write `pack_and_render`**

Add to `impl BoardView`. This mirrors `spikes/board-container/src/container.rs::render` (the GO spike) — build the ephemeral layout, walk it, pack, cull, render absolute tiles into an `overflow_scroll` surface:

```rust
    /// The masonry scroll container (spec §4). Builds the ephemeral tree, packs
    /// it into `cols_for_width(avail_width)` columns, and renders only tiles whose
    /// y-range intersects the visible band (+ `1×CELL_H` overdraw). Returns the
    /// element and the visible-band session ids (Task 5's gate consumes them).
    fn pack_and_render(
        &mut self,
        avail_width: f32,
        viewport_h: f32,
        scroll: ScrollHandle,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Vec<SessionId>) {
        let layout = build_ephemeral_layout(self.fleet.read(cx));
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
        let nodes = layout.board_tree(&board_id).unwrap_or_default();

        // nodes → parallel (pack items, per-tile session ids)
        let mut items: Vec<Item> = Vec::with_capacity(nodes.len());
        let mut tile_sessions: Vec<Vec<SessionId>> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            let sessions: Vec<SessionId> = node.leaf_sessions().into_iter().cloned().collect();
            items.push(match node {
                BoardNode::Card(_) => Item::card(),
                BoardNode::Group { .. } => Item::group(sessions.len()),
            });
            tile_sessions.push(sessions);
        }

        let cols = pack::cols_for_width(avail_width);
        let packing = pack::pack(&items, cols);

        // Last frame's painted offset (one-frame lag → overdraw covers it, §8).
        let scroll_top = (-f32::from(scroll.offset().y)).max(0.0);
        let overdraw = CELL_H;
        let lo = scroll_top - overdraw;
        let hi = scroll_top + viewport_h + overdraw;

        let mut content = div()
            .relative()
            .w(px(cols as f32 * CELL_W))
            .h(px(packing.content_height));

        let mut visible: Vec<SessionId> = Vec::new();
        for placed in &packing.tiles {
            if !placed.intersects_band(lo, hi) {
                continue; // culled → absent from child vec → gpui never builds it
            }
            let sessions = &tile_sessions[placed.item_index];
            for s in sessions {
                visible.push(s.clone());
            }
            match placed.item.kind {
                pack::Kind::Card => {
                    if let Some(tile) = self.absolute_card(&sessions[0], placed.cell_left(), placed.cell_top() + HEADER, cx) {
                        content = content.child(tile);
                    }
                }
                pack::Kind::Group { .. } => {
                    content = content.child(self.absolute_group(placed, sessions, cx));
                }
            }
        }

        self.last_built = visible.clone();

        let el = div()
            .id("board-scroll")
            .size_full()
            .overflow_scroll()
            .track_scroll(&scroll)
            .child(content)
            .into_any_element();
        (el, visible)
    }

    /// One loose card absolutely positioned at its body-zone (`top` already offset
    /// by HEADER by the caller). Clickable (focus the session).
    fn absolute_card(
        &self,
        session_id: &SessionId,
        left: f32,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let cached = self.cached_tiles.get(session_id)?.clone();
        let entity_id = self.card_views.get(session_id)?.entity_id();
        let sid = session_id.clone();
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(CARD_W))
                .h(px(CARD_H))
                .id(("session-card-click", entity_id))
                .on_click(cx.listener(move |board, event, window, cx| {
                    board.card_click(sid.clone(), event, window, cx);
                }))
                .child(cached)
                .into_any_element(),
        )
    }

    /// A group tile: a **bare neutral placeholder box** in the inter-tile gap plus
    /// its member cards at full size in body-zones. Chrome (ring color / header /
    /// rollups) is B-3; this arm proves the geometry and gives B-3 something to
    /// fill. Under basis B no group is reachable at runtime — exercised in B-4.
    fn absolute_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (fc, fr) = (placed.item.fc, placed.item.fr);
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = fc as f32 * CELL_W - GAP;
        let block_h = fr as f32 * CELL_H - GAP;

        let mut ring = div()
            .absolute()
            .left(px(x - INSET))
            .top(px(y - INSET))
            .w(px(block_w + 2.0 * INSET))
            .h(px(block_h + 2.0 * INSET))
            .rounded(px(12.0))
            .border_1()
            .border_color(gpui::rgb(0x3a3a42)); // neutral; B-3 recolors per group_token

        for (i, session) in sessions.iter().enumerate() {
            let cc = i % fc;
            let rr = i / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * CELL_H;
            if let Some(tile) = self.absolute_card(session, x - INSET + mx, y - INSET + my, cx) {
                ring = ring.child(tile);
            }
        }
        ring.into_any_element()
    }

    /// Test hook: the session ids whose tiles were built (in the visible band) at
    /// the last render — proves culling.
    pub fn visible_session_ids_for_test(&self) -> Vec<SessionId> {
        self.last_built.clone()
    }
```

Note on the group member offset: `absolute_card` positions relative to the scroll content (not the ring div), so the member left/top are the ring's absolute origin (`x - INSET`, `y - INSET`) **plus** the in-ring member offset (`mx`, `my`). That is what the code above passes. (The spike nested members *inside* the ring div; here members are siblings in the same absolute content coordinate space, which keeps a single click/positioning path via `absolute_card`.)

- [ ] **Step 3: Rewire `render` to call the container in both modes**

Replace the `render` body's `match mode` so both arms use `pack_and_render`. Compute `avail_width`/`viewport_h` from the window. Replace `render_board_grid`/`render_shrunk_boards` call sites:

```rust
impl Render for BoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_card_views(cx);
        let mode = ShellMode::from_fleet(self.fleet.read(cx));
        let viewport = window.viewport_size();
        let viewport_h = f32::from(viewport.height);
        let viewport_w = f32::from(viewport.width);

        let body = match &mode {
            ShellMode::Board => {
                let avail = (viewport_w - NAV_RAIL_W).max(CELL_W);
                let (surface, _visible) =
                    self.pack_and_render(avail, viewport_h, self.board_scroll.clone(), cx);
                div()
                    .id("shell-board")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().flex_grow().h_full().child(surface))
            }
            ShellMode::Focused { .. } => {
                let (rail, _visible) =
                    self.pack_and_render(RAIL_W, viewport_h, self.rail_scroll.clone(), cx);
                div()
                    .id("shell-focused")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().w(px(RAIL_W)).flex_shrink_0().h_full().child(rail))
                    .child(div().id("chat-slot").flex_grow().child("chat"))
                    .child(
                        div()
                            .id("navigator-slot")
                            .w(px(200.0))
                            .flex_shrink_0()
                            .child("navigator"),
                    )
                    .child(
                        div()
                            .id("working-area-slot")
                            .flex_grow()
                            .child(self.working_tab.view.clone()),
                    )
            }
        };
        div().id("board-view").size_full().child(body)
    }
}
```

Then **delete** the now-unused `render_board_grid`, `render_shrunk_boards`, and `render_card_tile` methods (they are replaced by `pack_and_render`/`absolute_card`). Leave `render_nav_rail` as-is.

Note: `last_mode` is still written elsewhere in Task-4 state — it becomes dead in Task 5. For Task 4, remove the `self.last_mode = Some(mode.clone())` line from the old render (it is gone in the replacement above) and leave the field + `recover_viewport_gates_on_reentry` in place until Task 5 (they still compile; the observe closure still calls recovery). If the compiler warns `last_mode` never read, silence is fine for one task — Task 5 removes it.

- [ ] **Step 4: Add a culling test**

Append to `crates/lens-ui/tests/acceptance_shell.rs`:

```rust
/// Culling (spec §4 unknown 2): on a board tall enough to overflow, tiles below
/// the visible band + overdraw are NOT built (absent from the child vec).
#[gpui::test]
async fn board_culls_offscreen_tiles(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 40; // 40 loose cards @ 200px cell / ~3 cols ⇒ ~14 rows ⇒ tall
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i:02}"))).collect();

    let fleet = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        fleet.update(cx, |f, cx| {
            for id in &ids {
                f.spawn_fake_session(id.clone(), cx);
            }
        });
        fleet
    });

    let fleet_for_window = fleet.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, None, cx)
    });
    // Normal-size window: only the top few rows fit; the rest overflow the band.
    vcx.simulate_resize(Size { width: px(1000.0), height: px(700.0) });
    vcx.run_until_parked();

    let built = vcx.read(|cx| board_handle.read(cx).visible_session_ids_for_test());
    assert!(!built.is_empty(), "some tiles must be built");
    assert!(built.len() < N, "off-screen tiles must be culled (built {} of {N})", built.len());
    // The last card (bottom row) is far below the band → not built.
    assert!(
        !built.contains(&ids[N - 1]),
        "bottom card must be culled while scrolled to top"
    );
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p lens-ui board_culls_offscreen_tiles shell_skeleton_acceptance`
Expected: `board_culls_offscreen_tiles` PASS; `shell_skeleton_acceptance` still PASS (it asserts slot structure — verify it does not depend on the removed `render_board_grid` internals; if it queried a `#board-grid` id, update it to `#board-scroll`).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/tests/acceptance_shell.rs
git commit -m "feat(board): B-2 absolute-masonry scroll container + culling (both modes)"
```

---

## Task 5: Container-driven visibility gate; retire the old gate

The container becomes the **sole visibility authority**. Card views init hidden; each render the container computes the visible session set and applies `set_visible(bool)` via `App::defer`, starting/stopping anim timers. This retires the paint-time `last_bounds` gate (`card/view.rs`) and the `recover_viewport_gates_on_reentry` edge-trigger (`board/mod.rs`), fixing the scroll-into-view **and** focus↔board re-entry freezes at the root (spec §4 unknown 3).

**Files:**
- Modify: `crates/lens-ui/src/card/view.rs`
- Modify: `crates/lens-ui/src/board/mod.rs`
- Modify: `crates/lens-ui/tests/acceptance_shell.rs`

**Interfaces:**
- Consumes: `pack_and_render`'s returned visible-session `Vec<SessionId>` (Task 4).
- Produces:
  - on `SessionCardView`: field `visible: bool` (init `false`); `pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>)`; `pub fn is_visible(&self) -> bool`; `pub fn timer_running_for_test(&self) -> bool`. Removes `pub fn invalidate_viewport_gate`.
  - on `BoardView`: field `gated_visible: HashSet<SessionId>`; `fn apply_visibility_gate(&mut self, want: HashSet<SessionId>, cx: &mut Context<Self>)`. Removes `recover_viewport_gates_on_reentry` + `last_mode`.

- [ ] **Step 1: Swap the card gate to a container-driven field**

In `crates/lens-ui/src/card/view.rs`:

Add the field to `SessionCardView` (near `anim_task`):

```rust
    /// Container-driven visibility gate (replaces the paint-time `last_bounds`
    /// gate). Init HIDDEN — the board container is the sole visibility authority
    /// and flips truly-visible cards on via `set_visible` (spec §4 unknown 3). If
    /// this init'd `true`, the first `set_visible(true)` would early-return and the
    /// anim timer would never spawn.
    visible: bool,
```

Init it `false` in `SessionCardView::new`'s returned `Self { .. }`:

```rust
            visible: false,
```

Add the methods to `impl SessionCardView` (replace `invalidate_viewport_gate`):

```rust
    /// Container calls this (via `cx.defer`, off its own render path) when a tile
    /// enters/leaves the visible band. Starts/stops the anim driver at the root —
    /// a card scrolled/returned into view respawns its timer here, which is exactly
    /// what the retired edge-triggered gate could not do (the freeze).
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        cx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Test hook: is the anim driver live?
    pub fn timer_running_for_test(&self) -> bool {
        self.anim_task.is_some()
    }
```

In `render`, replace the `last_bounds`-derived `visible` computation (the block currently at lines ~95-104: `let visible = match self.last_bounds.get() { ... }`) with the field:

```rust
        // Visibility is container-driven (spec §4 unknown 3), not paint-time.
        let visible = self.visible;
```

Leave the rest of the anim-driver logic (`let desired = anim_tick_for(wave).filter(|_| visible); if desired != self.anim_interval { ... }`) unchanged — it already spawns/drops the timer off `visible`. Leave the paint-time `canvas`/`last_bounds`/`paint_count` at the bottom of `render` intact (still used by `card_bounds_for_test` + demo instrumentation; it just no longer gates anything).

- [ ] **Step 2: Add the unified gate to `BoardView` + retire the old recovery**

In `crates/lens-ui/src/board/mod.rs`:

Add the field to `BoardView`:

```rust
    /// Session ids currently gated visible (their anim timers allowed to run).
    /// The container is the sole authority; diffed each render, applied via defer.
    gated_visible: HashSet<SessionId>,
```

Init it in `mount`:

```rust
            gated_visible: HashSet::new(),
```

**Remove** the `last_mode` field, its init (`last_mode: None`), the `recover_viewport_gates_on_reentry` method, and its call in the `cx.observe` closure. The observe closure becomes:

```rust
        cx.observe(&fleet_for_observe, |board: &mut BoardView, _, cx| {
            board.sync_card_views(cx);
            cx.notify();
        })
        .detach();
```

Add the gate method to `impl BoardView`:

```rust
    /// Apply the container-computed visible set to the card views — the diff since
    /// last frame, pushed via `App::defer` so no sibling card entity is read inside
    /// `render`'s accessed-entity window (the `.cached()` dirty-tracking landmine,
    /// [[viewport-reentry-freeze]]). Newly-visible cards spawn their timers; newly-
    /// hidden cards drop them. Cards absent from any surface stay hidden.
    fn apply_visibility_gate(&mut self, want: HashSet<SessionId>, cx: &mut Context<Self>) {
        if want == self.gated_visible {
            return;
        }
        let newly_vis: Vec<SessionId> = want.difference(&self.gated_visible).cloned().collect();
        let newly_hid: Vec<SessionId> = self.gated_visible.difference(&want).cloned().collect();
        let views = self.card_views.clone(); // Entity clones are cheap (Rc)
        self.gated_visible = want;
        cx.defer(move |app: &mut App| {
            for id in newly_vis {
                if let Some(v) = views.get(&id) {
                    v.update(app, |c, cx| c.set_visible(true, cx));
                }
            }
            for id in newly_hid {
                if let Some(v) = views.get(&id) {
                    v.update(app, |c, cx| c.set_visible(false, cx));
                }
            }
        });
    }
```

In `render`, capture the visible set from whichever surface is active and apply the gate before returning. Update the two match arms to bind `visible` and, after the `match`, call the gate:

```rust
        let (body, visible): (_, Vec<SessionId>) = match &mode {
            ShellMode::Board => {
                let avail = (viewport_w - NAV_RAIL_W).max(CELL_W);
                let (surface, visible) =
                    self.pack_and_render(avail, viewport_h, self.board_scroll.clone(), cx);
                let el = div()
                    .id("shell-board")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().flex_grow().h_full().child(surface));
                (el.into_any_element(), visible)
            }
            ShellMode::Focused { .. } => {
                let (rail, visible) =
                    self.pack_and_render(RAIL_W, viewport_h, self.rail_scroll.clone(), cx);
                let el = div()
                    .id("shell-focused")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().w(px(RAIL_W)).flex_shrink_0().h_full().child(rail))
                    .child(div().id("chat-slot").flex_grow().child("chat"))
                    .child(
                        div().id("navigator-slot").w(px(200.0)).flex_shrink_0().child("navigator"),
                    )
                    .child(
                        div().id("working-area-slot").flex_grow().child(self.working_tab.view.clone()),
                    );
                (el.into_any_element(), visible)
            }
        };
        self.apply_visibility_gate(visible.into_iter().collect(), cx);
        div().id("board-view").size_full().child(body)
```

(`into_any_element()` requires the `gpui::AnyElement` import already added in Task 4; both arms now return `AnyElement` so the `match` types unify.)

- [ ] **Step 3: Rewrite the two freeze tests to the container-gating contract**

In `crates/lens-ui/tests/acceptance_shell.rs`, the two tests `card_offscreen_in_focus_rail_resumes_animating_on_return` and `card_offscreen_resumes_when_board_mounts_focused` currently assert via `card_bounds_for_test` + the old gate. Rewrite their precondition + assertions to the new mechanism: an off-screen (culled) card reports `is_visible() == false` and a frozen `render_count`; on return/scroll-in it reports `is_visible() == true` and a resuming `render_count`. Replace the body of the first test with:

```rust
#[gpui::test]
async fn card_offscreen_in_focus_rail_resumes_animating_on_return(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 12; // enough rail cards to overflow a short window at 1 col
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i:02}"))).collect();

    let fleet = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        fleet.update(cx, |f, cx| {
            for id in &ids {
                f.spawn_fake_session(id.clone(), cx);
            }
        });
        for id in &ids {
            let card = fleet.read(cx).card(id).unwrap();
            card.update(cx, |c, _| c.status = SessionStatusValue::Running); // Working → animates
        }
        fleet
    });

    let fleet_for_window = fleet.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, None, cx)
    });
    // Wide + short: board (multi-col) keeps all cards on-screen; the 1-col focus
    // rail overflows so the bottom cards cull.
    vcx.simulate_resize(Size { width: px(3000.0), height: px(700.0) });
    vcx.run_until_parked();

    let top = ids[0].clone(); // on-screen control
    let bottom = ids[N - 1].clone(); // off-screen in the rail

    let (rc_top, rc_bottom) = vcx.read(|cx| {
        let views = board_handle.read(cx).card_views_for_test();
        (views[&top].read(cx).render_count.clone(), views[&bottom].read(cx).render_count.clone())
    });

    // Sanity: on the wide board every Working card is visible and animating.
    let base_top = rc_top.get();
    let base_bottom = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_top.get() > base_top && rc_bottom.get() > base_bottom,
        "sanity: all Working cards animate on the board"
    );

    // Enter focus mode; the bottom rail card culls → hidden → timer drops.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.focus_session(ids[0].clone(), cx)));
    vcx.run_until_parked();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    let (bottom_hidden, bottom_timer) = vcx.read(|cx| {
        let v = &board_handle.read(cx).card_views_for_test()[&bottom];
        (v.read(cx).is_visible(), v.read(cx).timer_running_for_test())
    });
    assert!(!bottom_hidden, "off-screen rail card must be gated hidden");
    assert!(!bottom_timer, "hidden card's anim timer must be dropped");
    let settled = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert_eq!(rc_bottom.get(), settled, "culled card must not re-render (no off-screen CPU)");

    // Return to the board — bottom card re-enters the visible band → timer respawns.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    vcx.run_until_parked();
    let after_blur = rc_bottom.get();
    for _ in 0..6 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_bottom.get() > after_blur,
        "FREEZE BUG: card off-screen in the rail is frozen after return — timer never respawned"
    );
}
```

For the second test (`card_offscreen_resumes_when_board_mounts_focused`) — read its current body and apply the **same** substitution: it mounts already-focused, so assert the bottom rail card is `is_visible()==false`/timer-dropped while focused, then `blur_to_board`, then assert `render_count` resumes. The mount-already-focused shape no longer needs the `last_mode` first-frame trick (the container recomputes the visible set every render regardless of how it was reached), so the test simply drops the `last_mode`-specific commentary and asserts the resume.

- [ ] **Step 4: Run the full lens-ui test suite**

Run: `cargo test -p lens-ui`
Expected: all pass, including the rewritten freeze tests + `board_culls_offscreen_tiles` + `ephemeral_layout_is_ordered_loose_cards`. If `shell_skeleton_acceptance` referenced `invalidate_viewport_gate`/`card_bounds_for_test` in a way that no longer holds, update those references.

- [ ] **Step 5: Run the gate**

Run: `cargo run -p xtask -- gate` (do NOT pipe through `tail` — [[xtask-gate-scope]]).
Expected: fmt clean, clippy zero warnings (no dead-code warnings for removed `last_mode`/`invalidate_viewport_gate`/`recover_viewport_gates_on_reentry` — they must be fully removed, not left dangling), all tests green.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/card/view.rs crates/lens-ui/src/board/mod.rs crates/lens-ui/tests/acceptance_shell.rs
git commit -m "feat(board): B-2 container-driven visibility gate; retire paint-time gate + re-entry recovery"
```

---

## Task 6: On-device verification + off-screen CPU check

The spike already measured cull-ON ≈6.8% vs all-timers ≈15.3% CPU on a 56-tile fixture (spec §4 unknown 4). This task confirms the same win holds in the real `lens-ui` app and that scroll/re-entry no longer freezes on device. Not a unit test — a verification checkpoint.

**Files:** none (verification only).

- [ ] **Step 1: Build + run the app (release) with enough sessions to overflow**

Use the project's run path (the `run` skill or the demo feature). Launch with ≳20 fake sessions so the board overflows and most tiles are off-screen. Scroll the board; confirm:
- tiles above/below the band are not painted (no visual artifacts on fast scroll; overdraw = 1×CELL_H covers pop-in);
- off-screen cards' spinners freeze, on-screen resume — no permanent freeze after scroll-in or after focus→board round-trips.

- [ ] **Step 2: Measure idle CPU, cull vs all-timers**

Mirror `spikes/board-container/measure.sh` against the real app if a toggle exists; otherwise sample idle CPU (Activity Monitor / `top -pid`) with the board overflowing and confirm it sits well below a no-cull baseline. Record the numbers in the handoff.

- [ ] **Step 3: Record results + push decision**

Update `docs/STATUS.md` (B-2 shipped) and write a short handoff `docs/handoffs/2026-07-21-board-b2-executed.md` with the measured CPU + the on-device freeze verification. Push is a separate call ([[commit-when-finished]]) — surface it, don't auto-push.

---

## Self-Review (against spec `2026-07-20-board-packing-and-group-rendering-design.md`)

- **§2.1 cell grid / §2.2 foot / §2.3 pack / §2.4 tile placement** → Task 1 (packer port, tested against every §2.2 anchor) + Task 4 (tile placement geometry: loose card at `Y+HEADER`, group ring in the gap, members at body-zones).
- **§4 unknown 1 scroll surface** → Task 4 (`overflow_scroll` + `track_scroll` + explicit content-height child + `offset().y`).
- **§4 unknown 2 culling** → Task 1 (`intersects_band`) + Task 4 (cull loop + `board_culls_offscreen_tiles`).
- **§4 unknown 3 timer gate + retire old gate + freeze fix** → Task 5 (container-driven `set_visible` via defer; removes `last_bounds` gate + `recover_viewport_gates_on_reentry`; rewritten freeze tests).
- **§4 unknown 4 CPU** → Task 6.
- **§5 focused rail = same logic at 1 col** → Task 4 (rail = `pack_and_render(RAIL_W)` → cols=1). **Deferred within B-2:** the §5 *compact-card variant* (~244px) is visual polish — B-2 runs the 286px rail with the full 280 card and wires its visibility/culling; note this in the handoff.
- **§6 seams** → `board_tree` added (Task 2). `group_of` is **B-3's** (rollups) — not built here (YAGNI; no B-2 consumer). Slot hit-testing / write path → B-4.
- **§8 overdraw = 1×CELL_H** → Task 4 (`overdraw = CELL_H`). Tunables left at mockup values (Global Constraints).
- **Group chrome (§3)** → **B-3**, not here. B-2 renders the bare placeholder box only.
- **Basis B** (no `SqliteBoardStore` wiring; ephemeral adapter) → Task 3, guarded per Global Constraints.

**Type-consistency check:** `pack::{Item, Kind, Placed, pack, cols_for_width, CELL_W, CELL_H, CARD_W, CARD_H, HEADER, GAP, INSET}` names are identical across Tasks 1/4. `BoardNode`/`board_tree`/`leaf_sessions` identical across Tasks 2/3/4. `set_visible`/`is_visible`/`visible` identical across the card (Task 5) and the gate (Task 5). `build_ephemeral_layout`/`visible_session_ids_for_test` identical across Tasks 3/4/5. No dangling references to removed `last_mode`/`invalidate_viewport_gate`/`recover_viewport_gates_on_reentry`.
