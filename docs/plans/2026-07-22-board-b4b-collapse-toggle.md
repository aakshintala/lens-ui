# Board B-4b — Collapse toggle + §7 collapsed-tile — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the group collapse toggle — the first real user write through B-4a's `run_op` seam — and render the §7 collapsed 1×1 status-rollup tile.

**Architecture:** A new idempotent `Op::SetCollapsed` rides B-4a's serialized commit-gated `run_op` (no optimistic machinery — that's B-4c). The renderer threads the persisted `collapsed` flag into the pack/render path: a collapsed group packs 1×1 and renders a status-count rollup instead of member cards, and its members are excluded from the visibility gate so no card views spawn for them. A caret-only `on_click` calls a `toggle_group_collapsed` method that issues the write. A unified "render `✓N` iff N>0" rule spans both chrome forms (folding a carried B-3 Minor).

**Tech Stack:** Rust, gpui 0.2.2, rusqlite (SqliteBoardStore), lens-core pure packer/domain, lens-ui board renderer.

## Global Constraints

- **Off-thread I/O only** — all SQLite access goes through B-4a's `run_op` → `cx.background_spawn`; the UI thread only `cx.update`/`cx.notify`. Never open/touch a store inline in `render` (AGENTS.md MANDATORY).
- **No-panic UI** — no `.unwrap()`/`.expect()` on runtime data paths; use typed errors / saturating arithmetic (matches B-3 `format_age`).
- **Commit-gated writes** — `SetCollapsed` flips the in-memory flag only on the persist reply; no optimistic apply (deferred to B-4c).
- **Render `✓N done` iff N > 0** — uniformly across collapsed footer and expanded header.
- **gpui test caveat** — `TestAppContext` fakes the text system; assert **structured chrome data** (snapshots), not painted pixels. Real caret-click + latency is the on-device verification step (Task 7). ([[gpui-test-noop-text-system]])
- **Wave taxonomy** — status rollup orders by the `derive_wave` priority ladder (NeedsInput · Failed · Working · AwaitingReview · Scheduled · Ready · Slept); `Wave::Neutral` is excluded. Supersedes §7's stale 5-status list.
- **Gate** — every task ends green under `cargo xtask gate` (clippy `-D warnings`, `fmt --check`, all crate suites + benches building). Never pipe the gate through `tail`.

**Spec:** `docs/specs/2026-07-22-board-b4b-collapse-toggle-design.md`.

---

### Task 1: `pack::Item::group_collapsed` — 1×1 collapsed group tile

**Files:**
- Modify: `crates/lens-core/src/pack.rs` (after `Item::group`, ~line 50; tests in the same file's `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pack::Item::group_collapsed(members: usize) -> Item` — a `Kind::Group { members }` item with `fc = 1, fr = 1` (footprint overridden regardless of member count, §7). Task 5 calls it for a collapsed group.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-core/src/pack.rs` tests:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-core --lib pack::tests::collapsed_group_packs_one_by_one`
Expected: FAIL — `no function or associated item named 'group_collapsed'`.

- [ ] **Step 3: Write minimal implementation**

Add to `impl Item` in `crates/lens-core/src/pack.rs` (right after `group`):

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-core --lib pack::tests::collapsed_group_packs_one_by_one`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/pack.rs
git commit -m "feat(pack): Item::group_collapsed — 1x1 collapsed group tile (B-4b §7)"
```

---

### Task 2: `status_rollup` — pure count-by-wave fold

**Files:**
- Modify: `crates/lens-ui/src/board/rollup.rs` (add struct + fn + tests)

**Interfaces:**
- Consumes: `crate::card::wave::Wave` (existing enum).
- Produces:
  - `rollup::StatusRollup { pub rows: Vec<(Wave, u32)> }` — non-empty waves only, priority-ladder order.
  - `rollup::status_rollup(member_waves: &[Wave]) -> StatusRollup`. Task 5 feeds it `derive_wave`-projected member waves.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-ui/src/board/rollup.rs` tests (`#[cfg(test)] mod tests`):

```rust
    #[test]
    fn status_rollup_counts_orders_and_drops_empties() {
        use crate::card::wave::Wave;
        // 2 Working, 1 Failed, 1 Ready, 1 Neutral (excluded).
        let waves = [
            Wave::Working,
            Wave::Ready,
            Wave::Working,
            Wave::Failed,
            Wave::Neutral,
        ];
        let r = status_rollup(&waves);
        // Ladder order: Failed before Working before Ready; Neutral absent; no zero rows.
        assert_eq!(
            r.rows,
            vec![(Wave::Failed, 1), (Wave::Working, 2), (Wave::Ready, 1)]
        );
    }

    #[test]
    fn status_rollup_empty_and_all_neutral_are_empty() {
        use crate::card::wave::Wave;
        assert!(status_rollup(&[]).rows.is_empty());
        assert!(status_rollup(&[Wave::Neutral, Wave::Neutral]).rows.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib board::rollup::tests::status_rollup_counts_orders_and_drops_empties`
Expected: FAIL — `cannot find function 'status_rollup'`.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/lens-ui/src/board/rollup.rs` (after `group_rollup`, before `format_group_spend`). Add the `Wave` import at the top of the fn or module:

```rust
use crate::card::wave::Wave;

/// The status-rollup body of a collapsed group (spec §7 / §4.1): one row per
/// non-empty wave, in `derive_wave` priority-ladder order. `Neutral` is excluded
/// (no meaningful status). Pure — the caller projects each member card to a `Wave`
/// via `derive_wave` and passes the slice; label + dot color are resolved at render.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusRollup {
    pub rows: Vec<(Wave, u32)>,
}

