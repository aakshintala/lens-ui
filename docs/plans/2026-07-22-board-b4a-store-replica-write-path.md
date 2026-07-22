# Board B-4a — store→replica write-path foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ephemeral `build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced off-thread from `SqliteBoardStore` behind a main-thread in-memory replica, so the board renders from the real store and survives restarts — shipping **no** user interactions.

**Architecture:** A `BoardReplica` gpui entity owns an in-memory `BoardLayout` that every render reads for free (no I/O on the frame path). All SQLite access runs off-thread through a **serialized single-in-flight `run_op`** pump (`cx.spawn` → `background_executor().spawn` → `WeakEntity::update`). Ops (`Load`, `PlaceSessions`) apply in enqueue order, so the in-memory layout is the latest committed state at quiescence. A `FleetStore` observer drives an additive, conn-pinned, batched reconcile (and, when non-writable, a coalesced recovery). `ReplicaState` gates writes, classifies SQLite errors (transient → bounded retry, persistent → `Stale`/drop), and drives a non-blocking banner.

**Tech Stack:** Rust, gpui 0.2.2, rusqlite, `lens-core` (`BoardStore`/`SqliteBoardStore`/`BoardLayout`), `lens-ui` (`FleetStore`/`BoardView`), criterion.

**Design doc (authority):** `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` (LOCKED, residual pass 2026-07-22). Section refs (§N) below point into it.

**Review pedigree:** authored from the LOCKED design; signatures source-verified; then codex (gpt-5.6) adversarially reviewed this plan vs the design and real source. This is **v2**, with all confirmed findings folded (see § Codex review dispositions).

## Global Constraints

_Every task's requirements implicitly include this section._

- **Off-thread I/O (MANDATORY, AGENTS.md:19 / `.agents/rust-ui.md`):** all disk I/O runs inside a background task; the UI thread only does `cx.update`/`cx.notify`/`entity.update`. The **only** synchronous `SqliteBoardStore` calls allowed are at bootstrap, **before `Application::run` starts a frame loop** — the prod store open (Task 8) and the demo seed (Task 8). Everything after is off-thread.
- **Never panic in the UI (AGENTS.md):** every store failure is non-fatal → a `ReplicaState` transition + banner. **No `unwrap`/`expect` on a store `Result` in the async path** — a background panic never resolves its `Task`, wedging the single-in-flight pump forever (codex M10). Mutex poisoning is recovered (`.unwrap_or_else(|p| p.into_inner())`), not `expect`ed.
- **Frame budget (MANDATORY):** 120fps / 8.3ms target, 90fps / 11.1ms regression line — asserted E2E on the **real `BoardView`** path (Task 10), not a synthetic spike.
- **Pinned connection (PROVISIONAL → B-5):** `BoardReplica.conn` = `ConnectionId::new("lens-app")` in prod, a fixed id in demo/test. This is what makes the two placement sources converge (§3.3/§4).
- **Single-in-flight ordering is load-bearing** (§2): exactly one op outstanding; replies apply in enqueue order. Required for correctness *and* deterministic tests.
- **Gate:** `cargo xtask gate` green — zero warnings, `cargo fmt --check`, all tests, benches build. Scope new crates into the gate's explicit `-p` list; never pipe the gate through `tail` ([[xtask-gate-scope]], [[terminal-spikes-process-learnings]]).
- **Commit** after each task's tests pass. Solo workflow: work on `main` is fine ([[integration-workflow]]); do **not** auto-push.

## Codex review dispositions (folded into v2)

