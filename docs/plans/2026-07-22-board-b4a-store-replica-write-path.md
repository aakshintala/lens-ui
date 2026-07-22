# Board B-4a — store→replica write-path foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ephemeral `build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced off-thread from `SqliteBoardStore` behind a main-thread in-memory replica, so the board renders from the real store and survives restarts — shipping **no** user interactions.

**Architecture:** A `BoardReplica` gpui entity owns an in-memory `BoardLayout` that every render reads for free (no I/O on the frame path). All SQLite access runs off-thread through a **serialized single-in-flight `run_op`** pump (`cx.spawn` → `background_executor().spawn` → `WeakEntity::update`). Ops (`Load`, `PlaceSessions`) apply in enqueue order, so the in-memory layout is the latest committed state at quiescence. A `FleetStore` observer drives an additive, conn-pinned, batched reconcile. Non-fatal error states (`ReplicaState`) gate writes and drive a non-blocking banner; recovery is an always-allowed reopen-`Load`.

**Tech Stack:** Rust, gpui 0.2.2, rusqlite, `lens-core` (`BoardStore`/`SqliteBoardStore`/`BoardLayout`), `lens-ui` (`FleetStore`/`BoardView`), criterion.

**Design doc (authority):** `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` (LOCKED, residual pass 2026-07-22). Section refs (§N) below point into it.

## Global Constraints

_Every task's requirements implicitly include this section._

- **Off-thread I/O (MANDATORY, AGENTS.md:19 / `.agents/rust-ui.md`):** all disk I/O runs inside a background task; the UI thread only does `cx.update`/`cx.notify`/`entity.update`. No `SqliteBoardStore` call on the render or update thread except the one bootstrap open in `main` (before `Application::new()`, no frame loop yet).
- **Never panic in the UI (AGENTS.md):** every store failure is non-fatal → a `ReplicaState` transition + banner, never `unwrap`/`expect` on a store `Result`.
- **Frame budget (MANDATORY):** 120fps / 8.3ms target, 90fps / 11.1ms regression line — asserted E2E on-device (Task 10), not by the pure `lens-core` bench alone.
- **Pinned connection (PROVISIONAL → B-5):** `BoardReplica.conn` = `ConnectionId::new("lens-app")` in prod, a fixed id in demo/test. This is what makes the two placement sources converge (§3.3/§4).
- **Single-in-flight ordering is load-bearing** (§2): exactly one op outstanding; replies apply in enqueue order. Required for correctness *and* deterministic tests.
- **Gate:** `cargo xtask gate` green — zero warnings, `cargo fmt --check`, all tests, benches build. Scope new crates into the gate's explicit `-p` list; never pipe the gate through `tail` (memory [[xtask-gate-scope]], [[terminal-spikes-process-learnings]]).
- **Commit** after each task's tests pass. Solo workflow: work on `main` is fine per [[integration-workflow]]; do **not** auto-push.

## Plan refinements over the spec sketch (read before Task 3)

The spec §2 sketches `store: Arc<Mutex<Box<dyn BoardStore>>>`. Two concrete refinements this plan makes, both faithful to the design's intent:

1. **Reopenable slot.** Recovery (§5) is a *fresh open behind the mutex*, which a bare `Box<dyn BoardStore>` can't do. The store field is `Arc<Mutex<StoreSlot>>` where `StoreSlot { path: PathBuf, store: Option<Box<dyn BoardStore + Send>> }` and `ensure_open` reopens from `path` when `store` is `None`. `+ Send` is required because the handle crosses into `background_executor().spawn` (rusqlite `Connection` is `Send`, not `Sync`; `Mutex` supplies `Sync`).
2. **`PlaceSessions` composes write + in-lock reload.** The trait's write methods return `Result<()>`, not the layout, so the op runner calls `place_sessions(...)` then `load_layout()` **under the same held lock** to produce the committed layout. Because the mutex + single-in-flight guarantee no interleaving writer, this is atomic in effect (dissolves review #2's *separate*-reload divergence). A post-commit reload failure → `Stale` with data safe on disk (recovery `Load` surfaces it).

## Test & construction conventions (verified against source — read before Task 3)

The repo has **no in-memory SQLite path** (`open_db` does `create_dir_all(path.parent())`, `db.rs:37`), and `FleetStore::fake` does not exist. Verified real primitives:

- **Store for tests/demo = a real file in a `tempfile::TempDir`**, not `:memory:`. The board tests already do this (`board.rs` test mod: `tempfile::tempdir()` + `SqliteBoardStore::open(dir.join("lens.db"))`). So the design's `:memory:` / `StoreSource::Memory` is dropped: the slot is `StoreSlot { path: PathBuf, store: Option<Box<dyn BoardStore + Send>> }`, and the replica holds a `_tempdir: Option<tempfile::TempDir>` to keep the file alive (None in prod, where `data_dir` is permanent). `ensure_open` reopens from `slot.path` uniformly — this is also how recovery (§5) reopens.
- **`in_memory_for_test` is renamed `for_test`** (tempfile-backed; the name would otherwise lie).
- **Fleet for tests = `FleetStore::new(clock, cx)`** — the *fake* ctor (`store.rs:28`, sets `fake: Some(FakeFleet::new())`, returns `Entity<Self>`). `new_live` (`:42`) is prod. The clock is `ManualUiClock::new(10_000)` cast to `Arc<dyn UiClock>` (`clock.rs`; used verbatim in `acceptance_shell.rs:59,64`).
- **Shared test helper** (define once in the `replica.rs` `#[cfg(test)]` mod; every `#[gpui::test]` below calls it):

```rust
#[cfg(test)]
fn test_fleet(cx: &mut gpui::App) -> Entity<FleetStore> {
    use std::sync::Arc;
    use crate::clock::{ManualUiClock, UiClock};
    FleetStore::new(Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>, cx)
}
```

- **All four ctors funnel through a private `build`** (installs the `FleetStore` observer + enqueues the first `Load`), so the observer is wired in exactly one place:

```rust
impl BoardReplica {
    fn build(
        store: Option<Box<dyn BoardStore + Send>>,
        path: PathBuf,
        conn: ConnectionId,
        tempdir: Option<tempfile::TempDir>,
        fleet: Entity<FleetStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            store: Arc::new(Mutex::new(StoreSlot { path, store })),
            conn,
            layout: default_board_layout(),
            state: ReplicaState::Loading,
            fleet,
            in_flight: false,
            pending: VecDeque::new(),
            reconcile_in_flight: false,
            banner_dismissed: false,
            _tempdir: tempdir,
        };
        cx.observe(&this.fleet.clone(), |this: &mut Self, _f, cx| this.reconcile(cx)).detach();
        this.run_op(Op::Load, cx);
        this
    }
}
```

