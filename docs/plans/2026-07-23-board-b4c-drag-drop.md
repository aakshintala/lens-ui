# Board B-4c drag-drop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship drag-to-reorder and drag in/out of groups with edge auto-scroll, via a commit-gated `Op::MoveItem` plus an ephemeral reflow-preview drag state machine.

**Architecture:** A pure reverse hit-test resolver in `lens_core::pack` maps a frozen `pack::Placed` snapshot (dragged item removed) to a `DropTarget`; `board/drag.rs` owns the Idle→Dragging→Committing state machine and reflow-preview (gap placeholder, never a second card). The write rides B-4a's serialized `run_op` exactly like `Op::SetCollapsed` — layout mutates only on `Wrote`. External mid-drag commits bump `layout_generation` and abandon the drag.

**Tech Stack:** Rust, gpui 0.2.2, sqlite (lens-core board store)

## Global Constraints

- **Commit-gated write model** — no optimistic apply, no rollback-snapshot machinery; in-memory layout changes only on the off-thread persist `Wrote` reply (mirrors `SetCollapsed`).
- **Resolver is pure in `lens_core::pack`** — `resolve_drop` + `to_move_ordinal` live with the packer over real `pack::Placed`; UI-free unit tests.
- **Drag glue in `board/drag.rs`** — gpui `on_drag` / `on_drag_move` / `on_drop`, state machine, reflow-preview, edge auto-scroll only; no packing math in the UI crate.
- **Groups do NOT nest via drag** — a dragged group only reorders among top-level siblings; into-group body branch is card-only (§6).
- **Empty groups persist** — dragging the last card out leaves an empty group; dissolve/ungroup is deferred (B-4d non-idempotent tier).
- **"Append to end" is CUT** — no dedicated bottom drop zone / append rule; the resolver is total via nearest-tile + reading-order side.
- **Frozen snapshot `S` resolves targets** — never the live reflow-preview; breaks the reflow feedback loop (§4.1).
- **`layout_generation` bump abandons the drag** — any external `Wrote`/`Loaded` (not this drag's own `MoveItem`) tears down during Dragging and aborts at drop (§3.4).

**Spec:** `docs/specs/2026-07-23-board-b4c-drag-drop-design.md`.
**Spike:** `docs/spikes/2026-07-23-board-b4c-drag-reverse-hittest.md` + `spikes/board-drag/src/lib.rs`.

---

## ⚠️ Spec/code mismatches found

Source-verified against the tree on 2026-07-23 (use these, not the stale line cites in the spec prose):

1. **`run_op_inner` SetCollapsed arm is at `replica.rs:547`, not `:560`.** Spec §2.1 cites `:560`; that line is `read_committed`. Mirror the arm at **:547**.
2. **SetCollapsed tests to mirror are at `replica.rs:777` / `:814` (Op call sites), not `:748`.** Spec §8 cites `:748` — that is only the start of `set_collapsed_round_trips_and_persists`; the `Op::SetCollapsed { .. }` pattern is at **:777** (round-trip) and **:814** (refused).
3. **`BoardStore::move_item` already exists** (`persist/board.rs` trait `:50`, `SqliteBoardStore` impl `:577`). Spec §2.1 says "the store persist wrapper is the new plumbing" — **it is not new**. Task 3 wires `Op::MoveItem` → the existing `store.move_item`; do not re-implement the persist method.
4. **§7 focused-rail `fc` clamp appears already landed** in `pack.rs`: `reshape_to_cols` (`:67`) is called from `pack` (`:151`), with regression tests `group_reflows_to_narrow_container` (`:324`) and `group_reflows_to_two_cols` (`:338`). Spec/STATUS prose still describes the unclamped-`fc` bug. Task 1 **verifies** the existing behavior (named focused-rail regression); no second clamp implementation.
5. **Spec cites `board.rs:380` for cycle rejection — confirmed** (the `CycleDetected` guard starts at `:380`). Domain `move_item` at `:359` confirmed.
6. **Spike lacks the §6 nesting test** (`dragged-group-over-group-body-falls-through-to-top-level`). Spec §8 requires it; Task 2 adds it alongside the 10 spike ports (adapted over `DropTile` / `pack::Placed`).
7. **`board/drag.rs` does not exist yet** — create under `crates/lens-ui/src/board/` and `mod drag;` in `board/mod.rs`.

---

## File Structure

| Path | Role |
| --- | --- |
| `crates/lens-core/src/pack.rs` | Already: packer + `Placed` + `reshape_to_cols`. **Add:** `DropTile`, `DropTarget`, `DraggedKind`, `resolve_drop`, `to_move_ordinal`, helper scan fns, resolver unit tests (Task 1 verify + Task 2). |
| `crates/lens-core/src/persist/board.rs` | **No new persist API** — `BoardStore::move_item` already at `:50`/`:577`. Task 3 only *calls* it from the replica. |
| `crates/lens-core/src/domain/board.rs:359` | Domain `move_item` (already exists; cycle reject `:380`). Untouched except as the semantic ground truth. |
| `crates/lens-ui/src/board/replica.rs` | **Add** `Op::MoveItem { item_id, new_board_id, new_parent, new_ordinal }` beside `SetCollapsed` (`:23`); commit-gate arm beside `:250`; `run_op_inner` arm beside `:547`; Failed-arm match beside `:422`; `layout_generation` counter bumped on external Loaded/Placed/Wrote. Tests mirroring `:777`/`:814`. |
| `crates/lens-ui/src/board/drag.rs` | **Create** — drag state machine (`DragState`), frozen snapshot `S`, reflow-preview pure helpers, edge-band scroll nudge, entity/pure tests (§3 / §8). |
| `crates/lens-ui/src/board/mod.rs` | `mod drag;`; wire `on_drag` / `on_drag_move` / `on_drop` on scrolled content; ghost payload `BoardItemId`; gap placeholder render; nudge `board_scroll` (`:113`). |
| `spikes/board-drag/` | **Delete** after Tasks 1–6 land (Task 7 hygiene, spec §9). |

---
### Task 1: §7 fold-in — verify focused-rail group `fc` clamp

**Files:**
- Modify: `crates/lens-core/src/pack.rs` (`reshape_to_cols` already at `:67`; `pack` already calls it at `:151`; existing tests `:324`/`:338`)
- Test: same file's `#[cfg(test)] mod tests` — add an explicitly-named focused-rail regression

**Interfaces:**
- Consumes: existing `pack::pack`, `Item::group`, `reshape_to_cols` (private).
- Produces: confirmed invariant — packing a multi-col group into `cols=1` stores reshaped `(fc, fr)` on `Placed.item` (focused rail = same container at 1 col). No new public API.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-core/src/pack.rs` tests (alongside `group_reflows_to_narrow_container`):

```rust
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
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p lens-core --lib pack::tests::focused_rail_group_reflows_to_one_col`

Expected:
- If `reshape_to_cols` is intact (current tree): the test **passes immediately** — treat that as "§7 already landed"; skip Step 3 implementation and go to Step 4 confirmation.
- If somehow regressing: FAIL with assertion on `(t.item.fc, t.item.fr)` (e.g. left as `(2, 2)`).

- [ ] **Step 3: Minimal implementation**

Only if Step 2 failed. Restore / ensure `pack` reshapes before placing (already present — do not duplicate):

```rust
// inside pack(), per item (pack.rs ~147–152):
let it = reshape_to_cols(it, cols);
let fc = it.fc.max(1).min(cols);
```

With `reshape_to_cols`:

```rust
fn reshape_to_cols(it: &Item, cols: usize) -> Item {
    match it.kind {
        Kind::Group { members } if it.fc > cols => {
            let fc = it.fc.min(cols).max(1);
            let fr = members.div_ceil(fc).max(1);
            Item { kind: it.kind, fc, fr }
        }
        _ => *it,
    }
}
```

If Step 2 was already green: **no code change** — leave a one-line note in the commit body that §7 was pre-landed.

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p lens-core --lib pack::tests::focused_rail_group_reflows_to_one_col`
Expected: PASS.

Also confirm the sibling still passes:
`cargo test -p lens-core --lib pack::tests::group_reflows_to_narrow_container` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/pack.rs
git commit -m "$(cat <<'EOF'
test(pack): focused-rail group-reflow regression (B-4c §7)

EOF
)"
```

---
### Task 2: Pure resolver in `lens_core::pack` — `resolve_drop` + `to_move_ordinal`

**Files:**
- Modify: `crates/lens-core/src/pack.rs` (after `impl Placed`, before `#[cfg(test)]`; tests in same file)
- Reference (port, do not copy toy model): `spikes/board-drag/src/lib.rs`