/// Priority ladder (matches `derive_wave`'s resolution order; `Neutral` omitted).
/// New waves inherit their ladder position here — no separate list to keep in sync.
const WAVE_LADDER: [Wave; 7] = [
    Wave::NeedsInput,
    Wave::Failed,
    Wave::Working,
    Wave::AwaitingReview,
    Wave::Scheduled,
    Wave::Ready,
    Wave::Slept,
];

pub fn status_rollup(member_waves: &[Wave]) -> StatusRollup {
    let rows = WAVE_LADDER
        .into_iter()
        .filter_map(|w| {
            let n = member_waves.iter().filter(|&&m| m == w).count() as u32;
            (n > 0).then_some((w, n))
        })
        .collect();
    StatusRollup { rows }
}
```

(If a top-of-module `use crate::card::wave::Wave;` conflicts with the per-fn import style, keep one module-level `use` and drop the in-test `use`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-ui --lib board::rollup::tests::status_rollup`
Expected: PASS (both `status_rollup_*` tests).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/rollup.rs
git commit -m "feat(board): status_rollup — pure count-by-wave fold, ladder order (B-4b §4.1)"
```

---

### Task 3: `Op::SetCollapsed` — commit-gated write op

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs` (Op enum ~17, OpOutcome ~22, `apply_outcome` ~262, `pump` re-gate ~234, `on_op_failed` ~380/396, `run_op_inner` ~490; import ~10; tests in-file)

**Interfaces:**
- Consumes: existing `SqliteBoardStore::set_collapsed(&BoardItemId, bool)`, `read_committed`, `load_state`, `write` (B-4a).
- Produces: `replica::Op::SetCollapsed { group_id: BoardItemId, collapsed: bool }` — Task 6 issues it via `BoardReplica::write`.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-ui/src/board/replica.rs` tests. `test_fleet`, `for_test_file`, `SqliteBoardStore`, `BoardId`, `DEFAULT_BOARD_ID`, `BoardItemKind` are already in scope in this module.

```rust
    #[gpui::test]
    async fn set_collapsed_round_trips_and_persists(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        // Seed a group into a real store on `path`, capture its id, drop the handle.
        let gid = {
            let store = SqliteBoardStore::open(&path).unwrap();
            store
                .create_group(&BoardId::new(DEFAULT_BOARD_ID), None, 0, "G")
                .unwrap()
        };

        let fleet = cx.update(test_fleet);
        let replica =
            cx.update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), path.clone(), cx)));
        cx.run_until_parked(); // Load the seeded group (collapsed == false).

        let is_collapsed = |r: &BoardReplica| {
            matches!(
                r.layout().item(&gid).map(|it| &it.kind),
                Some(BoardItemKind::Group { collapsed: true, .. })
            )
        };
        replica.read_with(cx, |r, _| assert!(!is_collapsed(r), "seeded expanded"));

        replica.update(cx, |r, cx| {
            r.write(
                Op::SetCollapsed {
                    group_id: gid.clone(),
                    collapsed: true,
                },
                cx,
            );
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Writable);
            assert!(is_collapsed(r), "flag flipped in the committed layout");
        });

        // Reopen the same path in a fresh replica — the collapse persisted.
        let fleet2 = cx.update(test_fleet);
        let replica2 =
            cx.update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet2, path.clone(), cx)));
        cx.run_until_parked();
        replica2.read_with(cx, |r, _| assert!(is_collapsed(r), "persisted across reopen"));
    }

    #[gpui::test]
    async fn set_collapsed_refused_when_non_writable(cx: &mut gpui::TestAppContext) {
        // A LoadFailed replica (bad path) refuses the write and counts it (banner honesty).
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
                Op::SetCollapsed {
                    group_id: BoardItemId::new("g_x"),
                    collapsed: true,
                },
                cx,
            )
        });
        assert!(matches!(disp, WriteDisposition::Rejected(ReplicaState::LoadFailed)));
        replica.read_with(cx, |r, _| assert_eq!(r.dropped_writes(), before + 1));
    }
```

Note: `write` returning `WriteDisposition::Rejected` does NOT itself increment `dropped_writes` in B-4a (only the `pump` re-gate does). If `set_collapsed_refused_when_non_writable`'s `dropped_writes` assertion fails for that reason, change it to assert only `matches!(disp, WriteDisposition::Rejected(ReplicaState::LoadFailed))` and drop the count assertion — the disposition is the contract. (Decide by the run in Step 2/4, don't guess.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica::tests::set_collapsed`
Expected: FAIL — `no variant named 'SetCollapsed'` / `no variant 'Wrote'`.

- [ ] **Step 3: Write minimal implementation**

In `crates/lens-ui/src/board/replica.rs`:

(a) Import `BoardItemId` — extend the `ids` use (line 10):

```rust
use lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};
```

(b) Add the `Op` variant (in `enum Op`, ~line 17):

```rust
pub(crate) enum Op {
    Load { initial: bool },
    PlaceSessions(Vec<(ConnectionId, SessionId)>),
    /// B-4b: idempotent group-collapse write (set the flag to an absolute value).
    SetCollapsed { group_id: BoardItemId, collapsed: bool },
}
```