---

## File structure

- `crates/lens-core/src/persist/db.rs` — **modify**: add `busy_timeout` PRAGMA (Task 1).
- `crates/lens-core/src/persist/board.rs` — **modify**: add `place_sessions` batch trait method + `SqliteBoardStore` impl (Task 2).
- `crates/lens-core/benches/board_pack.rs` — **create**: criterion bench for `board_tree`/pack math (Task 10).
- `crates/lens-ui/src/board/replica.rs` — **create**: `BoardReplica`, `Op`, `ReplicaState`, `StoreSlot`, the `run_op` pump, reconcile, recovery (Tasks 3–7).
- `crates/lens-ui/src/board/mod.rs` — **modify**: `BoardView` reads the replica; retire `test_layout`; observe replica; add banner; mount takes `replica` (Tasks 8, 9).
- `crates/lens-ui/src/board/layout_adapter.rs` — **delete** at Task 8 (stub retired).
- `crates/lens-app/src/main.rs` — **modify**: bootstrap-open the board store before actors; construct + pass `BoardReplica`; demo seeding (Tasks 7, 8).
- `crates/lens-ui/tests/acceptance_shell.rs` — **modify**: 5 `BoardView::mount` call sites gain a replica (Task 8).
- `spikes/board-container/` — **modify**: extend `measure.sh` / container to seed a group at scale (Task 10).

---

## Task 1: `busy_timeout` PRAGMA on the shared connection

**Files:**
- Modify: `crates/lens-core/src/persist/db.rs:54-58`
- Test: `crates/lens-core/src/persist/db.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `open_db(path, ddl, version) -> Result<(Connection, StoreMode)>` (existing).
- Produces: the returned `Connection` has `busy_timeout = 5000` ms set, covering open-time queries **and** every later write transaction on that connection (§5 write-failure contract, §6).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn open_db_sets_busy_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, _mode) = open_db(&dir.path().join("t.db"), CONTROL_DDL, SCHEMA_VERSION).unwrap();
    let ms: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ms, 5000, "busy_timeout must be set so SQLITE_BUSY retries in-driver");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-core --lib open_db_sets_busy_timeout`
Expected: FAIL — `assert_eq!` left `0` (default), right `5000`.

- [ ] **Step 3: Add the PRAGMA**

In `open_db`, set the busy timeout for **all** modes (it must cover the read-only-degraded reader's open query too), before the `ReadWrite`-only WAL block:

```rust
    // busy_timeout applies to open-time queries AND every later write txn on this
    // connection — absorbs SQLITE_BUSY from the ~dozen SqliteControlStore
    // connections on lens.db (design §5/§6). 5s: generous vs a human-scale op,
    // short enough that a truly stuck lock still surfaces as a (non-fatal) error.
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    if mode == StoreMode::ReadWrite {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(ddl)?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-core --lib open_db_sets_busy_timeout`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/persist/db.rs
git commit -m "feat(persist): busy_timeout on the store connection (absorbs SQLITE_BUSY)"
```

---

## Task 2: `place_sessions` batch method (O(N), one transaction)

**Files:**
- Modify: `crates/lens-core/src/persist/board.rs` (trait at `:16-57`, impl near `:430-469`)
- Test: `crates/lens-core/src/persist/board.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: existing `SqliteBoardStore` privates `guard_write`, `is_tombstoned`, `load_layout_inner`, `ensure_default_board`, `load_boards`, `new_item_id`, `now_ms`, `persist_board_items`, `touch_board`, `self.conn.unchecked_transaction`; domain `BoardLayout::place_session`, `find_card`, `item`.
- Produces: `fn place_sessions(&self, placements: &[(ConnectionId, SessionId)], target: &PlacementTarget) -> Result<()>` on `BoardStore` — places each non-tombstoned, not-already-present session, persisting each touched board **once** inside **one** transaction. Tombstoned/duplicate entries are silently skipped (matching single `place_session`).

**Why:** review #8 — k separate `place_session` calls each reload + persist the whole board (~O(k·N)); one batched transaction is O(N). Reconcile (Task 6) is the sole caller.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn place_sessions_batch_places_all_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
    let conn = ConnectionId::new("c1");
    let target = PlacementTarget { board_id: None, parent_item_id: None, ordinal: None };
    let keys = vec![
        (conn.clone(), SessionId::new("s1")),
        (conn.clone(), SessionId::new("s2")),
        (conn.clone(), SessionId::new("s3")),
    ];
    store.place_sessions(&keys, &target).unwrap();

    let layout = store.load_layout().unwrap().rows.into_iter().next().unwrap();
    let cards: Vec<_> = layout
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 3, "all three sessions placed");
    assert!(["s1", "s2", "s3"].iter().all(|s| cards.iter().any(|c| c == s)));
}

#[test]
fn place_sessions_skips_tombstoned_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
    let conn = ConnectionId::new("c1");
    let target = PlacementTarget { board_id: None, parent_item_id: None, ordinal: None };
    // s1 placed already; s2 tombstoned.
    store.place_session(&conn, &SessionId::new("s1"), &target).unwrap();
    tombstone_session(&store, &conn, &SessionId::new("s2")); // test helper: see note

    store
        .place_sessions(
            &[
                (conn.clone(), SessionId::new("s1")), // duplicate → skip
                (conn.clone(), SessionId::new("s2")), // tombstoned → skip
                (conn.clone(), SessionId::new("s3")), // new → place
            ],
            &target,
        )
        .unwrap();

    let layout = store.load_layout().unwrap().rows.into_iter().next().unwrap();
    let n = layout
        .items
        .iter()
        .filter(|i| matches!(i.kind, BoardItemKind::Card { .. }))
        .count();
    assert_eq!(n, 2, "s1 (dup) + s3 (new); s2 tombstoned stays absent");
}
```

> **Note on test setup:** stores are `tempfile::tempdir()` + `SqliteBoardStore::open(dir.join("lens.db"))` (the repo has no in-memory path). For `tombstone_session`, mirror the sibling test `place_tombstoned_session_is_noop` (`board.rs:1078`) to see how a row's `sessions.tombstoned_at` is set — do not invent a new harness.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core --lib place_sessions_batch`
Expected: FAIL — `no method named place_sessions`.

- [ ] **Step 3: Add the trait method**

In the `BoardStore` trait (after `place_session`, `:37`):