- **C1 tombstone reconcile loop (fixed):** `FleetStore.cards` never drops tombstoned sessions (verified — no `cards.remove` in `fleet/store.rs`), and `place_sessions` skips tombstoned keys, so re-diff-on-reply would re-enqueue the same key forever. Fix: a replica-side `suppressed` set — an attempted key still missing after its place is marked stuck and excluded from future diffs (Task 6). Self-terminating.
- **C2 load health (fixed):** outcomes carry `StoreMode` + skipped status; `ReadOnlyDegraded` → `Degraded` even on a clean read. `Load` is tagged `initial`; a failed **recovery** load preserves the prior layout/state (never blanks visible data), only an **initial** failure seeds the empty default (Task 4/5).
- **C3 compile fixes:** `HashSet<BoardId>` (BoardId is `Hash`, not `Ord`); `pub use replica::{BoardReplica, ReplicaState, WriteDisposition}`; `BoardView::new` (mod.rs:131) updated for the new `mount` arity; `tempfile` a **normal** dep of `lens-ui`.
- **C4 demo (fixed):** demo store opened+seeded **before `Application::run`** (compliant, no `cx.new` SQLite, no `expect`); demo cards go straight into `fleet.cards` as `run_demo` already does — `spawn_fake_session` is never called (it'd panic under `new_live`).
- **M5 rebutted (kept compose-reload):** codex proposed `place_sessions` return its in-memory layout to avoid a second read. **Rejected** — `load_layout` runs `reconcile_sessions` (board.rs:199): read-time lazy-placement + tombstone-pruning that the in-memory layout lacks. The `PlaceSessions` op therefore persists then `load_layout`s **under one lock**; a post-commit read failure → `Stale` (data safe, recovery re-reads). Correct over "no second read."
- **M6 (fixed):** `busy_timeout` moved to immediately after `Connection::open`; **typed retry** at the replica for `DatabaseBusy`/`DatabaseLocked` (bounded backoff, op kept queued) vs persistent errors (Stale/drop). `PersistError` carried typed through outcomes, never stringified.
- **M7 (fixed):** recovery is coalesced (`recovery_in_flight`); the fleet observer triggers a recovery `Load` when non-writable, `reconcile` when writable.
- **M8 (fixed):** `write() -> WriteDisposition`; rejection re-surfaces the banner; dropped-write count feeds honest banner copy. Loading-time interaction gating is a B-4c caller note.
- **M9 (claim corrected):** the batch does **one persist** (the disk-dominant cost) vs `k`; in-memory manipulation stays O(k·N) (linear `find_card`/`item` scans). Acceptable at reconcile scale; a domain index-batch is a deferred optimization pending the Task 10 bench.
- **M10 (fixed):** no `expect`/`unwrap` in the async path; poison recovered; every spawned op produces a terminal `apply_outcome`.
- **M11 (fixed):** the mandatory E2E measures `lens-app --demo` (real `BoardView` + real group chrome + replica reads) at `LENS_DEMO_N` scale, not `spikes/board-container`.
- **Minor 12 (fixed):** a controllable `BoardStore` test double with a **blocking barrier** (channel, not a busy-spin — [[worker-stall-gate-busy-spin-flake]]) exercises in-flight coalescing and injected write/read failures.

---

## Test & construction conventions (verified against source — read before Task 3)

The repo has **no in-memory SQLite path** (`open_db` does `create_dir_all(path.parent())`, `db.rs:37`), and `FleetStore::fake` does not exist. Verified real primitives:

- **Store for tests/demo = a real file in a `tempfile::TempDir`** (`board.rs` test mod uses `tempfile::tempdir()` + `SqliteBoardStore::open(dir.join("lens.db"))`). The slot is `StoreSlot { path: PathBuf, store: Option<Box<dyn BoardStore + Send>> }`; the replica holds `_tempdir: Option<tempfile::TempDir>` to keep the test/demo file alive (None in prod). `ensure_open` reopens from `slot.path` — this is also how recovery (§5) reopens. **`tempfile` is a normal dependency of `lens-ui`** (used by `pub fn for_test`, callable from the `acceptance_shell.rs` integration test, so not `#[cfg(test)]`).
- **Fleet for tests = `FleetStore::new(clock, cx)`** — the *fake* ctor (`store.rs:28`, sets `fake: Some(FakeFleet::new())`, returns `Entity<Self>`). `new_live` (`:42`) is prod/demo. Clock = `ManualUiClock::new(10_000)` as `Arc<dyn UiClock>` (`clock.rs`; used verbatim in `acceptance_shell.rs:59,64`).
- **Shared test helper** (define once in the `replica.rs` `#[cfg(test)]` mod; every `#[gpui::test]` below calls it):

```rust
#[cfg(test)]
fn test_fleet(cx: &mut gpui::App) -> Entity<FleetStore> {
    use std::sync::Arc;
    use crate::clock::{ManualUiClock, UiClock};
    FleetStore::new(Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>, cx)
}
```

- **All ctors funnel through a private `build`** (installs the `FleetStore` observer + enqueues the first `Load { initial: true }`), so the observer is wired in exactly one place:

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
            recovery_in_flight: false,
            op_retries: 0,
            suppressed: HashSet::new(),
            last_attempt: Vec::new(),
            dropped_writes: 0,
            banner_dismissed: false,
            _tempdir: tempdir,
        };
        cx.observe(&this.fleet.clone(), |this: &mut Self, _f, cx| this.on_fleet_change(cx)).detach();
        this.run_op(Op::Load { initial: true }, cx);
        this
    }
}
```

---

## File structure

- `crates/lens-core/src/persist/db.rs` — **modify**: `busy_timeout` PRAGMA right after `Connection::open` (Task 1).
- `crates/lens-core/src/persist/board.rs` — **modify**: `place_sessions` batch trait method + impl (Task 2).
- `crates/lens-core/benches/board_pack.rs` — **create**: criterion bench for `board_tree` (Task 10).
- `crates/lens-ui/src/board/replica.rs` — **create**: `BoardReplica`, `Op`, `ReplicaState`, `WriteDisposition`, `StoreSlot`, the pump, reconcile, error/retry/recovery (Tasks 3–7).
- `crates/lens-ui/src/board/mod.rs` — **modify**: `BoardView` reads the replica; retire `test_layout`; observe replica; banner; `mount`/`new` gain `replica`; `pub use replica::…` (Tasks 8, 9).
- `crates/lens-ui/src/board/layout_adapter.rs` — **delete** at Task 8.
- `crates/lens-ui/Cargo.toml` — **modify**: `tempfile` as a normal dep (Task 3).
- `crates/lens-app/src/main.rs` — **modify**: bootstrap-open before actors; construct/pass `BoardReplica`; demo seed before `Application::run` (Tasks 8, 10).
- `crates/lens-ui/tests/acceptance_shell.rs` — **modify**: 5 `mount` call sites gain a replica (Task 8).

---

## Task 1: `busy_timeout` on the shared connection

**Files:**
- Modify: `crates/lens-core/src/persist/db.rs:40`
- Test: `crates/lens-core/src/persist/db.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `open_db(path, ddl, version) -> Result<(Connection, StoreMode)>`.
- Produces: the returned `Connection` has `busy_timeout = 5000` ms, set **before** the first `meta` write/read (both lock-sensitive), covering open-time queries and every later write txn. Absorbs sub-5s `SQLITE_BUSY`; the replica's typed retry (Task 5) handles the rare >5s case.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn open_db_sets_busy_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, _mode) = open_db(&dir.path().join("t.db"), CONTROL_DDL, SCHEMA_VERSION).unwrap();
    let ms: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap();
    assert_eq!(ms, 5000);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-core --lib open_db_sets_busy_timeout`
Expected: FAIL — left `0`, right `5000`.

- [ ] **Step 3: Set the PRAGMA immediately after open**

Insert directly after `let conn = Connection::open(path)?;` (db.rs:40), **before** `CREATE TABLE IF NOT EXISTS meta` and `read_schema_version` (both take locks):

```rust
    let conn = Connection::open(path)?;
    // busy_timeout must precede the first lock-sensitive statement (meta create /
    // version read) and covers every later write txn on this connection. Absorbs
    // sub-5s SQLITE_BUSY from the ~dozen SqliteControlStore connections on lens.db
    // (design §5/§6); the replica's typed retry (Task 5) covers the rare >5s case.
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);")?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-core --lib open_db_sets_busy_timeout`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/persist/db.rs
git commit -m "feat(persist): busy_timeout set before first lock-sensitive statement"
```

---

## Task 2: `place_sessions` batch method (one persist)

**Files:**
- Modify: `crates/lens-core/src/persist/board.rs` (trait `:16-57`, impl near `:430-469`)
- Test: `crates/lens-core/src/persist/board.rs` (inline)

**Interfaces:**
- Consumes: `guard_write`, `is_tombstoned`, `load_layout_inner`, `ensure_default_board`, `load_boards`, `new_item_id`, `now_ms`, `persist_board_items`, `touch_board`, `self.conn.unchecked_transaction`; domain `BoardLayout::place_session`/`find_card`/`item`.
- Produces: `fn place_sessions(&self, placements: &[(ConnectionId, SessionId)], target: &PlacementTarget) -> Result<()>` on `BoardStore` — places each non-tombstoned, not-already-present session; persists **each touched board once** inside **one** transaction. Return `()` (the replica re-reads via `load_layout` for the reconciled view — see § M5 disposition). Tombstoned/duplicate entries are silently skipped (matching `place_session`).

**Cost note (M9):** the batch collapses `k` whole-board persists to one (the disk-dominant cost). In-memory it is still O(k·N) (`find_card`/`item` are linear scans). Fine at reconcile scale; a domain index-batch is deferred pending Task 10's bench.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn place_sessions_batch_places_all_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
    let conn = ConnectionId::new("c1");
    let target = PlacementTarget { board_id: None, parent_item_id: None, ordinal: None };
    store.place_sessions(
        &[(conn.clone(), SessionId::new("s1")),
          (conn.clone(), SessionId::new("s2")),
          (conn.clone(), SessionId::new("s3"))],
        &target,
    ).unwrap();

    let layout = store.load_layout().unwrap().rows.into_iter().next().unwrap();
    let cards: Vec<_> = layout.items.iter().filter_map(|i| match &i.kind {
        BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
        _ => None,
    }).collect();
    assert_eq!(cards.len(), 3);
    assert!(["s1", "s2", "s3"].iter().all(|s| cards.iter().any(|c| c == s)));
}

#[test]
fn place_sessions_skips_tombstoned_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
    let conn = ConnectionId::new("c1");
    let target = PlacementTarget { board_id: None, parent_item_id: None, ordinal: None };
    store.place_session(&conn, &SessionId::new("s1"), &target).unwrap();
    tombstone_session(&store, &conn, &SessionId::new("s2")); // helper: see note

    store.place_sessions(
        &[(conn.clone(), SessionId::new("s1")),   // dup → skip
          (conn.clone(), SessionId::new("s2")),   // tombstoned → skip
          (conn.clone(), SessionId::new("s3"))],  // new → place
        &target,
    ).unwrap();

    let layout = store.load_layout().unwrap().rows.into_iter().next().unwrap();
    let n = layout.items.iter().filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
    assert_eq!(n, 2);
}
```