**Interfaces:**
- Consumes: `pack::Placed`, `Kind`, `CARD_H`/`CARD_W`/`CELL_W`/`GAP`/`HEADER`, `BoardItemId` (`lens_core::domain::ids`).
- Produces (exact signatures — later tasks must match):

```rust
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

/// `snapshot` = frozen S: packed tiles with the dragged item already removed (§4.1).
pub fn resolve_drop(
    snapshot: &[DropTile],
    cursor: (f32, f32),
    dragged: DraggedKind,
) -> DropTarget;

pub fn to_move_ordinal(spatial_ordinal: usize, dragged_sibling_index: Option<usize>) -> usize;
```

Translation note: spike `resolve_drop(board, cols, cursor)` re-packs from toy `TopItem`. Production takes **already-packed** `DropTile`s (geometry from real `Placed`); tests build them via `pack(&[Item::…], cols)` then zip ids. `DraggedKind::Group` skips the into-group body branch (§6).

- [ ] **Step 1: Write the failing tests**

Add helpers + all ports to `pack.rs` tests. Keep exact assertion values where the model maps cleanly:

```rust
use crate::domain::ids::BoardItemId;

fn bid(s: &str) -> BoardItemId { BoardItemId::new(s) }

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
    assert_eq!(t, DropTarget { parent: None, ordinal: 1 });
}

#[test]
fn drop_below_center_inserts_after() {
    let items = [Item::card(), Item::card(), Item::card()];
    let s = snap(&items, &["a", "b", "c"], &[false; 3], 1);
    let py1 = CARD_H + GAP;
    let t = resolve_drop(&s, (CARD_W / 2.0, py1 + CARD_H - 4.0), DraggedKind::Card);
    assert_eq!(t, DropTarget { parent: None, ordinal: 2 });
}

#[test]
fn drop_below_everything_resolves_after_nearest() {
    // Spec cut the dedicated "append to end" rule; total nearest-tile still
    // lands after the last card when the cursor is far below (spike value 2).
    let items = [Item::card(), Item::card()];
    let s = snap(&items, &["a", "b"], &[false; 2], 1);
    let t = resolve_drop(&s, (CARD_W / 2.0, 10_000.0), DraggedKind::Card);
    assert_eq!(t, DropTarget { parent: None, ordinal: 2 });
}

#[test]
fn ordinal_is_not_spatial_order_under_backfill() {
    let items = [Item::group(4), Item::card(), Item::card()];
    let s = snap(&items, &["g", "x", "y"], &[false; 3], 3);
    let x_col2 = 2.0 * CELL_W + CARD_W / 2.0;
    let t = resolve_drop(&s, (x_col2, 4.0), DraggedKind::Card);
    assert_eq!(t, DropTarget { parent: None, ordinal: 1 });
    let py_y = CARD_H + GAP;
    let t_low = resolve_drop(&s, (x_col2, py_y + 4.0), DraggedKind::Card);
    assert_eq!(t_low, DropTarget { parent: None, ordinal: 2 });
}

#[test]
fn drop_into_expanded_group_body_targets_member_slot() {
    let items = [Item::group(4)];
    let s = snap(&items, &["g"], &[false], 3);
    let t = resolve_drop(&s, (4.0, HEADER + CARD_H / 2.0), DraggedKind::Card);
    assert_eq!(t, DropTarget { parent: Some(bid("g")), ordinal: 0 });
    let x_m1 = CELL_W + CARD_W - 4.0;
    let t2 = resolve_drop(&s, (x_m1, HEADER + CARD_H / 2.0), DraggedKind::Card);
    assert_eq!(t2, DropTarget { parent: Some(bid("g")), ordinal: 2 });
}

#[test]
fn empty_trailing_cell_in_partial_last_row_appends() {
    // Codex input (618.0, 218.0) → ordinal 5.
    let items = [Item::group(5)];
    let s = snap(&items, &["g"], &[false], 3);
    let t = resolve_drop(&s, (618.0, 218.0), DraggedKind::Card);
    assert_eq!(t, DropTarget { parent: Some(bid("g")), ordinal: 5 });
    let m4_center_x = CELL_W + CARD_W / 2.0;
    let y_row1 = HEADER + (CARD_H + GAP) + CARD_H / 2.0;
    assert_eq!(
        resolve_drop(&s, (m4_center_x - 20.0, y_row1), DraggedKind::Card),
        DropTarget { parent: Some(bid("g")), ordinal: 4 }
    );
    assert_eq!(
        resolve_drop(&s, (m4_center_x + 20.0, y_row1), DraggedKind::Card),
        DropTarget { parent: Some(bid("g")), ordinal: 5 }
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
    // §6: into-group branch is card-only; a dragged group never nests.
    let items = [Item::group(4), Item::group(2)];
    let s = snap(&items, &["g0", "g1"], &[false; 2], 3);
    // Cursor in g0's body (would be into-group for a card).
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
    assert_eq!(t, DropTarget { parent: None, ordinal: 0 });
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p lens-core --lib pack::tests::drop_above_center_inserts_before`