```rust
    /// Batch placement (§3.3): place each non-tombstoned, not-already-present
    /// session, persisting each touched board ONCE inside ONE transaction — O(N)
    /// vs k× `place_session`'s O(k·N). Tombstoned/duplicate entries are skipped.
    fn place_sessions(
        &self,
        placements: &[(ConnectionId, SessionId)],
        target: &PlacementTarget,
    ) -> Result<()>;
```

- [ ] **Step 4: Implement on `SqliteBoardStore`**

Add next to `place_session` (mirrors its logic, but loops before the single tx):

```rust
    fn place_sessions(
        &self,
        placements: &[(ConnectionId, SessionId)],
        target: &PlacementTarget,
    ) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        if layout.boards.is_empty() {
            self.ensure_default_board()?;
            layout.boards = self.load_boards()?.rows;
        }
        let mut touched: std::collections::BTreeSet<BoardId> = std::collections::BTreeSet::new();
        for (conn, session) in placements {
            if self.is_tombstoned(conn, session)? {
                continue;
            }
            if layout.find_card(conn, session).is_some() {
                continue;
            }
            let item_id = self.new_item_id("card");
            let created_at = self.now_ms();
            layout.place_session(conn.clone(), session.clone(), target, item_id.clone(), created_at)?;
            let board_id = layout.item(&item_id).expect("just inserted").board_id.clone();
            touched.insert(board_id);
        }
        if touched.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for board_id in &touched {
            self.persist_board_items(&layout, board_id)?;
            self.touch_board(board_id)?;
        }
        tx.commit()?;
        Ok(())
    }
```

> If any other `BoardStore` impls exist (e.g. a test fake), add a `place_sessions` there too — a default loop over `place_session` is acceptable for a non-Sqlite fake.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-core --lib place_sessions`
Expected: PASS (both).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-core/src/persist/board.rs
git commit -m "feat(persist): batch place_sessions (one txn, O(N)) for reconcile"
```

---

## Task 3: `BoardReplica` types + pure helpers (no async yet)

**Files:**
- Create: `crates/lens-ui/src/board/replica.rs`
- Modify: `crates/lens-ui/src/board/mod.rs` (add `mod replica;` near the other `mod`s)
- Test: `crates/lens-ui/src/board/replica.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces (consumed by Tasks 4–8):
  - `enum Op { Load, PlaceSessions(Vec<(ConnectionId, SessionId)>) }`
  - `enum ReplicaState { Loading, Writable, Degraded, LoadFailed, Stale }`
  - `struct StoreSlot { path: PathBuf, store: Option<Box<dyn BoardStore + Send>> }`
  - `struct BoardReplica { store: Arc<Mutex<StoreSlot>>, conn: ConnectionId, layout: BoardLayout, state: ReplicaState, fleet: Entity<FleetStore>, in_flight: bool, pending: VecDeque<Op>, reconcile_in_flight: bool, banner_dismissed: bool, _tempdir: Option<tempfile::TempDir> }` (see Test & construction conventions for `_tempdir`)
  - `BoardReplica::layout(&self) -> &BoardLayout`, `state(&self) -> ReplicaState`, `is_writable(&self) -> bool`
  - free fn `default_board_layout() -> BoardLayout` (one `board_default`/`Main` board, no items — so `default_board_id()` succeeds when the store gave us nothing)
  - free fn `state_after_load(skipped_empty: bool) -> ReplicaState`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn default_board_layout_has_a_default_board() {
    let l = default_board_layout();
    assert_eq!(l.default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
    assert!(l.items.is_empty());
}

#[test]
fn is_writable_only_in_writable_state() {
    assert!(state_is_writable(ReplicaState::Writable));
    for s in [ReplicaState::Loading, ReplicaState::Degraded, ReplicaState::LoadFailed, ReplicaState::Stale] {
        assert!(!state_is_writable(s), "{s:?} must gate writes");
    }
}

#[test]
fn load_with_skipped_rows_is_degraded() {
    assert_eq!(state_after_load(true), ReplicaState::Writable);
    assert_eq!(state_after_load(false), ReplicaState::Degraded); // skipped non-empty
}
```

> `state_is_writable(s)` is a free fn mirroring `is_writable`; keep the method `is_writable(&self) -> bool` = `state_is_writable(self.state)` so both are covered by one impl.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — unresolved names.

- [ ] **Step 3: Write the types + pure helpers**

```rust
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::Entity;
use lens_core::domain::board::{Board, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME};
use lens_core::domain::ids::{BoardId, ConnectionId, SessionId};
use lens_core::persist::BoardStore;

use crate::fleet::store::FleetStore;

#[derive(Debug)]
pub(crate) enum Op {
    Load,
    PlaceSessions(Vec<(ConnectionId, SessionId)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaState {
    Loading,
    Writable,
    Degraded,
    LoadFailed,
    Stale,
}

pub(crate) struct StoreSlot {
    pub(crate) path: PathBuf,
    pub(crate) store: Option<Box<dyn BoardStore + Send>>,
}

pub struct BoardReplica {
    pub(crate) store: Arc<Mutex<StoreSlot>>,
    pub(crate) conn: ConnectionId,
    pub(crate) layout: BoardLayout,
    pub(crate) state: ReplicaState,
    pub(crate) fleet: Entity<FleetStore>,
    pub(crate) in_flight: bool,
    pub(crate) pending: VecDeque<Op>,
    pub(crate) reconcile_in_flight: bool,
    pub(crate) banner_dismissed: bool,
    /// Keeps a test/demo `TempDir`'s file alive for the replica's lifetime; None in
    /// prod (the `data_dir` file is permanent). Reopen (`ensure_open`) uses `slot.path`.
    pub(crate) _tempdir: Option<tempfile::TempDir>,
}

pub(crate) fn state_is_writable(s: ReplicaState) -> bool {
    matches!(s, ReplicaState::Writable)
}

/// Load succeeded; `Degraded` iff some rows were skipped (kept observable, §5).
pub(crate) fn state_after_load(skipped_empty: bool) -> ReplicaState {
    if skipped_empty {
        ReplicaState::Writable
    } else {
        ReplicaState::Degraded
    }
}

/// A non-empty layout with just the default board, so `default_board_id()`
/// succeeds when the store handed us nothing (LoadFailed seeds this, §5).
pub(crate) fn default_board_layout() -> BoardLayout {
    BoardLayout {
        boards: vec![Board {
            id: BoardId::new(DEFAULT_BOARD_ID),
            name: DEFAULT_BOARD_NAME.into(),
            ordinal: 0,
            created_at: 0,
            updated_at: 0,
        }],
        items: vec![],
    }
}

impl BoardReplica {
    pub fn layout(&self) -> &BoardLayout {
        &self.layout
    }
    pub fn state(&self) -> ReplicaState {
        self.state
    }
    pub fn is_writable(&self) -> bool {
        state_is_writable(self.state)
    }
}
```