> **Note on `tombstone_session`:** mirror the sibling test `place_tombstoned_session_is_noop` (`board.rs:1078`) for how a row's `sessions.tombstoned_at` is set — do not invent a new harness.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core --lib place_sessions`
Expected: FAIL — `no method named place_sessions`.

- [ ] **Step 3: Add the trait method**

After `place_session` in the `BoardStore` trait (`:37`):

```rust
    /// Batch placement (§3.3): place each non-tombstoned, not-already-present session,
    /// persisting each touched board ONCE inside ONE transaction (one persist vs k).
    /// Tombstoned/duplicate entries are skipped. Callers re-read via `load_layout` for
    /// the reconciled view (read-time lazy-place + tombstone-prune).
    fn place_sessions(
        &self,
        placements: &[(ConnectionId, SessionId)],
        target: &PlacementTarget,
    ) -> Result<()>;
```

- [ ] **Step 4: Implement on `SqliteBoardStore`**

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
        let mut touched: std::collections::HashSet<BoardId> = std::collections::HashSet::new();
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

> `HashSet<BoardId>` (BoardId derives `Hash`, not `Ord`). The `.expect("just inserted")` mirrors the existing `place_session` (board.rs:462) — a synchronous, logically-unreachable invariant in `lens-core`, not the async pump. If a test-fake `BoardStore` impl exists, add a `place_sessions` there (a loop over `place_session` is fine for a fake).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-core --lib place_sessions`
Expected: PASS (both).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-core/src/persist/board.rs
git commit -m "feat(persist): batch place_sessions (one txn, one persist per board)"
```

---

## Task 3: `BoardReplica` types + pure helpers (no async yet)

**Files:**
- Create: `crates/lens-ui/src/board/replica.rs`
- Modify: `crates/lens-ui/src/board/mod.rs` (`pub mod replica;` + `pub use replica::{BoardReplica, ReplicaState, WriteDisposition};`)
- Modify: `crates/lens-ui/Cargo.toml` (`tempfile` normal dep)
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces (canonical — every later task uses these exact names):**

```rust
pub(crate) enum Op { Load { initial: bool }, PlaceSessions(Vec<(ConnectionId, SessionId)>) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaState { Loading, Writable, Degraded, LoadFailed, Stale }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteDisposition { Accepted, Rejected(ReplicaState) }

pub(crate) struct StoreSlot { pub(crate) path: PathBuf, pub(crate) store: Option<Box<dyn BoardStore + Send>> }

pub struct BoardReplica {
    pub(crate) store: Arc<Mutex<StoreSlot>>,
    pub(crate) conn: ConnectionId,
    pub(crate) layout: BoardLayout,
    pub(crate) state: ReplicaState,
    pub(crate) fleet: Entity<FleetStore>,
    pub(crate) in_flight: bool,
    pub(crate) pending: VecDeque<Op>,
    pub(crate) reconcile_in_flight: bool,
    pub(crate) recovery_in_flight: bool,
    pub(crate) op_retries: u32,
    pub(crate) suppressed: HashSet<(String, String)>,       // (conn,session) tombstoned/stuck (C1)
    pub(crate) last_attempt: Vec<(ConnectionId, SessionId)>, // keys of the in-flight PlaceSessions
    pub(crate) dropped_writes: u32,                          // banner honesty (M8)
    pub(crate) banner_dismissed: bool,
    pub(crate) _tempdir: Option<tempfile::TempDir>,          // keeps test/demo file alive; None in prod
}
```

Pure helpers (produced here, used everywhere):
- `state_is_writable(ReplicaState) -> bool` and method `is_writable(&self) -> bool`.
- `load_state(mode: StoreMode, skipped_empty: bool) -> ReplicaState`.
- `is_transient(err: &PersistError) -> bool`.
- `default_board_layout() -> BoardLayout`.
- `const MAX_RETRIES: u32 = 5;`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn default_board_layout_has_a_default_board() {
    let l = default_board_layout();
    assert_eq!(l.default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
    assert!(l.items.is_empty());
}

#[test]
fn load_state_maps_mode_and_skips() {
    assert_eq!(load_state(StoreMode::ReadWrite, true), ReplicaState::Writable);
    assert_eq!(load_state(StoreMode::ReadWrite, false), ReplicaState::Degraded); // skipped rows
    assert_eq!(load_state(StoreMode::ReadOnlyDegraded, true), ReplicaState::Degraded); // future schema
}

#[test]
fn is_transient_only_for_busy_or_locked() {
    let busy = PersistError::Sqlite(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error { code: rusqlite::ErrorCode::DatabaseBusy, extended_code: 5 },
        None,
    ));
    assert!(is_transient(&busy));
    assert!(!is_transient(&PersistError::ReadOnly));
}