(c) Add the `OpOutcome` variant (in `enum OpOutcome`, ~line 22) — a plain committed-layout swap, no place/reconcile bookkeeping:

```rust
    Wrote {
        layout: BoardLayout,
        skipped_empty: bool,
        mode: StoreMode,
    },
```

(d) Handle `Wrote` in `apply_outcome` (add an arm alongside `Placed`, ~line 280). Collapse changes no placements, so it does NOT reconcile:

```rust
            OpOutcome::Wrote {
                layout,
                skipped_empty,
                mode,
            } => {
                self.op_retries = 0;
                self.layout = Arc::new(layout);
                self.state = load_state(mode, skipped_empty);
            }
```

(e) `pump` re-gate — a write whose state flipped after it queued is dropped + counted (add an arm alongside the `PlaceSessions` guard, ~line 234):

```rust
                Some(Op::SetCollapsed { .. }) if !self.is_writable() => {
                    self.dropped_writes = self.dropped_writes.saturating_add(1);
                    continue;
                }
```

(f) `on_op_failed` — persistent-failure state arm for the new op (in the `match op`, ~line 380):

```rust
            Op::SetCollapsed { .. } => {
                self.state = ReplicaState::Stale; // keep current layout
            }
```

(g) `on_op_failed` — generalize the "drop queued writes on persistent failure" filter to cover ALL write ops, not just `PlaceSessions` (replace lines ~396–402):

```rust
        // Persistent failure: queued writes won't succeed on replay — drop (banner names them).
        let dropped = self
            .pending
            .iter()
            .filter(|o| !matches!(o, Op::Load { .. }))
            .count() as u32;
        self.dropped_writes = self.dropped_writes.saturating_add(dropped);
        self.pending.retain(|o| matches!(o, Op::Load { .. }));
```

(h) `run_op_inner` — persist + committed read (add an arm in the `match op`, ~line 499):

```rust
        Op::SetCollapsed { group_id, collapsed } => {
            store.set_collapsed(group_id, *collapsed)?; // persist (idempotent, absolute value)
            let (layout, skipped_empty, mode) = read_committed(store)?;
            Ok(OpOutcome::Wrote {
                layout,
                skipped_empty,
                mode,
            })
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica::tests::set_collapsed`
Expected: PASS (both). If `set_collapsed_refused_when_non_writable`'s count assertion fails, apply the Step-1 note (assert disposition only) and re-run.

- [ ] **Step 5: Run the full replica suite (no regressions in the B-4a write path)**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS (all existing B-4a tests + the 2 new ones).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): Op::SetCollapsed — commit-gated idempotent collapse write (B-4b §2)"
```

---

### Task 4: Unify `✓N iff N>0` + retire `group_header_text` (fold B-3 Minor)

This is a **pure refactor of the existing B-3 expanded-group render** — no collapse yet. It de-risks the `GroupChromeSnapshot` shape Task 5 extends, and closes B-3's carried "render-dead `group_header_text` / one-source" Minor by making the snapshot carry the *structured* chrome the elements render from.

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (`GroupChromeSnapshot` ~52, `absolute_group` header build ~372–450)
- Modify: `crates/lens-ui/src/board/rollup.rs` (remove `group_header_text` + its test)
- Modify: `crates/lens-ui/tests/acceptance_shell.rs` (the two group tests assert `snapshot.header` → switch to structured fields)

**Interfaces:**
- Produces: `GroupChromeSnapshot` with `spend_age: String` + `shows_completed: bool` (replaces `header: String`). Task 5 extends it further.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-ui/src/board/mod.rs` tests (`#[cfg(test)] mod tests`, uses the `#[gpui::test]` + `add_window_view` pattern already in this file). This asserts the unified rule at the snapshot level, seeding a group via a real store + `BoardReplica::new` (the B-4a acceptance pattern):

```rust
    #[gpui::test]
    async fn expanded_group_shows_completed_only_when_positive(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::board::{BoardId, DEFAULT_BOARD_ID};
        use lens_core::domain::ids::ConnectionId;
        use lens_core::persist::{BoardStore, PlacementTarget, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                f.spawn_fake_session(SessionId::new("s1"), cx);
            });
            fleet
        });
        let conn = ConnectionId::new("conn_test");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        store
            .place_session(
                &conn,
                &SessionId::new("s1"),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g1.clone()),
                    ordinal: Some(0),
                },
            )
            .unwrap();
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx
            .update(|cx| cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx)));
        cx.run_until_parked();

        let (board_view, vcx) =
            cx.add_window_view(|_, cx| BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx));
        vcx.run_until_parked();

        board_view.read_with(vcx, |b, _| {
            let chrome = b.group_chrome_for_test();
            assert_eq!(chrome.len(), 1);
            // completed_count is Archive-side (B-6) → 0 → the ✓N element is suppressed.
            assert_eq!(chrome[0].rollup.completed_count, 0);
            assert!(!chrome[0].shows_completed, "✓N hidden when count is 0");
        });
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib board::tests::expanded_group_shows_completed_only_when_positive`
Expected: FAIL — `no field 'shows_completed' on GroupChromeSnapshot`.

- [ ] **Step 3: Restructure the snapshot + header render (one source), suppress `✓0`**

In `crates/lens-ui/src/board/mod.rs`:

(a) Replace the `GroupChromeSnapshot` `header` field (~line 58) with structured fields:

```rust
#[derive(Clone, Debug)]
pub struct GroupChromeSnapshot {
    pub session_ids: Vec<SessionId>,
    pub name: String,
    pub accent: gpui::Hsla,
    pub rollup: rollup::GroupRollup,
    /// The "spend · age" middle text — the single source for the rendered element.
    pub spend_age: String,
    /// Whether the `✓N` element is rendered (⇔ `rollup.completed_count > 0`).
    pub shows_completed: bool,
}
```

(b) In `absolute_group` (~line 390), compute the shared values once and build both the snapshot and the elements from them. Replace the `let header = ...; let snapshot = ...;` block and the header-lane `.child(...)` for spend·age and ✓N:

```rust
        let rollup = rollup::group_rollup(&members, completed);
        let spend_age = format!(
            "{} · {}",
            rollup::format_group_spend(rollup.spend_usd),
            rollup::format_age(rollup.oldest_created_at, now_ms),
        );
        let shows_completed = rollup.completed_count > 0;

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            spend_age: spend_age.clone(),
            shows_completed,
        };
```

Then in the header-lane element (~line 433–447), render spend·age from `spend_age`, and gate the ✓N child on `shows_completed`. Replace the two `.child(...)` calls (the spend·age `div` and the `✓{completed}` `div`) with:

```rust
                .child(
                    div()
                        .flex_grow()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(spend_age.clone()),
                )
                .children(shows_completed.then(|| {
                    div()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(format!("✓{}", rollup.completed_count))
                }))
```

(`.children(Option<_>)` renders the element when `Some`, nothing when `None`.)

(c) Delete `group_header_text` and its `header_text_assembles_spec_order` test from `crates/lens-ui/src/board/rollup.rs` (it was only ever the snapshot string — now superseded by the structured fields; this retires the render-dead duplication).

- [ ] **Step 4: Fix the one acceptance assertion that used `snapshot.header`**

`crates/lens-ui/tests/acceptance_shell.rs` has exactly one `.header` reference — line 801 in `board_group_renders_chrome_and_rollup_via_replica`:

```rust
    assert_eq!(g.header, "Refactor · ~$3.50 · 2h · ✓0");
```

Replace it with the structured-field equivalents (same intent — name, spend·age, and the now-suppressed ✓N):

```rust
    assert_eq!(g.name, "Refactor");
    assert_eq!(g.spend_age, "~$3.50 · 2h");
    assert!(!g.shows_completed, "✓N suppressed at count 0");
```

(`group_rollup_refreshes_on_member_cost_change` ~813 asserts on `.rollup`, not `.header`, so it needs no change. Grep `.header` in the file to confirm no other reference before finishing.)

- [ ] **Step 5: Run the affected suites**

Run: `cargo test -p lens-ui --lib board::tests::expanded_group_shows_completed_only_when_positive`
Expected: PASS.
Run: `cargo test -p lens-ui --test acceptance_shell`
Expected: PASS (both group tests, now on structured fields).
Run: `cargo test -p lens-ui --lib board::rollup`
Expected: PASS (no `group_header_text` test remaining; compiles clean).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/src/board/rollup.rs crates/lens-ui/tests/acceptance_shell.rs
git commit -m "refactor(board): unify ✓N iff N>0, retire group_header_text — one source (B-4b §5, folds B-3 Minor)"
```

---

### Task 5: Collapsed render fork — 1×1 status-rollup tile + visibility exclusion

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (imports ~14–17; `GroupMeta` ~44; `GroupChromeSnapshot` ~52; `pack_and_render` node-build ~231–256 and tile-render ~296–305; new `absolute_collapsed_group`; tests)

**Interfaces:**
- Consumes: `pack::Item::group_collapsed` (Task 1), `rollup::status_rollup`/`StatusRollup` (Task 2), `crate::card::wave::{Wave, derive_wave}`, `crate::theme::ActiveLensTheme` (for `cx.lens_theme()`), `GroupChromeSnapshot.spend_age`/`shows_completed` (Task 4).
- Produces: a collapsed group renders a 1×1 tile; its members are excluded from the render's `visible` set; `GroupChromeSnapshot` gains `collapsed: bool` + `status_rows: Vec<(Wave, u32)>`.

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-ui/src/board/mod.rs` tests. Seeds a **collapsed** group (create_group → set_collapsed(true) → place two members) and asserts the fork: 1×1 pack, status rows present, and **members absent from `visible` + no card view**:

```rust
    #[gpui::test]
    async fn collapsed_group_renders_1x1_and_excludes_members_from_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        use lens_core::domain::board::{BoardId, DEFAULT_BOARD_ID};
        use lens_core::domain::ids::ConnectionId;
        use lens_core::domain::scalars::SessionStatusValue;
        use lens_core::persist::{BoardStore, PlacementTarget, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let conn = ConnectionId::new("conn_test");
        let (s1, s2) = (SessionId::new("s1"), SessionId::new("s2"));
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                for s in [&s1, &s2] {
                    let card = f.spawn_fake_session(s.clone(), cx);
                    card.update(cx, |c, _| c.status = SessionStatusValue::Running); // → Wave::Working
                }
            });
            fleet
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        for (i, s) in [&s1, &s2].into_iter().enumerate() {
            store
                .place_session(
                    &conn,
                    s,
                    &PlacementTarget {
                        board_id: Some(board.clone()),
                        parent_item_id: Some(g1.clone()),
                        ordinal: Some(i as i32),
                    },
                )
                .unwrap();
        }
        store.set_collapsed(&g1, true).unwrap(); // collapsed on disk
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();

        let (board_view, vcx) =
            cx.add_window_view(|_, cx| BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx));
        vcx.run_until_parked();

        board_view.read_with(vcx, |b, _| {
            let chrome = b.group_chrome_for_test();
            assert_eq!(chrome.len(), 1);
            assert!(chrome[0].collapsed, "group renders collapsed");
            // 2 Running members → one Working row, count 2.
            assert_eq!(
                chrome[0].status_rows,
                vec![(crate::card::wave::Wave::Working, 2)]
            );
            // THE FORK: collapsed members are NOT in the visible (card-view) set.
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert!(
                !visible.contains(&SessionId::new("s1")) && !visible.contains(&SessionId::new("s2")),
                "collapsed members must be excluded from the visibility gate"
            );
            // and no card view is instantiated for them (no hidden-entity leak).
            let views = b.card_views_for_test();
            assert!(
                !views.contains_key(&SessionId::new("s1"))
                    && !views.contains_key(&SessionId::new("s2")),
                "no card view spawned for a collapsed member"
            );
        });
    }
```