Register the module in `mod.rs`:

```rust
mod replica;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS. (Warnings about unused fields are expected until Task 4 — do not silence with `#[allow(dead_code)]`; the next task uses them.)

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs crates/lens-ui/src/board/mod.rs
git commit -m "feat(board): BoardReplica types + pure state helpers"
```

---

## Task 4: serialized `run_op` pump + `Load` (the core async path)

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline `#[cfg(test)]`, `#[gpui::test]`)

**Interfaces:**
- Consumes: Task 3 types; gpui `Context::spawn` (`|this: WeakEntity<Self>, cx: &mut AsyncApp| ...`), `background_executor().spawn`, `WeakEntity::update`.
- Produces (consumed by Tasks 5–7):
  - `BoardReplica::for_test(fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self` — tempfile-backed store, `conn = ConnectionId::new("conn_test")`, state `Loading`, enqueues `Load` (via `build`).
  - `fn build(...)` — the shared ctor funnel (see Test & construction conventions): installs the `FleetStore` observer + enqueues `Load`.
  - `fn run_op(&mut self, op: Op, cx: &mut Context<Self>)` — enqueue + `pump`.
  - `fn pump(&mut self, cx: &mut Context<Self>)` — single-in-flight spawn.
  - `fn apply_outcome(&mut self, outcome: OpOutcome, cx: &mut Context<Self>)` — main-thread apply.
  - `enum OpOutcome { Loaded { layout, skipped_empty }, Placed { layout, skipped_empty }, Failed { was_load: bool, err: String } }` (crate-internal; `String` keeps it `Send` and avoids leaking `PersistError`).

**Key shape (verbatim template):**

```rust
fn pump(&mut self, cx: &mut Context<Self>) {
    if self.in_flight {
        return;
    }
    let Some(op) = self.pending.pop_front() else {
        return;
    };
    self.in_flight = true;
    let store = Arc::clone(&self.store);
    let conn = self.conn.clone();
    cx.spawn(async move |this, cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move {
                let mut slot = store.lock().expect("board store mutex poisoned");
                run_op_blocking(&mut slot, &conn, op)
            })
            .await;
        this.update(cx, |this, cx| {
            this.apply_outcome(outcome, cx);
        })
        .ok(); // entity gone (window closed) → drop; nothing to apply.
    })
    .detach();
}
```

`run_op_blocking` (off-thread; opens-if-`None`, composes write+reload, drops the handle on `Err` so recovery reopens fresh):

```rust
fn run_op_blocking(slot: &mut StoreSlot, conn: &ConnectionId, op: Op) -> OpOutcome {
    let was_load = matches!(op, Op::Load);
    match run_op_inner(slot, conn, op) {
        Ok((layout, skipped_empty)) => {
            if was_load {
                OpOutcome::Loaded { layout, skipped_empty }
            } else {
                OpOutcome::Placed { layout, skipped_empty }
            }
        }
        Err(e) => {
            // Persistent (SQLITE_BUSY was absorbed by busy_timeout, Task 1) → drop the
            // handle so the next (recovery) Load reopens fresh from slot.path.
            slot.store = None;
            OpOutcome::Failed { was_load, err: e.to_string() }
        }
    }
}

fn run_op_inner(
    slot: &mut StoreSlot,
    _conn: &ConnectionId,
    op: Op,
) -> lens_core::persist::Result<(BoardLayout, bool)> {
    ensure_open(slot)?;
    let store = slot.store.as_ref().expect("ensure_open guarantees Some");
    match op {
        Op::Load => read_committed(store.as_ref()),
        Op::PlaceSessions(keys) => {
            store.place_sessions(&keys, &default_root_target())?;
            read_committed(store.as_ref()) // in-lock reload = committed layout (refinement #2)
        }
    }
}

fn ensure_open(slot: &mut StoreSlot) -> lens_core::persist::Result<()> {
    if slot.store.is_none() {
        // Reopen from the path — serves both first-open and recovery (§5). A bad
        // path (test/corruption) returns Err → LoadFailed/Stale upstream.
        slot.store = Some(Box::new(SqliteBoardStore::open(&slot.path)?));
    }
    Ok(())
}

fn read_committed(store: &dyn BoardStore) -> lens_core::persist::Result<(BoardLayout, bool)> {
    let loaded = store.load_layout()?;
    let skipped_empty = loaded.skipped.is_empty();
    let layout = loaded.rows.into_iter().next().unwrap_or_default();
    Ok((layout, skipped_empty))
}

fn default_root_target() -> PlacementTarget {
    PlacementTarget { board_id: None, parent_item_id: None, ordinal: None }
}
```

`apply_outcome` (Task 4 handles the success paths; Task 5 fills the `Failed` arm):

```rust
fn apply_outcome(&mut self, outcome: OpOutcome, cx: &mut Context<Self>) {
    self.in_flight = false;
    match outcome {
        OpOutcome::Loaded { layout, skipped_empty } => {
            self.layout = layout;
            self.state = state_after_load(skipped_empty);
            if self.is_writable() {
                self.reconcile(cx); // Task 6: initial/post-recovery reconcile
            }
        }
        OpOutcome::Placed { layout, skipped_empty } => {
            self.layout = layout;
            let _ = skipped_empty; // a place never introduces skips; keep Writable
            self.reconcile_in_flight = false; // Task 6
            self.reconcile(cx); // Task 6: re-diff on reply
        }
        OpOutcome::Failed { was_load, err } => {
            self.on_op_failed(was_load, err, cx); // Task 5
        }
    }
    cx.notify();
    self.pump(cx);
}
```

> For Task 4, stub `reconcile` as `fn reconcile(&mut self, _cx: &mut Context<Self>) {}` and `on_op_failed` as a minimal `self.state = ReplicaState::Stale;` — Tasks 5 and 6 replace them. This keeps Task 4 independently testable.

- [ ] **Step 1: Write the failing test**