#[test]
fn is_writable_only_in_writable_state() {
    assert!(state_is_writable(ReplicaState::Writable));
    for s in [ReplicaState::Loading, ReplicaState::Degraded, ReplicaState::LoadFailed, ReplicaState::Stale] {
        assert!(!state_is_writable(s));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — unresolved names.

- [ ] **Step 3: Write the types + pure helpers**

```rust
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{Context, Entity, prelude::*};
use lens_core::domain::board::{
    Board, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME, PlacementTarget,
};
use lens_core::domain::ids::{BoardId, ConnectionId, SessionId};
use lens_core::persist::{BoardStore, PersistError, SqliteBoardStore, StoreMode};

use crate::fleet::store::FleetStore;

pub(crate) const MAX_RETRIES: u32 = 5;

// ... Op / ReplicaState / WriteDisposition / StoreSlot / BoardReplica as in Interfaces ...

pub(crate) fn state_is_writable(s: ReplicaState) -> bool {
    matches!(s, ReplicaState::Writable)
}

/// Read succeeded; degrade on a future-schema store OR any skipped (corrupt) rows.
pub(crate) fn load_state(mode: StoreMode, skipped_empty: bool) -> ReplicaState {
    match mode {
        StoreMode::ReadOnlyDegraded => ReplicaState::Degraded,
        StoreMode::ReadWrite if skipped_empty => ReplicaState::Writable,
        StoreMode::ReadWrite => ReplicaState::Degraded,
    }
}

/// Only SQLITE_BUSY/LOCKED are worth a bounded retry; everything else (corruption,
/// IO, ReadOnly) is persistent — retrying would just fail again.
pub(crate) fn is_transient(err: &PersistError) -> bool {
    matches!(
        err,
        PersistError::Sqlite(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy
                || e.code == rusqlite::ErrorCode::DatabaseLocked
    )
}

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
    pub fn layout(&self) -> &BoardLayout { &self.layout }
    pub fn state(&self) -> ReplicaState { self.state }
    pub fn is_writable(&self) -> bool { state_is_writable(self.state) }
}
```

In `mod.rs`: `pub mod replica;` and `pub use replica::{BoardReplica, ReplicaState, WriteDisposition};`. In `crates/lens-ui/Cargo.toml`, add `tempfile` to `[dependencies]` (match the version `lens-core` dev-deps).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS. (Unused-field warnings until Task 4 are expected; do not `#[allow(dead_code)]`.)

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs crates/lens-ui/src/board/mod.rs crates/lens-ui/Cargo.toml
git commit -m "feat(board): BoardReplica types + pure state/error helpers"
```

---

## Task 4: serialized `run_op` pump + `Load`/`Place` happy path

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline, `#[gpui::test]`)

**Interfaces:**
- Produces: `build` (§ conventions), `for_test`, `run_op`, `pump`, `apply_outcome`, the off-thread `run_op_blocking`/`run_op_inner`/`read_committed`, and internal `OpOutcome`.
- `for_test(fleet, cx) -> Self` — tempfile store, `conn = "conn_test"`.

**`OpOutcome` (internal, `Send`):**

```rust
enum OpOutcome {
    Loaded { layout: BoardLayout, skipped_empty: bool, mode: StoreMode, initial: bool },
    Placed { layout: BoardLayout, skipped_empty: bool, mode: StoreMode },
    Failed { op: Op, err: PersistError }, // carry op back for retry/recovery decisions
}
```

**Pump (verbatim — no `expect` in the async path, M10):**

```rust
fn pump(&mut self, cx: &mut Context<Self>) {
    if self.in_flight {
        return;
    }
    // Re-gate: drop write ops no longer allowed (state flipped after they queued).
    let op = loop {
        match self.pending.pop_front() {
            None => return,
            Some(Op::PlaceSessions(_)) if !self.is_writable() => {
                self.dropped_writes = self.dropped_writes.saturating_add(1);
                continue;
            }
            Some(op) => break op,
        }
    };
    self.in_flight = true;
    if let Op::PlaceSessions(ref keys) = op {
        self.last_attempt = keys.clone();
    }
    let store = Arc::clone(&self.store);
    cx.spawn(async move |this, cx| {
        let outcome = cx
            .background_executor()
            .spawn(async move {
                let mut slot = store.lock().unwrap_or_else(|p| p.into_inner()); // poison → recover, never panic
                run_op_blocking(&mut slot, op)
            })
            .await;
        this.update(cx, |this, cx| this.apply_outcome(outcome, cx)).ok();
    })
    .detach();
}
```

**Off-thread runner (persist then reconciled read; drops the handle on Err so recovery reopens):**

```rust
fn run_op_blocking(slot: &mut StoreSlot, op: Op) -> OpOutcome {
    match run_op_inner(slot, &op) {
        Ok(outcome) => outcome,
        Err(err) => {
            slot.store = None; // reopen fresh on the next Load (recovery)
            OpOutcome::Failed { op, err }
        }
    }
}

fn run_op_inner(slot: &mut StoreSlot, op: &Op) -> lens_core::persist::Result<OpOutcome> {
    if slot.store.is_none() {
        slot.store = Some(Box::new(SqliteBoardStore::open(&slot.path)?)); // first-open or recovery
    }
    let Some(store) = slot.store.as_deref() else {
        return Err(PersistError::ReadOnly); // unreachable (just opened); typed, never a panic
    };
    match op {
        Op::Load { initial } => {
            let (layout, skipped_empty, mode) = read_committed(store)?;
            Ok(OpOutcome::Loaded { layout, skipped_empty, mode, initial: *initial })
        }
        Op::PlaceSessions(keys) => {
            store.place_sessions(keys, &default_root_target())?; // persist
            let (layout, skipped_empty, mode) = read_committed(store)?; // reconciled read (M5 rebuttal)
            Ok(OpOutcome::Placed { layout, skipped_empty, mode })
        }
    }
}

/// `load_layout` applies read-time reconcile (lazy-place + tombstone-prune), so this is
/// the authoritative committed view — for both Load and post-Place reads.
fn read_committed(store: &dyn BoardStore) -> lens_core::persist::Result<(BoardLayout, bool, StoreMode)> {
    let loaded = store.load_layout()?;
    let skipped_empty = loaded.skipped.is_empty();
    let layout = loaded.rows.into_iter().next().unwrap_or_default();
    Ok((layout, skipped_empty, store.mode()))
}

fn default_root_target() -> PlacementTarget {
    PlacementTarget { board_id: None, parent_item_id: None, ordinal: None }
}
```

**Apply (happy-path arms here; `on_op_failed` is Task 5):**

```rust
fn apply_outcome(&mut self, outcome: OpOutcome, cx: &mut Context<Self>) {
    self.in_flight = false;
    match outcome {
        OpOutcome::Loaded { layout, skipped_empty, mode, initial: _ } => {
            self.op_retries = 0;
            self.recovery_in_flight = false;
            self.layout = layout;
            self.state = load_state(mode, skipped_empty);
            if self.is_writable() {
                self.reconcile(cx); // initial/post-recovery reconcile (Task 6)
            }
        }
        OpOutcome::Placed { layout, skipped_empty, mode } => {
            self.op_retries = 0;
            self.layout = layout;
            self.state = load_state(mode, skipped_empty); // ~always Writable; consistent
            self.reconcile_in_flight = false;
            self.note_place_result();  // suppress stuck keys (Task 6, C1)
            self.reconcile(cx);        // re-diff on reply (Task 6)
        }
        OpOutcome::Failed { op, err } => {
            self.on_op_failed(op, err, cx); // Task 5
        }
    }
    cx.notify();
    self.pump(cx);
}
```

> For Task 4, stub `reconcile`/`note_place_result`/`on_op_failed`/`on_fleet_change` minimally (`reconcile`/`note_place_result` empty; `on_op_failed` = `self.state = ReplicaState::Stale;`; `on_fleet_change` empty). Tasks 5–6 fill them. `build`'s observer calls `on_fleet_change` (stub) — harmless.

- [ ] **Step 1: Write the failing tests**

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
    cx.run_until_parked();
    let c = ConnectionId::new("conn_test");
    replica.update(cx, |r, cx| {
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("a"))]), cx);
        r.run_op(Op::PlaceSessions(vec![(c.clone(), SessionId::new("b"))]), cx);
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        let n = r.layout().items.iter().filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
        assert_eq!(n, 2);
    });
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL — `for_test`/`run_op` unresolved.