Sabotage check (do this mentally / by temporary edit): if the fork is wrong and members are pushed into `visible`, the last two assertions fail — confirming the test bites.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib board::tests::collapsed_group_renders_1x1_and_excludes_members_from_visible`
Expected: FAIL — `no field 'collapsed' on GroupChromeSnapshot` (and, once that compiles, the fork assertions fail because members are still visible).

- [ ] **Step 3: Implement the fork**

In `crates/lens-ui/src/board/mod.rs`:

(a) Imports — add to the top:

```rust
use crate::card::wave::{Wave, derive_wave};
use crate::theme::ActiveLensTheme as _;
use lens_core::domain::ids::BoardItemId;
```

(b) Extend `GroupMeta` (~line 44) with the id + collapsed flag:

```rust
struct GroupMeta {
    id: BoardItemId,
    name: String,
    color_token: Option<String>,
    collapsed: bool,
    completed_count: u32,
}
```

(c) Extend `GroupChromeSnapshot` (Task 4's struct) with two fields:

```rust
    pub collapsed: bool,
    pub status_rows: Vec<(Wave, u32)>,
```

(d) In `pack_and_render`'s node-build loop (~line 242), read the id + collapsed and choose the footprint:

```rust
                BoardNode::Group { item, .. } => {
                    let (name, color_token, collapsed) = match &item.kind {
                        lens_core::domain::board::BoardItemKind::Group {
                            name,
                            color_token,
                            collapsed,
                            ..
                        } => (name.clone(), color_token.clone(), *collapsed),
                        _ => (String::new(), None, false),
                    };
                    let meta = GroupMeta {
                        id: item.id.clone(),
                        name,
                        color_token,
                        collapsed,
                        completed_count: 0, // Archive-side (B-6)
                    };
                    let item = if collapsed {
                        Item::group_collapsed(sessions.len())
                    } else {
                        Item::group(sessions.len())
                    };
                    (item, Some(meta))
                }
```

(e) In the tile-render match (~line 296, the `pack::Kind::Group { .. }` arm), branch on the meta's `collapsed`, and **only push member sessions into `visible` for the expanded arm**. Note the current loop pushes `sessions` into `visible` for every tile *before* the match (~line 283–285) — move that push so groups control it. Restructure the per-tile body:

```rust
        for placed in &packing.tiles {
            if !placed.intersects_band(lo, hi) {
                continue; // culled
            }
            let sessions = &tile_sessions[placed.item_index];
            match placed.item.kind {
                pack::Kind::Card => {
                    visible.push(sessions[0].clone());
                    if let Some(tile) =
                        self.absolute_card(&sessions[0], placed.cell_left(), placed.cell_top() + HEADER, cx)
                    {
                        content = content.child(tile);
                    }
                }
                pack::Kind::Group { .. } => {
                    let meta = tile_groups[placed.item_index].as_ref();
                    let collapsed = meta.map(|m| m.collapsed).unwrap_or(false);
                    if collapsed {
                        // FORK: members feed the rollup (read inside), but are NOT visible —
                        // no card views spawn for a collapsed group's members.
                        let (el, snap) = self.absolute_collapsed_group(placed, sessions, meta, now_ms, cx);
                        content = content.child(el);
                        group_chrome.push(snap);
                    } else {
                        for s in sessions {
                            visible.push(s.clone());
                        }
                        let (els, snap) = self.absolute_group(placed, sessions, meta, now_ms, cx);
                        for el in els {
                            content = content.child(el);
                        }
                        group_chrome.push(snap);
                    }
                }
            }
        }