```rust
#[gpui::test]
async fn load_op_populates_layout_and_becomes_writable(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::Writable);
        assert_eq!(r.layout().default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
    });
}

#[gpui::test]
async fn two_place_ops_apply_in_enqueue_order(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked(); // Load lands

    let c = ConnectionId::new("conn_test");
    replica.update(cx, |r, cx| {
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("a"))]), cx);
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("b"))]), cx);
    });
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        let n = r.layout().items.iter()
            .filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
        assert_eq!(n, 2, "both placements committed, in order, no out-of-order regress");
    });
}
```

> Use the real `FleetStore` fake constructor (search `fleet/store.rs` for the fake/test ctor; the survey shows `spawn_fake_session` + a `fake` field). If the fake ctor differs, mirror an existing `#[gpui::test]` in `acceptance_shell.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — `for_test`/`run_op` unresolved.

- [ ] **Step 3: Implement `build`, `for_test`, `run_op`, `pump`, the blocking runner, `apply_outcome`**

Add the templates above, plus:

Add `build` (the shared funnel from **Test & construction conventions**), plus `for_test` and `run_op`:

```rust
impl BoardReplica {
    pub fn for_test(fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("board.db");
        let store: Box<dyn BoardStore + Send> =
            Box::new(SqliteBoardStore::open(&path).expect("open test store"));
        Self::build(Some(store), path, ConnectionId::new("conn_test"), Some(dir), fleet, cx)
    }

    pub(crate) fn run_op(&mut self, op: Op, cx: &mut Context<Self>) {
        self.pending.push_back(op);
        self.pump(cx);
    }
}
```

Add the needed `use`s: `gpui::{Context, WeakEntity, prelude::*}`, `lens_core::domain::board::{BoardItemKind, PlacementTarget}`, `lens_core::persist::SqliteBoardStore`. Add `tempfile` as a **dev-dependency** of `lens-ui` if not already present (`for_test`/`for_demo` use it; `lens-core` already dev-deps it). Note `for_demo` (Task 7) also uses `tempfile` at runtime — if the demo path is compiled in non-test builds, `tempfile` must be a normal dep, not dev-only. Confirm which at Task 7.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS (both).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): serialized run_op pump + off-thread Load (single-in-flight)"
```

---

## Task 5: error states, recovery, write-failure contract

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces:**
- Produces:
  - `fn on_op_failed(&mut self, was_load: bool, err: String, cx: &mut Context<Self>)` — real body.
  - `fn write(&mut self, op: Op, cx: &mut Context<Self>)` — enqueues **iff** `is_writable()`, else no-op (state already surfaces it). Distinct from `run_op` so Task 6's reconcile and B-4b+ writes share one gate.
  - `fn retry_recovery(&mut self, cx: &mut Context<Self>)` — banner "Retry" / non-writable `FleetStore` notify path: enqueue a `Load` (always allowed).
  - Write-failure rule in `run_op`/`pump`: on transition to a non-writable state, **drop queued write ops** from `pending`; `pump` **re-gates** write ops on `is_writable()`.

**Behaviour to encode (§5):**
- `on_op_failed`: a failed **place** → `Stale`; a failed **load** → `LoadFailed` **and** seed `default_board_layout()` (so the board still renders, not a panic). Drop all queued `PlaceSessions` from `pending` (a persistent failure won't succeed on replay; never *silently* — the banner names it).
- `pump`: before spawning, if the popped op is a write (`PlaceSessions`) and `!is_writable()`, drop it and continue popping. A `Load` is always spawned.
- `retry_recovery`: `self.pending.push_back(Op::Load); self.pump(cx);` — allowed in any state.

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test]
async fn failed_load_seeds_default_board_and_marks_load_failed(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    // A File slot pointing at an un-openable path forces a Load failure.
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::LoadFailed);
        // renders, does not panic: default board present.
        assert_eq!(r.layout().default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
    });
}

#[gpui::test]
async fn non_writable_refuses_writes_but_accepts_recovery(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked(); // → LoadFailed

    // write() is a no-op while non-writable.
    replica.update(cx, |r, cx| {
        r.write(Op::PlaceSessions(vec![(r.conn.clone(), SessionId::new("x"))]), cx);
        assert!(r.pending.is_empty(), "write refused while non-writable");
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| assert_eq!(r.state(), ReplicaState::LoadFailed));
}
```

> `for_test_file(fleet, path, cx)` is a test ctor that funnels through `build` with `store: None` and no `TempDir`, so `ensure_open` tries the (bad) path and fails. Add it in this task.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — `for_test_file`/`write` unresolved; `LoadFailed` not set.

- [ ] **Step 3: Implement the error/recovery logic**

```rust
impl BoardReplica {
    pub(crate) fn for_test_file(
        fleet: Entity<FleetStore>,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        // store: None + a bad path → ensure_open's SqliteBoardStore::open fails → LoadFailed.
        Self::build(None, path, ConnectionId::new("conn_test"), None, fleet, cx)
    }

    pub(crate) fn write(&mut self, op: Op, cx: &mut Context<Self>) {
        if !self.is_writable() {
            return; // no-op; ReplicaState + banner already surface why.
        }
        self.run_op(op, cx);
    }

    pub(crate) fn retry_recovery(&mut self, cx: &mut Context<Self>) {
        self.banner_dismissed = false;
        self.run_op(Op::Load, cx); // Load is always allowed (recovery is a reopen-read).
    }

    fn on_op_failed(&mut self, was_load: bool, _err: String, cx: &mut Context<Self>) {
        self.reconcile_in_flight = false;
        self.banner_dismissed = false;
        if was_load {
            self.state = ReplicaState::LoadFailed;
            self.layout = default_board_layout(); // render an empty board, never panic.
        } else {
            self.state = ReplicaState::Stale;
        }
        // Persistent failure: queued writes won't succeed on replay. Drop them
        // (never silently — banner names the loss). Load ops are kept.
        self.pending.retain(|op| matches!(op, Op::Load));
        cx.notify();
    }
}
```

And re-gate in `pump` (replace the pop in Task 4's template):

```rust
    // Skip write ops that are no longer allowed (state flipped after they queued).
    let op = loop {
        match self.pending.pop_front() {
            None => return,
            Some(Op::PlaceSessions(_)) if !self.is_writable() => continue,
            Some(op) => break op,
        }
    };
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): ReplicaState error handling, recovery Load, write-failure drop+re-gate"
```

---

## Task 6: session-lifecycle reconcile (batched, conn-pinned, re-diff on reply)

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces:**
- Produces: real `fn reconcile(&mut self, cx)`, `fn missing_keys(&self, cx) -> Vec<(ConnectionId, SessionId)>`, and the `FleetStore` observer installed in the constructors.
- Consumes: `self.fleet.read(cx).cards` (`HashMap<SessionId, Entity<SessionCard>>`), `self.layout.items` (Card kind → `(conn, session)`).

**Behaviour (§3.3):**
- `missing_keys`: placed = `layout` Card items' `(conn, session)`; live = `fleet.cards` keys paired with `self.conn`; missing = live − placed.
- `reconcile`: no-op unless `is_writable()`; if `missing` empty → return; **coalesce** — if `reconcile_in_flight` return; else set `reconcile_in_flight`, `run_op(PlaceSessions(missing))`.
- Re-diff on reply: `apply_outcome`'s `Placed` arm already clears `reconcile_in_flight` and calls `reconcile` (Task 4 template) — this closes the coalesced-then-late-card gap. Tombstoned keys are no-op'd by `place_sessions` (Task 2), so a stale fleet key yields cheap repeated skips, not resurrection.
- Constructors install the observer: `cx.observe(&fleet, |this, _, cx| this.reconcile(cx)).detach()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test]
async fn fleet_session_gets_placed_and_persists(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();

    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("s1"), cx); });
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        let placed: Vec<_> = r.layout().items.iter().filter_map(|i| match &i.kind {
            BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
            _ => None,
        }).collect();
        assert_eq!(placed, vec!["s1".to_string()], "fleet session reconciled onto the board");
    });
}