- [ ] **Step 3: Implement `build` (§ conventions), `for_test`, `run_op`, the pump, runner, `apply_outcome`**

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

Add `use gpui::WeakEntity;` if needed (the `cx.spawn` closure's first arg is `WeakEntity<Self>`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: PASS (both).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): serialized run_op pump + off-thread Load/Place (single-in-flight)"
```

---

## Task 5: error classification, retry, recovery, write gating

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline; uses the barrier test-double from Step 1)

**Interfaces:**
- `on_op_failed(&mut self, op: Op, err: PersistError, cx)` — real body: transient → bounded backoff retry (op kept); persistent → `Stale`/`LoadFailed` + drop queued writes.
- `schedule_retry(&mut self, op: Op, backoff: Duration, cx)`.
- `write(&mut self, op: Op, cx) -> WriteDisposition` — `Accepted` iff writable; else `Rejected(state)` + re-surface banner.
- `begin_recovery(&mut self, cx)` (coalesced), `retry_recovery(&mut self, cx)` (banner Retry).
- `for_test_file(fleet, path, cx)` — bad-path ctor for failure tests.

**Behaviour (§5):**

```rust
fn on_op_failed(&mut self, op: Op, err: PersistError, cx: &mut Context<Self>) {
    // Transient (SQLITE_BUSY/LOCKED beyond busy_timeout): keep the op, back off, retry.
    if is_transient(&err) && self.op_retries < MAX_RETRIES {
        self.op_retries += 1;
        let backoff = std::time::Duration::from_millis(50u64 << self.op_retries.min(6)); // 100,200,…,≤3200ms
        self.schedule_retry(op, backoff, cx);
        return;
    }
    // Persistent (or retries exhausted).
    self.op_retries = 0;
    self.reconcile_in_flight = false;
    self.recovery_in_flight = false;
    self.last_attempt.clear();
    match op {
        Op::Load { initial: true } => {
            self.state = ReplicaState::LoadFailed;
            self.layout = default_board_layout(); // never loaded → render empty default, no panic
        }
        Op::Load { initial: false } => {
            // Failed RECOVERY: preserve visible data; a writable store just lost writability.
            if self.state == ReplicaState::Writable {
                self.state = ReplicaState::Stale;
            } // else keep Degraded/LoadFailed/Stale + existing layout
        }
        Op::PlaceSessions(_) => {
            self.state = ReplicaState::Stale; // keep current layout
        }
    }
    // Persistent failure: queued writes won't succeed on replay — drop (banner names them).
    let dropped = self.pending.iter().filter(|o| matches!(o, Op::PlaceSessions(_))).count() as u32;
    self.dropped_writes = self.dropped_writes.saturating_add(dropped);
    self.pending.retain(|o| matches!(o, Op::Load { .. }));
    self.banner_dismissed = false;
    cx.notify();
}

fn schedule_retry(&mut self, op: Op, backoff: std::time::Duration, cx: &mut Context<Self>) {
    self.pending.push_front(op);     // preserve ordering
    self.in_flight = true;           // hold the single-in-flight slot across the backoff
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(backoff).await;
        this.update(cx, |this, cx| { this.in_flight = false; this.pump(cx); }).ok();
    })
    .detach();
}

pub fn write(&mut self, op: Op, cx: &mut Context<Self>) -> WriteDisposition {
    if !self.is_writable() {
        self.banner_dismissed = false; // re-surface the banner on a rejected gesture
        cx.notify();
        return WriteDisposition::Rejected(self.state);
    }
    self.run_op(op, cx);
    WriteDisposition::Accepted
}

fn begin_recovery(&mut self, cx: &mut Context<Self>) {
    if self.recovery_in_flight {
        return; // coalesce: at most one recovery in flight (bounded, §5)
    }
    self.recovery_in_flight = true;
    self.run_op(Op::Load { initial: false }, cx); // Load is always allowed, any state
}

pub fn retry_recovery(&mut self, cx: &mut Context<Self>) {
    self.banner_dismissed = false;
    self.begin_recovery(cx);
}
```

> `in_flight` is set true both by `pump` (normal) and `schedule_retry` (backoff window), so `apply_outcome`'s trailing `self.pump(cx)` no-ops during a scheduled retry — the timer resumes it. `background_executor().timer(Duration)` is gpui 0.2.2's delay primitive. **Chatty-fleet throttle** on a persistently-failing store is the design's noted-not-built min-interval throttle (§5) — coalescing bounds it to one attempt at a time.
>
> `for_test_file(fleet, path, cx)`: `Self::build(None, path, ConnectionId::new("conn_test"), None, fleet, cx)` — `store: None` + a bad path → `ensure_open` fails → `LoadFailed`.

**Barrier test-double (Minor 12).** Add a `#[cfg(test)]` `BoardStore` impl that wraps a real `SqliteBoardStore` and, per a `crossbeam_channel` gate, **blocks** a `place_sessions`/`load_layout` call until the test releases it — a blocking recv, NOT a `yield_now` busy-spin ([[worker-stall-gate-busy-spin-flake]]). It can also be armed to return a chosen `PersistError` once. Used to deterministically exercise in-flight coalescing and injected failures.

- [ ] **Step 1: Write the failing tests (with the barrier double)**