Expected: FAIL — `cannot find function \`resolve_drop\`` (or `DropTile` / `DropTarget` not found).

- [ ] **Step 3: Minimal implementation**

Add to `crates/lens-core/src/pack.rs` (after `impl Placed`):

```rust
use crate::domain::ids::BoardItemId;

#[derive(Clone, Debug)]
pub struct DropTile {
    pub placed: Placed,
    pub id: BoardItemId,
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

pub fn resolve_drop(
    snapshot: &[DropTile],
    cursor: (f32, f32),
    dragged: DraggedKind,
) -> DropTarget {
    let (cx, cy) = cursor;
    // (1) into expanded group body — card drags only (§6).
    if dragged == DraggedKind::Card {
        for tile in snapshot {
            let Kind::Group { members } = tile.placed.item.kind else { continue };
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
    let col = (((cx - x0) / CELL_W).floor() as isize).clamp(0, fc as isize - 1) as usize;
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
    let Some((_, k)) = best else { return 0 };
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
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p lens-core --lib pack::tests::`

Filter for the new names, or run the whole pack suite:
`cargo test -p lens-core --lib pack::tests`

Expected: PASS for all eleven resolver tests listed in Step 1 (10 spike ports + §6 nesting).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/pack.rs
git commit -m "$(cat <<'EOF'
feat(pack): resolve_drop + to_move_ordinal over Placed (B-4c §4)

EOF
)"
```

---
### Task 3: Write path — `Op::MoveItem` + commit-gate + `run_op_inner`

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs` — `Op` enum `:17`, re-gate `:250`, Failed match `:422`, `run_op_inner` `:514`/`:547`, tests after SetCollapsed (`:777`/`:814`)
- Call (do not recreate): `BoardStore::move_item` in `crates/lens-core/src/persist/board.rs:50` / `:577`
- Domain ground truth: `BoardLayout::move_item` `crates/lens-core/src/domain/board.rs:359`

**Interfaces:**
- Consumes: `store.move_item(&BoardItemId, &BoardId, Option<BoardItemId>, i32) -> Result<()>`; `read_committed`; `is_writable` (`:234`); `pump` in_flight (`:239`).
- Produces:

```rust
Op::MoveItem {
    item_id: BoardItemId,
    new_board_id: BoardId,
    new_parent: Option<BoardItemId>,
    new_ordinal: i32,
}
```

Also: `layout_generation: u64` on `BoardReplica`, bumped on every `Loaded` / `Placed` / `Wrote` that is **not** this drag's own reply (Task 4 reads it; bump all Wrote for now and let drag.rs compare generations — see Step 3 note). Expose `pub fn layout_generation(&self) -> u64`.

- [ ] **Step 1: Write the failing test**

Add after the SetCollapsed tests in `replica.rs` (mirror `:777` / `:814`):

```rust
fn top_level_card_ids(layout: &BoardLayout) -> Vec<BoardItemId> {
    let mut cards: Vec<(i32, BoardItemId)> = layout
        .items
        .iter()
        .filter(|i| i.parent_item_id.is_none())
        .filter_map(|i| match &i.kind {
            BoardItemKind::Card { .. } => Some((i.ordinal, i.id.clone())),
            _ => None,
        })
        .collect();
    cards.sort_by_key(|(ord, _)| *ord);
    cards.into_iter().map(|(_, id)| id).collect()
}

#[gpui::test]
async fn move_item_round_trips_reorder_and_persists(cx: &mut gpui::TestAppContext) {
    // Seed + reopen pattern mirrors `load_reads_persisted_card` / SetCollapsed round-trip.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    {
        let store = SqliteBoardStore::open(&path).unwrap();
        let c = ConnectionId::new("conn_test");
        let target = PlacementTarget {
            board_id: None,
            parent_item_id: None,
            ordinal: None,
        };
        store
            .place_session(&c, &SessionId::new("a"), &target)
            .unwrap();
        store
            .place_session(&c, &SessionId::new("b"), &target)
            .unwrap();
    }
    let fleet = cx.update(test_fleet);
    let replica = cx
        .update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), path.clone(), cx)));
    cx.run_until_parked();
    let (id0, id1, board_id) = replica.read_with(cx, |r, _| {
        let ids = top_level_card_ids(r.layout());
        assert_eq!(ids.len(), 2);
        (
            ids[0].clone(),
            ids[1].clone(),
            r.layout().default_board_id().unwrap().clone(),
        )
    });
    // Move id0 after id1 → new_ordinal 1.
    replica.update(cx, |r, cx| {
        r.write(
            Op::MoveItem {
                item_id: id0.clone(),
                new_board_id: board_id.clone(),
                new_parent: None,
                new_ordinal: 1,
            },
            cx,
        );
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::Writable);
        assert_eq!(top_level_card_ids(r.layout()), vec![id1.clone(), id0.clone()]);
    });
    let fleet2 = cx.update(test_fleet);
    let replica2 =
        cx.update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet2, path.clone(), cx)));
    cx.run_until_parked();
    replica2.read_with(cx, |r, _| {
        assert_eq!(top_level_card_ids(r.layout()), vec![id1, id0]);
    });
}

#[gpui::test]
async fn move_item_round_trips_into_and_out_of_group(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    let gid = {
        let store = SqliteBoardStore::open(&path).unwrap();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        let gid = store.create_group(&board_id, None, 0, "G").unwrap();
        let c = ConnectionId::new("conn_test");
        store
            .place_session(
                &c,
                &SessionId::new("loose"),
                &PlacementTarget {
                    board_id: None,
                    parent_item_id: None,
                    ordinal: None,
                },
            )
            .unwrap();
        gid
    };
    let fleet = cx.update(test_fleet);
    let replica = cx
        .update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), path.clone(), cx)));
    cx.run_until_parked();
    let (card_id, board_id) = replica.read_with(cx, |r, _| {
        let layout = r.layout();
        let board_id = layout.default_board_id().unwrap().clone();
        let card_id = layout
            .items
            .iter()
            .find(|it| matches!(it.kind, BoardItemKind::Card { .. }))
            .map(|it| it.id.clone())
            .expect("seeded card");
        (card_id, board_id)
    });
    replica.update(cx, |r, cx| {
        r.write(
            Op::MoveItem {
                item_id: card_id.clone(),
                new_board_id: board_id.clone(),
                new_parent: Some(gid.clone()),
                new_ordinal: 0,
            },
            cx,
        );
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        let it = r.layout().item(&card_id).unwrap();
        assert_eq!(it.parent_item_id.as_ref(), Some(&gid));
    });
    replica.update(cx, |r, cx| {
        r.write(
            Op::MoveItem {
                item_id: card_id.clone(),
                new_board_id: board_id.clone(),
                new_parent: None,
                new_ordinal: 0,
            },
            cx,
        );
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        assert!(r.layout().item(&card_id).unwrap().parent_item_id.is_none());
    });
}

#[gpui::test]
async fn move_item_refused_when_non_writable(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(test_fleet);
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet, "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked();
    let before = replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::LoadFailed);
        r.dropped_writes()
    });
    let disp = replica.update(cx, |r, cx| {
        r.write(
            Op::MoveItem {
                item_id: BoardItemId::new("i_x"),
                new_board_id: BoardId::new(DEFAULT_BOARD_ID),
                new_parent: None,
                new_ordinal: 0,
            },
            cx,
        )
    });
    assert!(matches!(
        disp,
        WriteDisposition::Rejected(ReplicaState::LoadFailed)
    ));
    replica.read_with(cx, |r, _| assert_eq!(r.dropped_writes(), before + 1));
}

#[gpui::test]
async fn move_item_idempotent_rerun_is_noop(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(test_fleet);
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();
    let c = ConnectionId::new("conn_test");
    replica.update(cx, |r, cx| {
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("a"))]), cx);
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("b"))]), cx);
    });
    cx.run_until_parked();
    let (item_id, board_id, order_before) = replica.read_with(cx, |r, _| {
        let ids = top_level_card_ids(r.layout());
        (
            ids[0].clone(),
            r.layout().default_board_id().unwrap().clone(),
            ids,
        )
    });
    let make_op = || Op::MoveItem {
        item_id: item_id.clone(),
        new_board_id: board_id.clone(),
        new_parent: None,
        new_ordinal: 0, // already at 0 → no-op
    };
    replica.update(cx, |r, cx| {
        r.write(make_op(), cx);
    });
    cx.run_until_parked();
    replica.update(cx, |r, cx| {
        r.write(make_op(), cx);
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        assert_eq!(top_level_card_ids(r.layout()), order_before);
    });
}
```