#[gpui::test]
async fn coalesced_then_late_card_still_placed(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();

    // Two cards added back-to-back; the 2nd notify coalesces while the 1st place is in flight.
    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("a"), cx); });
    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("b"), cx); });
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        let n = r.layout().items.iter()
            .filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
        assert_eq!(n, 2, "reply-triggered re-diff caught the coalesced late card");
    });
    // settles with no further ops
    replica.read_with(cx, |r, _| assert!(!r.in_flight && r.pending.is_empty()));
}

#[gpui::test]
async fn double_reconcile_is_idempotent(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();
    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("s1"), cx); });
    cx.run_until_parked();
    replica.update(cx, |r, cx| r.reconcile(cx)); // manual second reconcile
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        let n = r.layout().items.iter()
            .filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
        assert_eq!(n, 1, "one row per session, no duplicate");
    });
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — `reconcile` is still the Task-4 stub (no placement happens).

- [ ] **Step 3: Implement reconcile + observer**

```rust
impl BoardReplica {
    pub(crate) fn missing_keys(&self, cx: &App) -> Vec<(ConnectionId, SessionId)> {
        use std::collections::HashSet;
        let placed: HashSet<(String, String)> = self
            .layout
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                BoardItemKind::Card { conn, session } => {
                    Some((conn.as_str().to_string(), session.as_str().to_string()))
                }
                _ => None,
            })
            .collect();
        self.fleet
            .read(cx)
            .cards
            .keys()
            .filter(|s| !placed.contains(&(self.conn.as_str().to_string(), s.as_str().to_string())))
            .map(|s| (self.conn.clone(), s.clone()))
            .collect()
    }

    pub(crate) fn reconcile(&mut self, cx: &mut Context<Self>) {
        if !self.is_writable() {
            return;
        }
        let missing = self.missing_keys(cx);
        if missing.is_empty() {
            return;
        }
        if self.reconcile_in_flight {
            return; // coalesce; the in-flight place's reply re-diffs.
        }
        self.reconcile_in_flight = true;
        self.run_op(Op::PlaceSessions(missing), cx);
    }
}
```

The `FleetStore` observer is **already installed by `build`** (Task 4), so no ctor edits here — this task only fills in `reconcile`/`missing_keys` (replacing the Task-4 stub). The observer calls `reconcile`, which no-ops until `Writable`, so early notifies during `Loading` are harmless; the post-`Load` reconcile (`apply_outcome` `Loaded` arm) catches the current fleet snapshot regardless of subscription timing.