```rust
#[gpui::test]
async fn failed_initial_load_seeds_default_board(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::LoadFailed);
        assert_eq!(r.layout().default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
    });
}

#[gpui::test]
async fn write_rejected_when_non_writable(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
    });
    cx.run_until_parked(); // → LoadFailed
    let d = replica.update(cx, |r, cx| {
        r.write(Op::PlaceSessions(vec![(r.conn.clone(), SessionId::new("x"))]), cx)
    });
    assert_eq!(d, WriteDisposition::Rejected(ReplicaState::LoadFailed));
    replica.read_with(cx, |r, _| assert!(r.pending.is_empty()));
}

// Recovery: a store that fails once then succeeds (barrier double armed to fail the first Load).
#[gpui::test]
async fn recovery_load_restores_writable(cx: &mut gpui::TestAppContext) {
    // build a replica on a barrier double armed: first Load → Err(DatabaseBusy) exhausting retries,
    // OR a bad path swapped good; assert state goes LoadFailed → (retry_recovery) → Writable.
    // (Wire via a for_test_double ctor that takes the armed store.)
}
```

- [ ] **Step 2–4: Red → implement `on_op_failed`/`schedule_retry`/`write`/`begin_recovery`/`retry_recovery`/`for_test_file` + the barrier double → green.**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL then PASS. Add a transient-retry test (barrier double returns `DatabaseBusy` twice then succeeds → op eventually applies, `op_retries` reset).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): typed retry, recovery coalescing, write gating + WriteDisposition"
```

---

## Task 6: reconcile (batched, conn-pinned, suppress-stuck, re-diff on reply)

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces:**
- `on_fleet_change(&mut self, cx)` — the observer target (installed by `build`): writable → `reconcile`; non-writable+error → `begin_recovery`.
- `reconcile(&mut self, cx)`, `missing_keys(&self, cx) -> Vec<(ConnectionId, SessionId)>`, `placed_key_strings(&self) -> HashSet<(String,String)>`, `note_place_result(&mut self)`.

```rust
fn on_fleet_change(&mut self, cx: &mut Context<Self>) {
    if self.is_writable() {
        self.reconcile(cx);
    } else if matches!(self.state, ReplicaState::Degraded | ReplicaState::LoadFailed | ReplicaState::Stale) {
        self.begin_recovery(cx); // automatic recovery on fleet activity (§5)
    }
    // Loading: initial Load in flight; nothing to do.
}

fn placed_key_strings(&self) -> HashSet<(String, String)> {
    self.layout.items.iter().filter_map(|i| match &i.kind {
        BoardItemKind::Card { conn, session } =>
            Some((conn.as_str().to_string(), session.as_str().to_string())),
        _ => None,
    }).collect()
}

fn missing_keys(&self, cx: &Context<Self>) -> Vec<(ConnectionId, SessionId)> {
    let placed = self.placed_key_strings();
    // snapshot fleet keys, then diff (avoids holding the fleet borrow)
    let live: Vec<SessionId> = self.fleet.read(cx).cards.keys().cloned().collect();
    live.into_iter().filter_map(|s| {
        let k = (self.conn.as_str().to_string(), s.as_str().to_string());
        if placed.contains(&k) || self.suppressed.contains(&k) { None }
        else { Some((self.conn.clone(), s)) }
    }).collect()
}

fn reconcile(&mut self, cx: &mut Context<Self>) {
    if !self.is_writable() {
        return;
    }
    let missing = self.missing_keys(cx);
    if missing.is_empty() {
        return;
    }
    if self.reconcile_in_flight {
        return; // coalesce; the in-flight place's reply re-diffs
    }
    self.reconcile_in_flight = true;
    self.run_op(Op::PlaceSessions(missing), cx); // pump records last_attempt
}

/// C1: an attempted key STILL missing after its place is tombstoned/stuck → suppress it,
/// so re-diff-on-reply cannot re-enqueue it forever.
fn note_place_result(&mut self) {
    let placed = self.placed_key_strings();
    for (c, s) in std::mem::take(&mut self.last_attempt) {
        let k = (c.as_str().to_string(), s.as_str().to_string());
        if !placed.contains(&k) {
            self.suppressed.insert(k);
        }
    }
}
```

> `missing_keys` takes `&Context<Self>` (derefs to `App` for the `fleet.read`). The `Placed` arm (Task 4) calls `note_place_result()` **then** `reconcile(cx)`: stuck keys are suppressed before the re-diff, so the diff shrinks monotonically and settles. A genuinely new card (coalesced during the in-flight place) is not suppressed → placed on the re-diff.

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
            BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()), _ => None,
        }).collect();
        assert_eq!(placed, vec!["s1".to_string()]);
    });
}

#[gpui::test]
async fn tombstoned_fleet_key_settles_no_loop(cx: &mut gpui::TestAppContext) {
    // Seed a session, tombstone it in the store, keep its card in fleet.cards.
    // Assert reconcile suppresses it after one attempt and the pump goes idle (no hang).
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();
    // ... tombstone "s_dead" in the replica's store (helper), insert its card into fleet.cards ...
    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("s_dead"), cx); });
    cx.run_until_parked(); // MUST settle
    replica.read_with(cx, |r, _| {
        assert!(!r.in_flight && r.pending.is_empty(), "no infinite reconcile");
        assert!(r.suppressed.contains(&("conn_test".into(), "s_dead".into())));
    });
}

#[gpui::test]
async fn coalesced_late_card_placed_via_barrier(cx: &mut gpui::TestAppContext) {
    // Using the barrier double: block the first place, add a 2nd card (its notify coalesces),
    // release, assert BOTH end placed and it settles. (Deterministic, not add-both-then-park.)
}

#[gpui::test]
async fn double_reconcile_idempotent(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();
    fleet.update(cx, |f, cx| { f.spawn_fake_session(SessionId::new("s1"), cx); });
    cx.run_until_parked();
    replica.update(cx, |r, cx| r.reconcile(cx));
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        let n = r.layout().items.iter().filter(|i| matches!(i.kind, BoardItemKind::Card { .. })).count();
        assert_eq!(n, 1);
    });
}
```

- [ ] **Step 2–4: Red → implement `on_fleet_change`/`reconcile`/`missing_keys`/`placed_key_strings`/`note_place_result` → green.**

Run: `cargo test -p lens-ui --lib board::replica`
Expected: FAIL then PASS (all four; the tombstone test must terminate).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): conn-pinned reconcile w/ suppress-stuck + re-diff-on-reply (no tombstone loop)"
```

---

## Task 7: production `new`

**Files:**
- Modify: `crates/lens-ui/src/board/replica.rs`
- Test: `crates/lens-ui/src/board/replica.rs` (inline)

**Interfaces:**
- `BoardReplica::new(store: Option<Box<dyn BoardStore + Send>>, path: PathBuf, conn: ConnectionId, fleet: Entity<FleetStore>, cx) -> Self` — prod. `store` is the bootstrap-opened handle (Task 8) or `None` if that open failed; `path` lets `ensure_open`/recovery (re)open. `None` + a bad path → `LoadFailed` with the **real** conn.

> The demo does **not** get a `for_demo` ctor — demo seeding happens in `main` before `Application::run` (Task 8), then the demo calls `new(Some(seeded_store), …)` like prod.

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
}
```