```

(Delete the old pre-match `for s in sessions { visible.push(...) }` block so visibility is decided per-arm.)

(f) Update `absolute_group` (Task 4) to also set the two new snapshot fields on its `GroupChromeSnapshot`: `collapsed: false, status_rows: Vec::new()`.

(g) Add the collapsed renderer. Place after `absolute_group`:

```rust
    /// A collapsed group (§7): a 1×1 tile reusing the group ring/accent/tint, with a
    /// header `● name · [spend · age] · ▸` and a body of status-rollup rows
    /// (`● N <label>` per non-empty wave), plus a `✓N done` footer rendered iff N>0.
    /// Members feed the rollup (read here) but are not rendered as cards.
    fn absolute_collapsed_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        meta: Option<&GroupMeta>,
        now_ms: i64,
        cx: &mut Context<Self>,
    ) -> (AnyElement, GroupChromeSnapshot) {
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = CELL_W - GAP;
        let block_h = CELL_H - GAP;

        let name = meta.map(|m| m.name.clone()).unwrap_or_default();
        let completed = meta.map(|m| m.completed_count).unwrap_or(0);
        let group_id = meta.map(|m| m.id.clone());
        let accent = group_accent(meta.and_then(|m| m.color_token.as_deref()));

        // Narrow projections read from member cards: cost/age (spend·age) + Wave (rollup).
        let (member_costs, member_waves): (Vec<rollup::MemberCost>, Vec<Wave>) = {
            let fleet = self.fleet.read(cx);
            let mut costs = Vec::with_capacity(sessions.len());
            let mut waves = Vec::with_capacity(sessions.len());
            for s in sessions {
                if let Some(e) = fleet.cards.get(s) {
                    let card = e.read(cx);
                    costs.push(rollup::MemberCost::from_card(card));
                    waves.push(derive_wave(card, now_ms, false));
                }
            }
            (costs, waves)
        };
        let rollup = rollup::group_rollup(&member_costs, completed);
        let status = rollup::status_rollup(&member_waves);
        let spend_age = format!(
            "{} · {}",
            rollup::format_group_spend(rollup.spend_usd),
            rollup::format_age(rollup.oldest_created_at, now_ms),
        );
        let shows_completed = rollup.completed_count > 0;

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            spend_age: spend_age.clone(),
            shows_completed,
            collapsed: true,
            status_rows: status.rows.clone(),
        };

        let theme = cx.lens_theme();
        let mut body = div()
            .absolute()
            .left(px(x))
            .top(px(y + HEADER))
            .w(px(block_w))
            .h(px(block_h - HEADER))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px_1p5();
        for (w, n) in &status.rows {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(div().size(px(8.0)).rounded_full().bg(w.status_color(theme)))
                    .child(
                        div()
                            .text_color(gpui::rgb(0xc4c4cc))
                            .child(format!("{n} {}", wave_rollup_label(*w))),
                    ),
            );
        }
        body = body.children(shows_completed.then(|| {
            div()
                .text_color(gpui::rgb(0x8a8a94))
                .child(format!("✓ {} done →", rollup.completed_count))
        }));

        // caret (▸) — Task 6 wires the on_click; render the glyph here.
        let _ = group_id; // Task 6 consumes this for the toggle handler
        let ring = div()
            .absolute()
            .left(px(x - INSET))
            .top(px(y - INSET))
            .w(px(block_w + 2.0 * INSET))
            .h(px(block_h + 2.0 * INSET))
            .rounded(px(12.0))
            .border_1()
            .border_color(accent)
            .bg(accent.opacity(0.07));
        let header = div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(block_w))
            .h(px(HEADER))
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_1p5()
            .child(div().size(px(8.0)).rounded_full().bg(accent))
            .child(div().text_color(gpui::rgb(0xd6d6de)).child(name.clone()))
            .child(
                div()
                    .flex_grow()
                    .text_color(gpui::rgb(0x8a8a94))
                    .child(spend_age.clone()),
            )
            .child(div().text_color(gpui::rgb(0x8a8a94)).child("▸"));

        let tile = div()
            .child(ring)
            .child(header)
            .child(body)
            .into_any_element();
        (tile, snapshot)
    }