> `missing_keys` takes `&App` (a read context). Inside `reconcile` (which has `&mut Context<Self>`), pass `cx` — `Context<Self>` derefs to `App` for reads. If the borrow checker objects, snapshot the keys first: `let live: Vec<SessionId> = self.fleet.read(cx).cards.keys().cloned().collect();` then diff against `placed` without holding the `fleet.read` borrow.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): batched conn-pinned reconcile with re-diff-on-reply"
```

---

## Task 7: production `new` + demo seeding

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs` (add `new`, `for_demo`)
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces:**
- Produces:
  - `BoardReplica::new(store: Option<Box<dyn BoardStore + Send>>, path: PathBuf, conn: ConnectionId, fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self` — prod path. `store` is the bootstrap-opened handle (Task 8), or `None` if that open failed; `path` lets `ensure_open`/recovery (re)open. `None` + a bad path → `LoadFailed` with the **real** `conn` (not a test ctor).
  - `BoardReplica::for_demo(fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self` — tempfile-backed store + `conn_demo`; **seeds a group with members at construction** (before the first reconcile, so members aren't re-placed loose) via synchronous `create_group` + `place_session` on the store, and the demo caller also spawns those members as fake fleet sessions.

**Why seed synchronously:** the demo store is constructed right here, so a direct pre-`Load` write is deterministic and needs no round-trip; the subsequent `Load` reads the seeded group, rendering B-3 group chrome via the real path for the first time.

- [ ] **Step 1: Write the failing test**

```rust
#[gpui::test]
async fn demo_seeds_a_group_rendered_via_real_path(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_demo(fleet.clone(), cx)));
    cx.run_until_parked();

    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::Writable);
        let groups = r.layout().items.iter()
            .filter(|i| matches!(i.kind, BoardItemKind::Group { .. })).count();
        assert_eq!(groups, 1, "demo seeds exactly one group, loaded via the store");
    });
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib board::replica::demo_seeds`
Expected: FAIL — `for_demo` unresolved.

- [ ] **Step 3: Implement `new` + `for_demo`**

```rust
impl BoardReplica {
    pub fn new(
        store: Option<Box<dyn BoardStore + Send>>,
        path: PathBuf,
        conn: ConnectionId,
        fleet: Entity<FleetStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(store, path, conn, None, fleet, cx)
    }

    pub fn for_demo(fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self {
        let dir = tempfile::tempdir().expect("demo tempdir");
        let path = dir.path().join("board.db");
        let store = SqliteBoardStore::open(&path).expect("demo store"); // seeds default board
        let conn = ConnectionId::new("conn_demo");
        let board = BoardId::new(DEFAULT_BOARD_ID);
        // Seed one group + two members BEFORE the first Load, so reconcile sees them placed.
        let group = store.create_group(&board, None, 0, "Demo group").expect("seed group");
        let members = [SessionId::new("demo_a"), SessionId::new("demo_b")];
        for (i, s) in members.iter().enumerate() {
            store
                .place_session(
                    &conn,
                    s,
                    &PlacementTarget {
                        board_id: Some(board.clone()),
                        parent_item_id: Some(group.clone()),
                        ordinal: Some(i as i32),
                    },
                )
                .expect("seed member");
        }
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        Self::build(Some(boxed), path, conn, Some(dir), fleet, cx)
    }
}
```

> A fresh `SqliteBoardStore::open` already seeds the default board (test `fresh_open_seeds_default_board`, `board.rs`), so `create_group` against `DEFAULT_BOARD_ID` works with no extra step.
> The demo **caller** (Task 8, `main.rs` demo branch) must also `fleet.spawn_fake_session(SessionId::new("demo_a"/"demo_b"), cx)` so their `SessionCard` entities exist and the group's member cards animate. They're already Card items under the group, so reconcile won't re-place them loose. `create_group(board_id, parent_item_id, ordinal, name) -> BoardItemId` per `board.rs:489`.
> **Dep note:** `for_demo` runs in non-test builds, so `tempfile` must be a **normal** dependency of the crate that hosts it (not dev-only). If keeping `tempfile` out of the prod dep tree matters, host `for_demo` behind a `demo` cfg/feature, or point the demo at a real temp path via `std::env::temp_dir()` instead of `tempfile`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-ui --lib board::replica::demo_seeds`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): prod new() + demo group seeding via the real store path"
```

---

## Task 8: wire the read path into `BoardView` + app bootstrap

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (fields, `mount`, `pack_and_render`, observers; retire `test_layout`)
- Delete: `crates/lens-ui/src/board/layout_adapter.rs`
- Modify: `crates/lens-app/src/main.rs` (bootstrap open before actors; construct + pass replica; demo branch)
- Modify: `crates/lens-ui/tests/acceptance_shell.rs` (5 mount call sites)
- Test: migrated B-3 group fixture test in `board/mod.rs`

**Interfaces:**
- `BoardView::mount(fleet, replica: Entity<BoardReplica>, working_tab, pty_probe, cx)` — new `replica` param.
- `pack_and_render` reads `self.replica.read(cx).layout().clone()` (replaces `test_layout`/`build_ephemeral_layout`); the `default_board_id() == Err` guard (`mod.rs:230-233`) stays as the render-safety net.
- `BoardView` observes the replica (layout/banner changes → `cx.notify()`), in addition to the existing `FleetStore` observe (membership/focus, `mod.rs:110`).

- [ ] **Step 1: Migrate the B-3 group fixture test (write it against the new path)**

Replace the `test_layout`-injecting B-3 fixture test with one that seeds a group into a replica and asserts the group chrome renders through the real path. Model it on the existing fixture (search `mod.rs` tests for `test_layout` / `group_chrome_for_test`):

```rust
#[gpui::test]
async fn group_chrome_renders_via_replica(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_demo(fleet.clone(), cx)));
    cx.run_until_parked();
    let (board, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx)
    });
    // assert group chrome present via the existing seam (group_chrome_for_test or paint bounds)
    board.read_with(&vcx, |b, _| {
        assert!(b.group_chrome_for_test().len() >= 1, "group renders through the real store path");
    });
}
```

Run: `cargo test -p lens-ui --test acceptance_shell group_chrome_renders_via_replica` (or `--lib` if the fixture lives in `mod.rs`).
Expected: FAIL — `mount` arity mismatch / `test_layout` gone.

- [ ] **Step 2: Rewire `BoardView`**

- Add field `replica: Entity<BoardReplica>`; **remove** `test_layout: Option<BoardLayout>`.
- `mount` gains `replica: Entity<BoardReplica>`, stores it, and observes it:

```rust
    pub fn mount(
        fleet: Entity<FleetStore>,
        replica: Entity<BoardReplica>,
        working_tab: TabHandle,
        pty_probe: Option<PtyProbe>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&replica, |_board: &mut BoardView, _, cx| cx.notify()).detach();
        // ... existing fleet observe stays ...
```

- `pack_and_render`:

```rust
        let layout = self.replica.read(cx).layout().clone();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
```

- Delete `use layout_adapter::build_ephemeral_layout;` and the `mod layout_adapter;` line; `rm crates/lens-ui/src/board/layout_adapter.rs`.

- [ ] **Step 3: Update the app + demo call sites (`main.rs`)**

Bootstrap-open the board store in `open_stores` (or alongside it), **before** the session actors spawn, and thread it to the window so `mount` can build the replica:

```rust
    // Board store: opened at bootstrap (before actors) to minimize SQLITE_BUSY (§6).
    // Non-fatal: on open failure, pass source with a None store so the replica
    // starts LoadFailed and can recover via reopen-Load.
    let board_db = data_dir.join("lens.db");
    let mut board_store_for_window: Option<Box<dyn BoardStore + Send>> =
        SqliteBoardStore::open(&board_db).ok().map(|s| Box::new(s) as _);
```

`board_store_for_window` is `mut` because the window closure `.take()`s it. Only one window path (live vs demo) runs per launch, so the single `Option` has a single consumer.

At each `BoardView::mount` site (`main.rs:110`, `165`), construct the replica first:

```rust
                let replica = cx.new(|cx| {
                    BoardReplica::new(
                        board_store_for_window.take(), // Option<Box<..>>: None if bootstrap open failed
                        board_db.clone(),
                        conn_id.clone(), // "lens-app" (§4)
                        fleet.clone(),
                        cx,
                    )
                });
                let board = cx.new(|cx| {
                    BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx)
                });
```

> Threading detail: `board_store` is a single `Box`, but there are two mount sites (live vs demo/placeholder window paths). Only one window path runs per launch; clone the `board_db` path and move the `Box` into whichever branch executes. For the **demo** branch, use `BoardReplica::for_demo(fleet.clone(), cx)` and `fleet.spawn_fake_session(SessionId::new("demo_a"/"demo_b"), cx)` (Task 7 note). Pin `conn = "lens-app"` in prod (`conn_id`, `main.rs:306`).

- [ ] **Step 4: Update the 5 acceptance-test call sites**

At `acceptance_shell.rs:81,194,292,643,748`, build a replica before each `mount`:

```rust
        let replica = cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx));
        let (board_handle, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet_for_window, replica.clone(), placeholder_tab(cx), None, cx)
        });
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p lens-ui`
Expected: PASS — migrated fixture + all acceptance tests. Run the app once to eyeball (Task 10 covers perf): `cargo run -p lens-app` (or the demo entry) and confirm the board renders + the demo group shows.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-app/src/main.rs crates/lens-ui/tests/acceptance_shell.rs
git rm crates/lens-ui/src/board/layout_adapter.rs
git commit -m "feat(board): BoardView reads BoardReplica; retire ephemeral stub + test_layout seam"
```

---

## Task 9: non-blocking error banner

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (render a banner from `replica.state()`)
- Test: `crates/lens-ui/src/board/mod.rs` (inline)

**Interfaces:**
- Consumes: `replica.read(cx).state()`, `replica.read(cx).banner_dismissed` (add a `banner_dismissed(&self) -> bool` getter + `dismiss_banner`/`retry` methods on `BoardReplica`).
- Produces: a small non-modal notice over the board area with copy per `ReplicaState`, a **Retry** button (→ `replica.update(cx, |r, cx| r.retry_recovery(cx))`), and a **Dismiss** (→ `r.dismiss_banner()`).

**Copy (verbatim, §5):**
- `Degraded`: "Some board items couldn't be read — changes won't save."
- `LoadFailed`: "Couldn't load your board — data on disk is untouched."
- `Stale`: "Couldn't save your last change — reconnecting."
- `Loading`/`Writable`: no banner.

- [ ] **Step 1: Write the failing test**

```rust
#[gpui::test]
async fn banner_shows_for_load_failed(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked();
    let (board, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx)
    });
    board.read_with(&vcx, |b, cx| {
        assert!(b.banner_text(cx).is_some(), "LoadFailed surfaces a banner");
    });
}
```

> Add a small `BoardView::banner_text(&self, cx) -> Option<&'static str>` that maps `replica.state()` + `!banner_dismissed` → the copy above; render it when `Some`. Test the mapping (unit) even if asserting painted pixels is out of scope per [[gpui-test-noop-text-system]].