- [ ] **Step 1: Write the failing test**

```rust
#[gpui::test]
async fn new_with_none_store_and_bad_path_is_load_failed(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    let replica = cx.update(|cx| {
        cx.new(|cx| BoardReplica::new(None, "/dev/null/nope.db".into(), ConnectionId::new("lens-app"), fleet.clone(), cx))
    });
    cx.run_until_parked();
    replica.read_with(cx, |r, _| {
        assert_eq!(r.state(), ReplicaState::LoadFailed);
        assert_eq!(r.conn.as_str(), "lens-app"); // real conn, not a test ctor
    });
}
```

- [ ] **Step 2–4: Red → add `new` → green.**

Run: `cargo test -p lens-ui --lib board::replica::new_with_none`
Expected: FAIL then PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): production BoardReplica::new (Option<store> + reopen path)"
```

---

## Task 8: wire `BoardView` + app bootstrap + demo seed

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs` (`mount`/`new` gain `replica`; `pack_and_render` reads replica; retire `test_layout`; observe replica)
- Delete: `crates/lens-ui/src/board/layout_adapter.rs`
- Modify: `crates/lens-app/src/main.rs` (bootstrap open before actors; live + demo wiring)
- Modify: `crates/lens-ui/tests/acceptance_shell.rs` (5 call sites)
- Test: migrated group-chrome fixture in `board/mod.rs`

**Interfaces:**
- `BoardView::mount(fleet, replica: Entity<BoardReplica>, working_tab, pty_probe, cx)`.
- `BoardView::new(fleet, replica: Entity<BoardReplica>, cx)` — update the wrapper (mod.rs:131) to thread `replica` into `mount`.
- `pack_and_render` reads `self.replica.read(cx).layout().clone()`; the `default_board_id() == Err` guard (mod.rs:230-233) stays.

- [ ] **Step 1: Migrate the B-3 group fixture test to the real path**

```rust
#[gpui::test]
async fn group_chrome_renders_via_replica(cx: &mut gpui::TestAppContext) {
    let fleet = cx.update(|cx| test_fleet(cx));
    // Seed a group into a for_test replica's store, then reload:
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
    cx.run_until_parked();
    replica.update(cx, |r, cx| seed_group_for_test(r, cx)); // helper: create_group + place members, then Load
    cx.run_until_parked();
    let (board, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx)
    });
    board.read_with(&vcx, |b, _| {
        assert!(b.group_chrome_for_test().len() >= 1, "group renders through the real store path");
    });
}
```

> `seed_group_for_test` uses the replica's store handle (a test-only accessor) to `create_group` + `place_session` members, then enqueues a `Load`. Model the group-chrome assertion on the retired `test_layout` fixture (`group_chrome_for_test`).

Run: `cargo test -p lens-ui --test acceptance_shell group_chrome_renders_via_replica` (or `--lib`).
Expected: FAIL — `mount` arity / `test_layout` gone.

- [ ] **Step 2: Rewire `BoardView`**

- Add `replica: Entity<BoardReplica>`; **remove** `test_layout`.
- `mount` gains `replica`, stores + observes it:

```rust
    pub fn mount(
        fleet: Entity<FleetStore>,
        replica: Entity<BoardReplica>,
        working_tab: TabHandle,
        pty_probe: Option<PtyProbe>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&replica, |_b: &mut BoardView, _, cx| cx.notify()).detach();
        // ... existing fleet observe stays ...
```

- Update `new` (mod.rs:131) to take + forward `replica`:

```rust
    pub fn new(fleet: Entity<FleetStore>, replica: Entity<BoardReplica>, cx: &mut App) -> Entity<Self> {
        let working_tab = /* existing */;
        cx.new(|cx| Self::mount(fleet, replica, working_tab, None, cx))
    }
```

- `pack_and_render`:

```rust
        let layout = self.replica.read(cx).layout().clone();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
```

- Delete `mod layout_adapter;` + `use layout_adapter::build_ephemeral_layout;`; `git rm` the file.
- Update any caller of `BoardView::new` (grep) to pass a replica.

- [ ] **Step 3: App wiring — live + demo (`main.rs`)**

Bootstrap-open the board store before the session actors, in `open_stores`/bootstrap (before `Application::new`):

```rust
    let board_db = data_dir.join("lens.db");
    let mut board_store_for_window: Option<Box<dyn BoardStore + Send>> =
        SqliteBoardStore::open(&board_db).ok().map(|s| Box::new(s) as _); // None on failure → LoadFailed+recover
```

At the live `BoardView::mount` sites (main.rs:110,165), build the replica first:

```rust
                let replica = cx.new(|cx| BoardReplica::new(
                    board_store_for_window.take(), board_db.clone(),
                    conn_id.clone() /* "lens-app" */, fleet.clone(), cx,
                ));
                let board = cx.new(|cx| {
                    BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx)
                });
```

**Demo (`run_demo`, main.rs:123):** open + seed the board store **before** `Application::new().run` (compliant); inside the window, pass it to `new`. The demo already inserts cards into `fleet.cards` (`new_live`, main.rs:156-163) — pin the replica conn to a demo id and seed a group over some of those session-ids:

```rust
    // BEFORE Application::new():
    let demo_dir = tempfile::tempdir().expect("demo tempdir");
    let demo_db = demo_dir.path().join("board.db");
    let demo_conn = ConnectionId::new("lens-app"); // match cards' placement conn
    let demo_store: Option<Box<dyn BoardStore + Send>> = seed_demo_group(&demo_db, &demo_conn).ok();
    // ... move demo_dir/demo_db/demo_store into the run closure; inside cx.open_window:
                let replica = cx.new(|cx| BoardReplica::new(
                    demo_store_for_window.take(), demo_db.clone(), demo_conn.clone(), fleet.clone(), cx));
                let board = cx.new(|cx| BoardView::mount(fleet.clone(), replica.clone(), placeholder_tab(cx), None, cx));
```

`seed_demo_group(db, conn)`: `SqliteBoardStore::open(db)?`, `create_group(default board, None, 0, "Demo group")`, `place_session` two of `demo_preset_cards`' session-ids under it; return the boxed store. Runs before `Application::run` → off-thread rule respected. Reconcile places the remaining demo cards loose; the two seeded ones render under group chrome.

- [ ] **Step 4: Update the 5 acceptance-test call sites (`acceptance_shell.rs:81,194,292,643,748`)**

```rust
        let replica = cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx));
        let (board_handle, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet_for_window, replica.clone(), placeholder_tab(cx), None, cx)
        });
```

- [ ] **Step 5: Run tests + eyeball the app**