```

(h) Add the label helper near `group_accent` (bottom of the file):

```rust
/// Title-case label for a status-rollup row (§7). `Neutral` is never in a rollup
/// (excluded by `status_rollup`), so it maps to an empty label.
fn wave_rollup_label(w: Wave) -> &'static str {
    match w {
        Wave::NeedsInput => "Needs input",
        Wave::Failed => "Failed",
        Wave::Working => "Working",
        Wave::AwaitingReview => "Awaiting review",
        Wave::Scheduled => "Scheduled",
        Wave::Ready => "Ready",
        Wave::Slept => "Slept",
        Wave::Neutral => "",
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p lens-ui --lib board::tests::collapsed_group_renders_1x1_and_excludes_members_from_visible`
Expected: PASS.

- [ ] **Step 5: Run the board + acceptance suites (no regression to expanded groups / culling)**

Run: `cargo test -p lens-ui --lib board`
Run: `cargo test -p lens-ui --test acceptance_shell`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): collapsed 1x1 status-rollup tile + visibility fork (B-4b §3–§4)"
```

---

### Task 6: Wire the caret toggle — `toggle_group_collapsed` + `on_click`

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (new `toggle_group_collapsed`; caret `on_click` in `absolute_group` ⌄ and `absolute_collapsed_group` ▸; thread `group_id` into `absolute_group`)
- Modify: `crates/lens-ui/src/board/replica.rs` (remove `#[allow(dead_code)]` on `write` — now live)

**Interfaces:**
- Consumes: `replica::Op::SetCollapsed` (Task 3), `BoardReplica::write`, `BoardLayout::item` + `BoardItemKind::Group { collapsed }`.
- Produces: `BoardView::toggle_group_collapsed(&mut self, group_id: BoardItemId, cx)` — the single entry point both carets call (and the acceptance test drives, standing in for the gpui hit-test).

- [ ] **Step 1: Write the failing test**

Add to `crates/lens-ui/src/board/mod.rs` tests — drive the toggle end-to-end through the method the caret closure calls, asserting the render fork flips both ways. Reuse the collapsed-seed helper shape (seed **expanded** this time):

```rust
    #[gpui::test]
    async fn toggle_group_collapsed_flips_render_and_visibility(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::board::{BoardId, DEFAULT_BOARD_ID};
        use lens_core::domain::ids::ConnectionId;
        use lens_core::domain::scalars::SessionStatusValue;
        use lens_core::persist::{BoardStore, PlacementTarget, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let conn = ConnectionId::new("conn_test");
        let (s1, s2) = (SessionId::new("s1"), SessionId::new("s2"));
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                for s in [&s1, &s2] {
                    let card = f.spawn_fake_session(s.clone(), cx);
                    card.update(cx, |c, _| c.status = SessionStatusValue::Running);
                }
            });
            fleet
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        for (i, s) in [&s1, &s2].into_iter().enumerate() {
            store
                .place_session(
                    &conn,
                    s,
                    &PlacementTarget {
                        board_id: Some(board.clone()),
                        parent_item_id: Some(g1.clone()),
                        ordinal: Some(i as i32),
                    },
                )
                .unwrap();
        }
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();
        let (board_view, vcx) =
            cx.add_window_view(|_, cx| BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx));
        vcx.run_until_parked();

        let collapsed = |b: &BoardView| b.group_chrome_for_test()[0].collapsed;
        board_view.read_with(vcx, |b, _| assert!(!collapsed(b), "starts expanded"));

        // Toggle → collapse (the caret closure calls exactly this).
        let gid = g1.clone();
        vcx.update(|_, cx| {
            board_view.update(cx, |b, cx| b.toggle_group_collapsed(gid.clone(), cx));
        });
        vcx.run_until_parked();
        board_view.read_with(vcx, |b, _| {
            assert!(collapsed(b), "collapsed after toggle");
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert!(visible.is_empty(), "collapsed members leave the visible set");
        });

        // Toggle again → expand.
        vcx.update(|_, cx| {
            board_view.update(cx, |b, cx| b.toggle_group_collapsed(g1.clone(), cx));
        });
        vcx.run_until_parked();
        board_view.read_with(vcx, |b, _| {
            assert!(!collapsed(b), "expanded after second toggle");
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert_eq!(visible.len(), 2, "members visible again");
        });
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib board::tests::toggle_group_collapsed_flips_render_and_visibility`
Expected: FAIL — `no method named 'toggle_group_collapsed'`.

- [ ] **Step 3: Add the method + wire the carets**

In `crates/lens-ui/src/board/mod.rs`:

(a) Add the method (near `card_click`, ~line 190):

```rust
    /// The caret toggle entry point (both ⌄ expanded and ▸ collapsed call this).
    /// Reads the current flag from the replica's committed layout, issues the flipped
    /// `SetCollapsed` write (commit-gated; a non-writable replica refuses + banners).
    fn toggle_group_collapsed(&mut self, group_id: BoardItemId, cx: &mut Context<Self>) {
        let current = matches!(
            self.replica.read(cx).layout().item(&group_id).map(|it| &it.kind),
            Some(lens_core::domain::board::BoardItemKind::Group { collapsed: true, .. })
        );
        self.replica.update(cx, |r, cx| {
            r.write(
                replica::Op::SetCollapsed {
                    group_id,
                    collapsed: !current,
                },
                cx,
            );
        });
    }
```

(b) Wire the collapsed `▸` caret in `absolute_collapsed_group` (Task 5). Replace the `let _ = group_id;` line and the plain `▸` child. Give the caret its own clickable element with `stop_propagation` ([[gpui-nested-click-stop-propagation]]):

```rust
        let caret = {
            let gid = group_id.clone();
            div()
                .id(("group-caret", placed.item_index))
                .cursor_pointer()
                .text_color(gpui::rgb(0x8a8a94))
                .child("▸")
                .on_click(cx.listener(move |board, _ev, _win, cx| {
                    cx.stop_propagation();
                    if let Some(gid) = gid.clone() {
                        board.toggle_group_collapsed(gid, cx);
                    }
                }))
        };
```

and use `.child(caret)` in the header instead of `.child(div()...child("▸"))`. (`group_id` here is `Option<BoardItemId>` from meta; keep the `if let Some` guard.)

(c) Wire the expanded `⌄` caret in `absolute_group` (~line 448). `absolute_group` currently has no `group_id` — thread it from meta at the top of the fn:

```rust
        let group_id = meta.map(|m| m.id.clone());
```

and replace the static caret child (`.child(div().text_color(gpui::rgb(0x8a8a94)).child("⌄"))`) with:

```rust
                .child({
                    let gid = group_id.clone();
                    div()
                        .id(("group-caret", placed.item_index))
                        .cursor_pointer()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child("⌄")
                        .on_click(cx.listener(move |board, _ev, _win, cx| {
                            cx.stop_propagation();
                            if let Some(gid) = gid.clone() {
                                board.toggle_group_collapsed(gid, cx);
                            }
                        }))
                })
```

(d) In `crates/lens-ui/src/board/replica.rs`, remove the now-obsolete `#[allow(dead_code)]` above `pub(crate) fn write` (~line 423) — it's live now. If clippy still flags it as unused in some build config, keep it; the gate run in Step 5 is authoritative.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p lens-ui --lib board::tests::toggle_group_collapsed_flips_render_and_visibility`
Expected: PASS.

- [ ] **Step 5: Full gate**

Run: `cargo xtask gate`
Expected: PASS — clippy `-D warnings`, `fmt --check`, lens-core + lens-client + lens-ui suites, benches build. (If `write`'s `#[allow(dead_code)]` removal now errors as "unused", the gate says so — restore it.)

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): caret toggle → SetCollapsed write, expanded ⌄ + collapsed ▸ (B-4b §2.2)"
```

---

### Task 7: On-device verification + review

**Files:** none (verification + review gate).

- [ ] **Step 1: Launch the demo and toggle collapse on-device**

The B-4a demo seeds a "Demo group" (B-3 chrome live). Launch it and exercise the real caret:

Run (in the session, so the display link has a foreground window — [[terminal-realwindow-harness-pitfalls]]):
```
! cargo run -p lens-ui --bin lens-demo --release
```
(Use the actual demo bin/target the B-4a handoff names if different — check `docs/handoffs/` / `Cargo.toml` `[[bin]]`.)

Observe: click the group's `⌄` → it collapses to a 1×1 status-rollup tile (member cards disappear, status rows appear, `▸` shown); click `▸` → it expands back. Other tiles reflow (snap, no animation — expected). No banner appears (writable).

- [ ] **Step 2: Read the commit-gated toggle latency (feeds B-4c)**

The collapse write is commit-gated: the flag flips on the persist reply. Confirm the toggle feels instant (no visible lag between click and repaint). Capture a rough reading (eyeball or a `log::debug!` timestamp around `toggle_group_collapsed` → next render) and **record it in STATUS.md / the B-4c seam note** — this is the datum B-4c uses to decide optimistic-vs-commit-gated. If it lags perceptibly, note that too; do NOT change the write model here (that's B-4c's call).

- [ ] **Step 3: At-scale sanity (no whole-board storm on collapse)**

Run:
```
! LENS_DEMO_N=125 cargo run -p lens-ui --bin lens-demo --release
```
Collapse/expand the seeded group at ~1000 cards; confirm it stays smooth (the §6.2 status-read invalidation is scoped to the collapsed group's members; collapsing *reduces* live card views). Note any frame-budget regression against the B-4a ~120fps reading.

- [ ] **Step 4: Cross-family + Opus review**

Per project rules ([[codex-as-reviewer]], [[review-spend-policy]], [[whole-branch-review-needs-a-builder]]):
- **codex gpt-5.6** whole-branch review (`codex exec -s read-only`) of the diff — focus: the `SetCollapsed` idempotent-retry safety, the visibility fork (no leaked card views), the `✓N` one-source refactor.
- **Opus** whole-branch synthesis review (a gate-running reviewer + adjudication of any divergence).

Fold findings; re-run `cargo xtask gate`.

- [ ] **Step 5: Update STATUS + memory**

Per [[end-of-session-status-update]]: update `docs/STATUS.md` (B-4b EXECUTED, the latency reading, B-4c's decision datum) and roll detail into `STATUS-ARCHIVE.md`. Save a `board-b4b-executed` memory (commit-gated latency result, the visibility-fork gotcha, the retired `group_header_text`) + update `MEMORY.md`.

---

## Self-Review

**Spec coverage** (against `docs/specs/2026-07-22-board-b4b-collapse-toggle-design.md`):
- §2 commit-gated `SetCollapsed` + idempotent retry → Task 3. ✓
- §2.1 `Op` variant + `run_op_inner` arm → Task 3. ✓
- §2.2 caret-only `on_click` + `stop_propagation` → Task 6. ✓
- §3 thread `collapsed` into UI, `board_tree` keeps members, branch `pack_and_render` → Task 5. ✓
- §3.1 visibility fork (members excluded from `visible`, no card views) → Task 5 (sabotage-verified). ✓
- §4 collapsed tile chrome (ring/tint/header `▸`/status rows/footer) → Task 5. ✓
- §4.1 `status_rollup` pure fold, `derive_wave` projection, ladder order, Neutral excluded, `now_ms` note → Task 2 (fold) + Task 5 (projection). ✓
- §5 unified `✓N iff N>0` both forms → Task 4 (expanded) + Task 5 (collapsed footer). ✓
- §6 folds B-3 Minor (one-source header, live snapshot assertion) → Task 4. ✓
- §6 defaults: snap (no code — inherent) ✓; §6.2 invalidation carryforward → Task 7 Step 3 note. ✓
- §7 tests: round-trip+persist (T3), state gating (T3), 1×1 pack (T1/T5), visibility fork sabotage (T5), status_rollup units (T2), `✓N` both forms (T4/T5), caret toggle (T6), on-device latency (T7). ✓
- §9 review → Task 7 Step 4. ✓

**Placeholder scan:** no "TBD"/"handle edge cases"/"similar to". The one `let _ = group_id;` in Task 5 is explicitly consumed by Task 6 (noted at both ends). ✓

**Type consistency:** `Op::SetCollapsed { group_id: BoardItemId, collapsed: bool }`, `OpOutcome::Wrote`, `status_rollup(&[Wave]) -> StatusRollup { rows: Vec<(Wave,u32)> }`, `GroupChromeSnapshot { …, spend_age, shows_completed, collapsed, status_rows }`, `Item::group_collapsed(usize)`, `toggle_group_collapsed(BoardItemId, cx)`, `wave_rollup_label(Wave)` — consistent across tasks. Snapshot grows once in Task 4 (`spend_age`/`shows_completed`, retiring `header`), then Task 5 (`collapsed`/`status_rows`); every consumer updated in the same task. ✓