- [ ] **Step 2–4: Red → implement `banner_text` + the notice element + Retry/Dismiss wiring → green**

Run: `cargo test -p lens-ui --lib banner_shows_for_load_failed`
Expected: FAIL then PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): non-blocking ReplicaState banner with Retry/Dismiss"
```

---

## Task 10: perf — three distinct measures + gate

**Files:**
- Create: `crates/lens-core/benches/board_pack.rs`
- Modify: `crates/lens-core/Cargo.toml` (`[[bench]]`)
- Modify: `spikes/board-container/` (seed a group at scale in the container)
- Modify: `xtask` gate list if a new production crate/bench needs inclusion (it doesn't — bench builds under `-p lens-core`).

**Measures (§7 — the E2E is prior-slice B-2/B-3 debt paid down here, sized as a real task):**

- [ ] **Step 1: `lens-core` pack/`board_tree` criterion bench (supporting, gate-automatable)**

```rust
// crates/lens-core/benches/board_pack.rs
use criterion::{criterion_group, criterion_main, Criterion};
use lens_core::domain::board::{BoardLayout, /* build a layout of N cards incl. one group */};

fn bench_board_tree(c: &mut Criterion) {
    let layout = build_layout_with_group(1000); // helper: 1000 cards + one group
    let board = layout.default_board_id().unwrap().clone();
    c.bench_function("board_tree_1000_with_group", |b| {
        b.iter(|| {
            let nodes = layout.board_tree(&board).unwrap();
            criterion::black_box(nodes.len());
        })
    });
}
criterion_group!(benches, bench_board_tree);
criterion_main!(benches);
```

```toml
[[bench]]
name = "board_pack"
harness = false
```

Run: `cargo bench -p lens-core --bench board_pack` — record the baseline in the commit message (matches the `persist_throughput`/`reduce_throughput` convention).

- [ ] **Step 2: Frame-budget E2E on-device (MANDATORY)**

Extend `spikes/board-container` so its `Container` seeds **N items including one group** (parameterize `REPEATS`/add a group tile), and run `spikes/board-container/measure.sh` at realistic (~100) and stress (~1000+) N. Hold **120fps / 8.3ms** target, flag **90fps / 11.1ms** regression. This is the first at-scale exercise of B-3 group render-time member reads — record FPS/CPU (cull ON vs `--all-timers`) in the commit body. Per [[wave-perf-fps-attribution]], CPU is per-frame full-tree re-render, not paint — sample accordingly.

- [ ] **Step 3: Op-latency (off-frame, supporting)**

A `#[gpui::test]` (or a small bench) that times `Load` + a batched `PlaceSessions` of N via `run_until_parked` wall-clock — confirm sessions appear promptly and the mutex isn't held excessively. Explicitly **not** a frame-budget assertion; log the numbers, don't gate on them.

- [ ] **Step 4: Full gate**

Run: `cargo xtask gate`
Expected: green — zero warnings, `cargo fmt --check`, all tests, benches build. Do not pipe through `tail`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/benches/board_pack.rs crates/lens-core/Cargo.toml spikes/board-container/
git commit -m "perf(board): pack bench + at-scale group render E2E (B-2/B-3 debt) + op-latency"
```

---

## Self-review checklist (run before handing off)

- **Spec coverage:** §1 scope → Tasks 3–9; §2 components → Tasks 3–7; §3 read/write/reconcile → Tasks 4/6/8; §4 pinned conn → Global Constraints + Task 8; §5 error/banner/recovery → Tasks 5/9; §6 construction order → Tasks 7/8; §7 testing+perf → every task's tests + Task 10; §8 seams → not built (correct). Deferred (B-4b/c/d, B-5, B-6) → untouched.
- **Type consistency:** `place_sessions(&[(ConnectionId, SessionId)], &PlacementTarget)` (Tasks 2/4/6); `Op`/`ReplicaState`/`StoreSlot` names stable across Tasks 3–8; `mount(fleet, replica, working_tab, pty_probe, cx)` used identically in Task 8's app + test sites.
- **Placeholder scan:** the store/fleet ctors are now verified against source (§ Test & construction conventions): tempfile-backed `SqliteBoardStore::open`, `FleetStore::new(clock, cx)` with `ManualUiClock`. The one remaining "mirror the sibling test" pointer is the `tombstone_session` test helper (Task 2) — a test-setup detail, not production code.
- **Review diversity (MANDATORY):** after the code lands, one cross-family review (codex/gpt-5.6 per [[review-spend-policy]]) of the whole B-4a diff, including a gate-runner ([[whole-branch-review-needs-a-builder]]).