Run: `cargo test -p lens-ui`
Then: `cargo run -p lens-app --features demo -- --demo` — confirm the board renders and the seeded **Demo group** shows group chrome around its two member cards.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-app/src/main.rs crates/lens-ui/tests/acceptance_shell.rs
git rm crates/lens-ui/src/board/layout_adapter.rs
git commit -m "feat(board): BoardView reads BoardReplica; app+demo wiring; retire ephemeral stub"
```

---

## Task 9: non-blocking error banner

**Files:**
- Modify: `crates/lens-ui/src/board/mod.rs`
- Modify: `crates/lens-ui/src/board/replica.rs` (getters: `banner_dismissed()`, `dropped_writes()`, `dismiss_banner()`)
- Test: `crates/lens-ui/src/board/mod.rs` (inline)

**Interfaces:**
- `BoardView::banner_text(&self, cx) -> Option<String>` from `replica.state()` + `!banner_dismissed`; Retry → `replica.update(|r,cx| r.retry_recovery(cx))`; Dismiss → `r.dismiss_banner()`.

**Copy (§5; honest about multi-write loss, M8):**
- `Degraded`: "Some board items couldn't be read — changes won't save."
- `LoadFailed`: "Couldn't load your board — data on disk is untouched."
- `Stale`: base "Couldn't save — reconnecting."; if `dropped_writes > 0`, append " (N change(s) not saved)."
- `Loading`/`Writable`: `None`.

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
    board.read_with(&vcx, |b, cx| assert!(b.banner_text(cx).is_some()));
}
```

- [ ] **Step 2–4: Red → `banner_text` mapping + the non-modal dismissible notice element + Retry/Dismiss wiring → green.**

Run: `cargo test -p lens-ui --lib banner_shows_for_load_failed`
Expected: FAIL then PASS. (Assert the mapping; painted pixels are out of scope per [[gpui-test-noop-text-system]].)

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/board/mod.rs crates/lens-ui/src/board/replica.rs
git commit -m "feat(board): non-blocking ReplicaState banner (Retry/Dismiss, honest loss count)"
```

---

## Task 10: perf — three measures + gate

**Files:**
- Create: `crates/lens-core/benches/board_pack.rs`; Modify: `crates/lens-core/Cargo.toml`
- Use (no new spike): `lens-app --features demo -- --demo` with `LENS_DEMO_N` (real `BoardView`)

**Measures (§7; the E2E is prior-slice B-2/B-3 debt paid down here — a sizable task, not a swap):**

- [ ] **Step 1: `lens-core` `board_tree` criterion bench (supporting, gate-automatable)**

```rust
// crates/lens-core/benches/board_pack.rs
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_board_tree(c: &mut Criterion) {
    let layout = build_layout_with_group(1000); // helper: 1000 cards + one group
    let board = layout.default_board_id().unwrap().clone();
    c.bench_function("board_tree_1000_with_group", |b| {
        b.iter(|| criterion::black_box(layout.board_tree(&board).unwrap().len()))
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

Run: `cargo bench -p lens-core --bench board_pack`; record the baseline in the commit body.

- [ ] **Step 2: Frame-budget E2E on the REAL path (MANDATORY, M11)**

Run the demo binary (real `BoardView` + real replica + real group chrome) at realistic and stress N — `LENS_DEMO_N` replicates the 8 preset cards, so `N≈100` → `LENS_DEMO_N=12`, `N≈1000+` → `LENS_DEMO_N=125`:

```bash
cargo build -p lens-app --features demo --release
LENS_DEMO_N=12  ./target/release/lens-app --demo   # ~100 cards + a group
LENS_DEMO_N=125 ./target/release/lens-app --demo   # ~1000 cards + a group
```

Sample **frame time / FPS + CPU** via the existing `spawn_demo_paint_instrumentation` + the [[wave-perf-fps-attribution]] `measure.sh` approach **pointed at this binary** (CPU is per-frame full-tree re-render, not paint — sample accordingly). Hold 120fps/8.3ms target; flag 90fps/11.1ms. Record numbers (cull ON) in the commit body. This is the first at-scale exercise of B-3 group render-time member reads on the real app — closes B-2 Task 6's residual.

> If the demo instrumentation reports paint-only, extend it (or `measure.sh`) to capture wall-clock frame interval; the metric that gates is frame time on the real render path, not the pure `board_tree` bench.

- [ ] **Step 3: Op-latency (off-frame, supporting)**

A `#[gpui::test]` timing `Load` + a batched `PlaceSessions` of N via `run_until_parked` wall-clock — confirm prompt appearance and that the mutex isn't held excessively. **Not** a frame-budget assertion; log, don't gate.

- [ ] **Step 4: Full gate**

Run: `cargo xtask gate`
Expected: green — zero warnings, `cargo fmt --check`, all tests, benches build. Do not pipe through `tail`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/benches/board_pack.rs crates/lens-core/Cargo.toml
git commit -m "perf(board): pack bench + real-path at-scale group render E2E (B-2/B-3 debt) + op-latency"
```

---

## Self-review checklist (run before handing off)

- **Spec coverage:** §1 scope → T3–T9; §2 components → T3–T7; §3 read/write/reconcile → T4/T6/T8; §4 pinned conn → Global Constraints + T8; §5 error/banner/recovery/retry → T5/T9; §6 construction order → T7/T8; §7 testing+perf → each task + T10; §8 seams untouched. Deferred (B-4b/c/d, B-5, B-6) untouched.
- **Codex findings:** C1 (suppress-stuck T6), C2 (mode+initial/recovery T4/T5), C3 (HashSet/pub use/BoardView::new/tempfile T2/T3/T8), C4 (demo pre-run seed T8), M5 (compose-reload kept, rebutted), M6 (busy_timeout T1 + typed retry T5), M7 (recovery coalescing T5/T6), M8 (WriteDisposition T5 + banner T9), M9 (claim corrected T2), M10 (no-expect pump T4), M11 (real-path E2E T10), Minor 12 (barrier double T5/T6).
- **Type consistency:** `place_sessions(&[(ConnectionId, SessionId)], &PlacementTarget) -> Result<()>` (T2/T4); `Op`/`ReplicaState`/`WriteDisposition`/`OpOutcome`/`StoreSlot` stable (T3–T6); `mount(fleet, replica, working_tab, pty_probe, cx)` identical in T8 app + tests; `is_transient`/`load_state`/`load_state` used consistently.
- **Placeholder scan:** store/fleet ctors verified (`SqliteBoardStore::open` + tempfile; `FleetStore::new` + `ManualUiClock`). Remaining "mirror the sibling" pointers are test-only setup (`tombstone_session`, `seed_group_for_test`, barrier double), not production code.
- **Review diversity (MANDATORY):** after code lands, one cross-family review of the whole B-4a diff incl. a gate-runner ([[review-spend-policy]], [[whole-branch-review-needs-a-builder]]).