Implementer note: Prefer rebuilding `Op::MoveItem` via a closure over deriving `Clone` on `Op`. `sibling_ids_sorted` is **private** — use `layout.items` + ordinal sort (`top_level_card_ids` above). Seed with `SqliteBoardStore::place_session` + `for_test_file` (same as `load_reads_persisted_card`) or `Op::PlaceSessions` via `run_op` (same as `two_place_ops_apply_in_enqueue_order`).

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p lens-ui --lib board::replica::tests::move_item_refused_when_non_writable`

Expected: FAIL — `no variant or associated item named \`MoveItem\` found for enum \`Op\``.

- [ ] **Step 3: Minimal implementation**

1. Extend `Op` (`replica.rs:17`):

```rust
Op::MoveItem {
    item_id: BoardItemId,
    new_board_id: BoardId,
    new_parent: Option<BoardItemId>,
    new_ordinal: i32,
},
```

(Derive or impl `Clone` on `Op` if the idempotent test needs it — `SetCollapsed` fields are already cloneable.)

2. Re-gate arm beside `:250`:

```rust
Some(Op::MoveItem { .. }) if !self.is_writable() => {
    self.dropped_writes = self.dropped_writes.saturating_add(1);
    continue;
}
```

3. Failed persistent-failure match beside `:422`:

```rust
Op::MoveItem { .. } => {
    self.state = ReplicaState::Stale;
}
```

4. `run_op_inner` arm beside SetCollapsed `:547`:

```rust
Op::MoveItem {
    item_id,
    new_board_id,
    new_parent,
    new_ordinal,
} => {
    store.move_item(item_id, new_board_id, new_parent.clone(), *new_ordinal)?;
    let (layout, skipped_empty, mode) = read_committed(store)?;
    Ok(OpOutcome::Wrote {
        layout,
        skipped_empty,
        mode,
    })
}
```

5. `layout_generation: u64` field on `BoardReplica`, init `0`. In `apply_outcome`, after assigning `self.layout` for `Loaded` / `Placed` / `Wrote`, do `self.layout_generation = self.layout_generation.wrapping_add(1);`. Expose:

```rust
pub fn layout_generation(&self) -> u64 {
    self.layout_generation
}
```

Task 4 records the generation at drag-start and treats any bump as external (including this drag's own Wrote — Committing ends before needing generation; abandon only compares during Dragging / at drop before issue). Optional refinement: skip bump when the completed op was `MoveItem` *and* drag owns it — not required if drag exits Committing→Idle on Wrote without consulting generation.

- [ ] **Step 4: Run test, verify it passes**

Run:
```
cargo test -p lens-ui --lib board::replica::tests::move_item_round_trips_reorder_and_persists
cargo test -p lens-ui --lib board::replica::tests::move_item_round_trips_into_and_out_of_group
cargo test -p lens-ui --lib board::replica::tests::move_item_refused_when_non_writable
cargo test -p lens-ui --lib board::replica::tests::move_item_idempotent_rerun_is_noop
```
Expected: PASS all four.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "$(cat <<'EOF'
feat(board): Op::MoveItem commit-gated write path (B-4c §2)

EOF
)"
```

---
### Task 4: Drag state machine + reflow-preview in `board/drag.rs`

**Files:**
- Create: `crates/lens-ui/src/board/drag.rs`
- Modify: `crates/lens-ui/src/board/mod.rs` — add `mod drag;` (wiring to gpui is Task 5)
- Consumes replica: `layout_generation()` from Task 3; `Op::MoveItem` from Task 3
- Consumes pack: `DropTile`, `DropTarget`, `DraggedKind`, `resolve_drop`, `to_move_ordinal` from Task 2

**Interfaces:**
- Consumes: Task 2 resolver; Task 3 `Op::MoveItem` + `layout_generation`.
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Idle,
    Dragging,
    Committing,
}

#[derive(Clone, Debug)]
pub struct DragSession {
    pub phase: DragPhase,
    pub dragged_id: BoardItemId,
    pub dragged_kind: DraggedKind,
    pub start_generation: u64,
    /// Frozen S — packed tiles with dragged removed (§4.1).
    pub snapshot: Vec<DropTile>,
    /// Last resolved target (updated on move; held through Committing).
    pub target: DropTarget,
    /// Sibling index of dragged within its parent at drag-start (for to_move_ordinal).
    pub dragged_sibling_index: Option<usize>,
    /// Parent at drag-start (`None` = top-level) — for same-parent shift.
    pub start_parent: Option<BoardItemId>,
    pub board_id: BoardId,
}

pub fn start_drag(
    dragged_id: BoardItemId,
    dragged_kind: DraggedKind,
    snapshot: Vec<DropTile>,
    start_generation: u64,
    initial_cursor: (f32, f32),
    dragged_sibling_index: Option<usize>,
    start_parent: Option<BoardItemId>,
    board_id: BoardId,
) -> DragSession;

pub fn on_cursor_move(session: &mut DragSession, cursor: (f32, f32), current_generation: u64) -> bool;
// returns false if abandoned (generation bumped)

pub fn begin_commit(session: &mut DragSession, current_generation: u64) -> Option<Op>;
// None = aborted (generation changed or not Dragging)

pub fn on_wrote(session: &mut DragSession);
pub fn on_failed(session: &mut DragSession);
pub fn cancel(session: &mut DragSession);

/// Pure: placeholder footprint (fc, fr) for the empty gap — NOT a second card view.
pub fn reflow_preview_placeholder_footprint(item: &Item) -> (usize, usize);
```

`begin_commit` returns `Op::MoveItem` (`Op` is `pub(crate)` in `replica`; `drag` is a sibling `board` module).

- [ ] **Step 1: Write the failing test**

Create `crates/lens-ui/src/board/drag.rs` with tests module first (file can be tests-only until Step 3):

```rust
#[cfg(test)]
mod tests {
    use lens_core::domain::board::DEFAULT_BOARD_ID;
    use lens_core::domain::ids::{BoardId, BoardItemId};
    use lens_core::pack::{pack, DropTile, DraggedKind, Item};

    fn bid(s: &str) -> BoardItemId { BoardItemId::new(s) }
    fn board() -> BoardId { BoardId::new(DEFAULT_BOARD_ID) }

    fn three_card_snapshot() -> Vec<DropTile> {
        // S = board with "b" removed (dragged).
        let items = [Item::card(), Item::card()]; // a, c
        let packing = pack(&items, 1);
        packing
            .tiles
            .into_iter()
            .zip(["a", "c"])
            .map(|(placed, id)| DropTile {
                placed,
                id: bid(id),
                collapsed: false,
            })
            .collect()
    }

    fn start_b() -> DragSession {
        start_drag(
            bid("b"),
            DraggedKind::Card,
            three_card_snapshot(),
            1,
            (140.0, 4.0),
            Some(1),
            None,
            board(),
        )
    }

    #[test]
    fn dragging_to_committing_holds_preview_target() {
        let mut s = start_b();
        assert_eq!(s.phase, DragPhase::Dragging);
        let held = s.target.clone();
        assert!(begin_commit(&mut s, 1).is_some());
        assert_eq!(s.phase, DragPhase::Committing);
        assert_eq!(s.target, held, "Committing holds last reflow-preview target");
    }

    #[test]
    fn wrote_invisible_swap_returns_idle() {
        let mut s = start_b();
        let _ = begin_commit(&mut s, 1);
        on_wrote(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn failed_discards_preview_returns_idle() {
        let mut s = start_b();
        let _ = begin_commit(&mut s, 1);
        on_failed(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
        assert!(s.snapshot.is_empty());
    }

    #[test]
    fn cancel_returns_idle() {
        let mut s = start_b();
        cancel(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn external_commit_mid_drag_abandons_during_dragging() {
        let mut s = start_b();
        assert!(!on_cursor_move(&mut s, (140.0, 80.0), 2));
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn external_commit_aborts_at_drop() {
        let mut s = start_b();
        assert!(begin_commit(&mut s, 2).is_none(), "generation changed → abort");
        assert_eq!(s.phase, DragPhase::Idle);
    }
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p lens-ui --lib board::drag::tests::dragging_to_committing_holds_preview_target`

Expected: FAIL — module `drag` not found / `start_drag` not found (add `mod drag;` first if needed so the compile error is the missing fn).

- [ ] **Step 3: Minimal implementation**

`crates/lens-ui/src/board/drag.rs`:

```rust
use lens_core::domain::ids::{BoardId, BoardItemId};
use lens_core::pack::{
    resolve_drop, to_move_ordinal, DropTarget, DropTile, DraggedKind, Item,
};

use crate::board::replica::Op;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Idle,
    Dragging,
    Committing,
}

#[derive(Clone, Debug)]
pub struct DragSession {
    pub phase: DragPhase,
    pub dragged_id: BoardItemId,
    pub dragged_kind: DraggedKind,
    pub start_generation: u64,
    pub snapshot: Vec<DropTile>,
    pub target: DropTarget,
    pub dragged_sibling_index: Option<usize>,
    /// Parent at drag-start (`None` = top-level) — for same-parent shift.
    pub start_parent: Option<BoardItemId>,
    pub board_id: BoardId,
}

pub fn start_drag(
    dragged_id: BoardItemId,
    dragged_kind: DraggedKind,
    snapshot: Vec<DropTile>,
    start_generation: u64,
    initial_cursor: (f32, f32),
    dragged_sibling_index: Option<usize>,
    start_parent: Option<BoardItemId>,
    board_id: BoardId,
) -> DragSession {
    let target = resolve_drop(&snapshot, initial_cursor, dragged_kind);
    DragSession {
        phase: DragPhase::Dragging,
        dragged_id,
        dragged_kind,
        start_generation,
        snapshot,
        target,
        dragged_sibling_index,
        start_parent,
        board_id,
    }
}

pub fn on_cursor_move(
    session: &mut DragSession,
    cursor: (f32, f32),
    current_generation: u64,
) -> bool {
    if session.phase != DragPhase::Dragging {
        return false;
    }
    if current_generation != session.start_generation {
        cancel(session);
        return false;
    }
    session.target = resolve_drop(&session.snapshot, cursor, session.dragged_kind);
    true
}

pub fn begin_commit(
    session: &mut DragSession,
    current_generation: u64,
) -> Option<Op> {
    if session.phase != DragPhase::Dragging {
        return None;
    }
    if current_generation != session.start_generation {
        cancel(session);
        return None;
    }
    let sibling_idx = if session.target.parent == session.start_parent {
        session.dragged_sibling_index
    } else {
        None
    };
    let new_ordinal = to_move_ordinal(session.target.ordinal, sibling_idx) as i32;
    let op = Op::MoveItem {
        item_id: session.dragged_id.clone(),
        new_board_id: session.board_id.clone(),
        new_parent: session.target.parent.clone(),
        new_ordinal,
    };
    session.phase = DragPhase::Committing;
    Some(op)
}

pub fn on_wrote(session: &mut DragSession) {
    *session = idle_shell(session);
}

pub fn on_failed(session: &mut DragSession) {
    *session = idle_shell(session);
}

pub fn cancel(session: &mut DragSession) {
    *session = idle_shell(session);
}

fn idle_shell(session: &DragSession) -> DragSession {
    DragSession {
        phase: DragPhase::Idle,
        dragged_id: session.dragged_id.clone(),
        dragged_kind: session.dragged_kind,
        start_generation: session.start_generation,
        snapshot: Vec::new(),
        target: DropTarget {
            parent: None,
            ordinal: 0,
        },
        dragged_sibling_index: None,
        start_parent: None,
        board_id: session.board_id.clone(),
    }
}

pub fn reflow_preview_placeholder_footprint(item: &Item) -> (usize, usize) {
    (item.fc.max(1), item.fr.max(1))
}
```

In `board/mod.rs`: `mod drag;`

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p lens-ui --lib board::drag::tests`

Expected: PASS (all six state-machine tests).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/drag.rs crates/lens-ui/src/board/mod.rs
git commit -m "$(cat <<'EOF'
feat(board): drag state machine + reflow-preview helpers (B-4c §3)

EOF
)"
```

---
### Task 5: gpui wiring — on_drag / on_drag_move / on_drop + gap placeholder + edge auto-scroll

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` — `BoardView` fields; `pack_and_render` (~`:391`); scrolled content element; `board_scroll` (`:113`)
- Modify: `crates/lens-ui/src/board/drag.rs` — edge-band helper (pure, unit-tested)
- Uses: Task 4 `DragSession` / `start_drag` / `on_cursor_move` / `begin_commit` / `on_wrote` / `on_failed`

**Interfaces:**
- Consumes: Task 4 state machine; Task 2 `resolve_drop` (via drag.rs); Task 3 `replica.write(Op::MoveItem)`.
- Produces:
  - `BoardView.drag: Option<DragSession>` (or always-held `DragSession` with Idle phase).
  - Ghost payload type = `BoardItemId` via gpui `on_drag(id, |id, cx| { … ghost view … })`.
  - Reflow-preview render: empty gap placeholder reserving `fc × fr` at the resolved slot (no second card).
  - `edge_scroll_delta(cursor_y, viewport_top, viewport_h, band_px, nudge_px) -> f32` in `drag.rs`.
  - Handlers bound on the **scrolled content** element so `cursor − bounds.origin` is content-local (§5).

- [ ] **Step 1: Write the failing test**

Add to `drag.rs` tests (pure edge math — gpui handlers are verified in Task 6 real-window):

```rust
#[test]
fn edge_scroll_nudges_near_top_and_bottom() {
    // Viewport [0, 600]; band 40px; nudge 12px per move event.
    assert_eq!(edge_scroll_delta(10.0, 0.0, 600.0, 40.0, 12.0), -12.0); // toward top
    assert_eq!(edge_scroll_delta(590.0, 0.0, 600.0, 40.0, 12.0), 12.0); // toward bottom
    assert_eq!(edge_scroll_delta(300.0, 0.0, 600.0, 40.0, 12.0), 0.0); // middle → none
}

#[test]
fn reflow_preview_uses_gap_not_second_card() {
    // Structural: placeholder footprint equals the dragged item's fc×fr; the
    // renderer must paint an empty reserved slot (asserted via a small pure
    // helper that BoardView will call).
    let card = Item::card();
    assert_eq!(reflow_preview_placeholder_footprint(&card), (1, 1));
    let g = Item::group(4);
    assert_eq!(reflow_preview_placeholder_footprint(&g), (g.fc, g.fr));
}
```

Also add a `BoardView`-level unit/entity test only if an existing snapshot hook can assert "during Dragging, last preview exposes a gap id / placeholder marker" without painting — otherwise keep entity coverage in Task 4 and defer visual proof to Task 6.

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p lens-ui --lib board::drag::tests::edge_scroll_nudges_near_top_and_bottom`

Expected: FAIL — `cannot find function \`edge_scroll_delta\``.

- [ ] **Step 3: Minimal implementation**

1. In `drag.rs`:

```rust
/// Positive delta scrolls content down (cursor near bottom); negative toward top.
pub fn edge_scroll_delta(
    cursor_y: f32,
    viewport_top: f32,
    viewport_h: f32,
    band_px: f32,
    nudge_px: f32,
) -> f32 {
    let y = cursor_y - viewport_top;
    if y <= band_px {
        -nudge_px
    } else if y >= viewport_h - band_px {
        nudge_px
    } else {
        0.0
    }
}
```

Band width / nudge are constants on `BoardView` (e.g. `EDGE_BAND_PX: f32 = 40.0`, `EDGE_NUDGE_PX: f32 = 12.0`) — **on-device tuning** in Task 6; these defaults are the starting point only.

2. On `BoardView`: field `drag: Option<DragSession>` (None = Idle).

3. In tile render (card / group chrome), attach:

```rust
.div()
  .on_drag(item_id.clone(), |id, cx| {
      // Ghost: lightweight card/group chrome clone following the cursor.
      // Payload type = BoardItemId.
      cx.new(|_| DragGhost { id: id.clone() })
  })
```

Implement `DragGhost` as a tiny `Render` view in `drag.rs` (title/body stub is enough — real chrome polish is Task 6 eyes).

4. On the scrolled **content** element inside `pack_and_render` (the same node that already receives absolute tile children):

```rust
.on_drag_move(cx.listener(|this, event: &DragMoveEvent<BoardItemId>, window, cx| {
    let bounds = event.bounds; // painted, already scrolled
    let cursor = event.event.position;
    let local = (
        f32::from(cursor.x) - f32::from(bounds.origin.x),
        f32::from(cursor.y) - f32::from(bounds.origin.y),
    );
    let gen = this.replica.read(cx).layout_generation();
    if let Some(ref mut session) = this.drag {
        if !drag::on_cursor_move(session, local, gen) {
            this.drag = None;
            cx.notify();
            return;
        }
        let dy = drag::edge_scroll_delta(
            f32::from(cursor.y),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.height),
            EDGE_BAND_PX,
            EDGE_NUDGE_PX,
        );
        if dy != 0.0 {
            // Nudge board_scroll toward the edge (§5).
            let mut off = this.board_scroll.offset();
            off.y -= px(dy); // gpui scroll offset sign: confirm on device in Task 6
            this.board_scroll.set_offset(off);
        }
        cx.notify();
    }
}))
.on_drop(cx.listener(|this, id: &BoardItemId, _window, cx| {
    let gen = this.replica.read(cx).layout_generation();
    let Some(ref mut session) = this.drag else { return };
    if &session.dragged_id != id {
        return;
    }
    if let Some(op) = drag::begin_commit(session, gen) {
        this.replica.update(cx, |r, cx| { r.write(op, cx); });
        // Stay in Committing until Wrote/Failed observed via replica observe.
    } else {
        this.drag = None;
    }
    cx.notify();
}))
```

5. Drag **start**: when `on_drag` constructor runs (or a pre-drag hook), build frozen `S`:
   - Run the same items→`pack` path as `pack_and_render`.
   - Zip `DropTile { placed, id, collapsed }`.
   - **Remove** the dragged tile from the vec.
   - `start_drag(…, replica.layout_generation(), …)`.

6. Reflow-preview render (while `drag.phase` is `Dragging` or `Committing`):
   - Build pack items from committed layout **with the dragged item relocated to `session.target`** (parent + spatial ordinal), OR pack `S` + insert a placeholder `Item` at the target slot.
   - For the dragged id's slot, render an **empty** `div` sized to `fc * CELL_W - GAP` × `item_height` — **not** a `SessionCardView` / group chrome copy.
   - Ghost carries the real card visuals.

7. Replica observe (already `cx.observe(&replica, …)`): the trigger is **phase-gated, not layout-diffed** (do NOT compare committed layout to the held preview — that is the fragile path):
   - **In `Committing`, on ANY replica layout update → `on_wrote` → clear drag.** The drag's own `MoveItem` `Wrote` is the common case (invisible swap, §3.2). If an *external* `PlaceSessions`/`Load` `Wrote` lands first during the ~1ms window, this clears the preview one frame early — but the `MoveItem` is already enqueued (`write` was called at drop) and `pump` serializes on `in_flight`, so it still commits at its resolved ordinal; the visible result is the §3.4 "swap plus one new card," which is accepted, not a divergence. This is why the trigger needs no generation check in `Committing` (see Task 3 Step 3 note #5).
   - If the replica surfaces a **persistent** failure (state → `Stale`, dropped-write banner): `on_failed` → clear drag, committed layout unchanged.
   - If `layout_generation` bumped while still `Dragging` (handled in `on_cursor_move`, Step 3 above): tear down (`cancel`) immediately, do not wait for drop.

Exact gpui `DragMoveEvent` field names — confirm against gpui `0.2.2` docs/source at implement time (`event.event.position` vs `event.position`); keep content-local math as `cursor − bounds.origin`.

- [ ] **Step 4: Run test, verify it passes**

Run:
```
cargo test -p lens-ui --lib board::drag::tests::edge_scroll_nudges_near_top_and_bottom
cargo test -p lens-ui --lib board::drag::tests::reflow_preview_uses_gap_not_second_card
cargo test -p lens-ui --lib board::drag::tests
```
Expected: PASS.

Also: `cargo check -p lens-ui` green (handlers compile).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/drag.rs crates/lens-ui/src/board/mod.rs
git commit -m "$(cat <<'EOF'
feat(board): gpui drag wiring, gap preview, edge auto-scroll (B-4c §5)

EOF
)"
```

---
### Task 6: Real-window verify (§8 — "the RUN is the only proof")

**Files:**
- No production source changes required unless on-device tuning demands constant tweaks (`EDGE_BAND_PX` / `EDGE_NUDGE_PX` in `board/mod.rs` or `drag.rs`) or a gpui ghost-abort quirk workaround (§3.4 note).
- Manual run surface: `cargo run -p lens-ui -- --demo` (or the repo's documented demo entrypoint — confirm via `cargo run -p lens-ui -- --help` / existing STATUS demo instructions).

**Interfaces:**
- Consumes: Tasks 1–5 end-to-end.
- Produces: a short verification note in the commit message (and optionally a one-line STATUS bullet if the branch already updates STATUS — do not invent a new doc file).

- [ ] **Step 1: Write the failing test**

There is **no** headless drag test. Spec §8 + spike explicitly forbid faking `active_drag` / hitboxes ([[false-green-probe-drives-production-path]]).

Create a checklist file is **not** required. Instead, record the manual matrix as comments in the commit body. The "failing" gate is: feature incomplete until the matrix is exercised on a real window.

Manual matrix (must all pass):
1. **Reorder** two top-level cards — drop above/below center matches preview; on release, layout commits with invisible swap (~1ms).
2. **Into group** — drag a card onto an expanded group's body; lands in member slot; partial last-row empty cell appends.
3. **Out of group** — drag a member onto the top-level masonry; empty group **persists**.
4. **Group reorder** — drag a group by its header; does not nest when over another group's body (§6).
5. **Collapsed group** — drop on rollup stays top-level (no into-group).
6. **Edge auto-scroll** — drag near top/bottom of the board viewport; `board_scroll` nudges; tune `EDGE_BAND_PX` / `EDGE_NUDGE_PX` until it feels right.
7. **External-commit abandon (best-effort)** — if a demo can spawn a session mid-drag, confirm preview tears down; ghost may linger until mouse-up (gpui caveat §3.4) and `on_drop` no-ops.

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo run -p lens-ui -- --demo`

Expected before this task is complete: drag either unimplemented or partially wired — exercise once to confirm the build launches. If Tasks 1–5 are done, proceed to Step 3 as the actual verify (this step documents the pre-tune baseline).

- [ ] **Step 3: Minimal implementation**

On-device only:
- Adjust `EDGE_BAND_PX` / `EDGE_NUDGE_PX` (and scroll-offset sign if inverted).
- Fix any gpui API mismatches discovered (`DragMoveEvent` fields, `set_offset` sign).
- Confirm ghost constructor does not spawn a second live `SessionCardView` (gap stays empty).
- If external-commit ghost linger is confusing, do **not** build a defer-placements polish — accepted per §3.4.

No new resolver or write-path logic here.

- [ ] **Step 4: Run test, verify it passes**

Re-run the Step 1 matrix against `--demo`. Expected: all seven behaviors observed; no panic; failed writes (if forced) discard preview and show the existing replica banner.

Also run: `cargo xtask gate` (or at minimum `cargo test -p lens-core --lib pack::tests` + `cargo test -p lens-ui --lib board::`) — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/src/board/drag.rs
git commit -m "$(cat <<'EOF'
test(board): real-window drag verify + edge-scroll tune (B-4c §8)

EOF
)"
```

If no source tweaks were needed: empty commit is forbidden — skip this commit and fold the verify note into Task 7's commit body instead.

---
### Task 7: Spike hygiene — delete `spikes/board-drag/`

**Files:**
- Delete: `spikes/board-drag/` (entire crate: `Cargo.toml`, `src/lib.rs`, and any bench/test leftovers)
- Modify: workspace `Cargo.toml` (remove the `spikes/board-drag` member if listed)
- Modify: any CI / xtask path that references `board-drag` (grep before deleting)

**Interfaces:**
- Consumes: Task 2 ports (resolver live in `lens_core::pack`); design doc already folds the spike verdict.
- Produces: spike gone; workspace still builds.

- [ ] **Step 1: Write the failing test**

Not a unit test — a workspace membership assertion. Grep first:

```bash
rg -n "board-drag" Cargo.toml spikes docs/STATUS.md .github 2>/dev/null
ls spikes/board-drag
```

Expected: paths exist (pre-delete). Record the member line in root `Cargo.toml` to edit.

- [ ] **Step 2: Run test, verify it fails**

The "failure" condition for this task is: `spikes/board-drag` still present after B-4c landed — confirm with `test -d spikes/board-drag && echo STILL_THERE`.

- [ ] **Step 3: Minimal implementation**

```bash
# Remove workspace member entry for spikes/board-drag from the root Cargo.toml.
# Then delete the crate:
rm -rf spikes/board-drag
```

Ensure no remaining references:

```bash
rg -n "board-drag" .
```

Expected: only historical mentions in `docs/specs/…`, `docs/spikes/…`, and this plan (leave those).

- [ ] **Step 4: Run test, verify it passes**

```bash
test ! -d spikes/board-drag && echo GONE
cargo check -p lens-core
cargo check -p lens-ui
cargo xtask gate
```

Expected: `GONE`; checks / gate PASS.

- [ ] **Step 5: Commit**

```bash
git add -A spikes/board-drag Cargo.toml
git commit -m "$(cat <<'EOF'
chore: delete spikes/board-drag after B-4c resolver land (§9)

EOF
)"
```

---
## Self-review

### 1. Spec coverage

| Spec section | Task |
| --- | --- |
| §1 Scope (reorder + in/out-group + edge auto-scroll; non-goals deferred) | Tasks 3–6 (write + drag + verify); B-4d/B-5/append/dissolve explicitly out |
| §2 Commit-gated write / `Op::MoveItem` | Task 3 |
| §2.1 re-gate / `run_op_inner` / store.move_item / idempotent | Task 3 (store already exists — call only) |
| §3 Feedback + state machine (ghost + reflow-preview gap; Idle/Dragging/Committing; Wrote/Failed) | Tasks 4–5 |
| §3.4 External commits / `layout_generation` abandon | Task 3 (counter) + Task 4 (tear-down / abort-at-drop) + Task 6 (ghost caveat) |
| §4 Resolver home in `lens_core::pack`; frozen S; `resolve_drop` / `to_move_ordinal` | Task 2 |
| §4.2 steps (into-group, header, collapsed, top-level nearest) | Task 2 tests |
| §4.3 append-to-end CUT; ordinal convention separate | Task 2 (`to_move_ordinal` + `drop_below_everything_resolves_after_nearest`) |
| §5 Edge auto-scroll / `board_scroll` / content-local handlers | Task 5 (+ tune in Task 6) |
| §6 Group drop semantics / no nesting via drag | Task 2 (`dragged_group_over_group_body_…`) + Task 5/6 group-drag wiring |
| §7 Focused-rail `fc` clamp | Task 1 (verify pre-landed `reshape_to_cols`) |
| §8 Test checklist (10 spike + nesting; write path; state machine; real-window) | Tasks 2–6 |
| §9 Spike hygiene | Task 7 |

### 2. Placeholder scan

Scanned for TBD / TODO / "similar to Task N" / "add error handling" / "write tests for the above" without code. Residual implementer notes are concrete (use `layout.items`, prefer closure over `Clone`, confirm gpui `DragMoveEvent` field names against 0.2.2 at wire time) — not placeholders. Task 6 correctly has no headless drag test (spec forbid).

### 3. Type consistency

Locked signatures across tasks:

- `DropTile { placed: Placed, id: BoardItemId, collapsed: bool }`
- `DraggedKind { Card, Group }`
- `DropTarget { parent: Option<BoardItemId>, ordinal: usize }`
- `resolve_drop(snapshot: &[DropTile], cursor: (f32, f32), dragged: DraggedKind) -> DropTarget`
- `to_move_ordinal(spatial_ordinal: usize, dragged_sibling_index: Option<usize>) -> usize`
- `Op::MoveItem { item_id: BoardItemId, new_board_id: BoardId, new_parent: Option<BoardItemId>, new_ordinal: i32 }`
- `start_drag(dragged_id, dragged_kind, snapshot, start_generation, initial_cursor, dragged_sibling_index, start_parent, board_id) -> DragSession`
- `begin_commit(session, current_generation) -> Option<Op>`
- `reflow_preview_placeholder_footprint(item: &Item) -> (usize, usize)`
- `edge_scroll_delta(cursor_y, viewport_top, viewport_h, band_px, nudge_px) -> f32`

Inline fixes applied during self-review: Task 3 tests rewritten to real `place_session` / `layout.items` patterns (dropped private `sibling_ids_sorted` / invented `test_fleet_with_two_sessions`); Task 4 Interfaces/`start_drag` aligned with Step 3 (`start_parent` + `board_id`); Step 1 drag tests use `start_b()` helper with the full signature.

