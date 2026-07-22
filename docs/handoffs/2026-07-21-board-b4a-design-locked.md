# Handoff — 2026-07-21 — B-4a design LOCKED, ready for writing-plans

**State of `main`:** B-3 shipped (tip after this session's docs commits). B-4a is a
**design-only** session — no B-4a code yet. Spec + STATUS + memory committed. `main` is
ahead of `origin` (unpushed; push is the user's call, per [[commit-when-finished]]).

## Where to pick up

**Next action: run `writing-plans` on the B-4a spec**, then execute subagent-driven (same
as B-3: composer-2.5 implementers, codex gpt-5.6 cross-family review per task, Opus
whole-branch). Branch `board-b4a` off main first (never implement on main).

- **Spec (LOCKED):** `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md`
- **Memory:** [[board-b4a-design]]

## What B-4a is

Foundation slice of B-4 (which was decomposed into B-4a + interaction follow-ons
B-4b collapse / B-4c drag / B-4d grouping-menus). B-4a replaces the ephemeral
`build_ephemeral_layout` stub with a persisted `BoardLayout` from `SqliteBoardStore`, via a
new `BoardReplica` gpui entity. **No user interactions** — the board renders from the real
store and survives restarts. That's the whole deliverable.

## The hard-won design decisions (don't re-litigate; they cost a grill + a review)

1. **All SQLite is off-thread** (`Arc<Mutex<Box<dyn BoardStore>>>` + `cx.background_spawn`),
   behind a **serialized single-in-flight `run_op`** on the main-thread `BoardReplica`. This
   is MANDATORY (AGENTS.md:19 / `.agents/rust-ui.md`) — the codex review caught that the
   grill's "inline SQLite is fine" **violated** it. **Renders read the in-memory
   `replica.layout`, never SQLite.** Single-in-flight is required for correctness *and* test
   determinism (concurrent `background_spawn` replies land in thread-pool order → out-of-order
   regress + flaky tests). NOT a persistent actor — transient per-op spawns, queue on the entity.
2. **`run_op` is the write seam** B-4b/c/d extend by adding `Op` variants (no serialization
   rework). `write` gates on `is_writable()`; `reconcile` too; **`Load`/recovery ops are
   always allowed** (recovery = reopen-`Load`, since `StoreMode` is immutable).
3. **Conn pinned to the app `Connection.id` (`"lens-app"`)** so `BoardReplica`'s FleetStore
   placement converges with `load_layout`'s built-in sessions-table reconcile (no double-place).
4. **Non-fatal errors:** explicit `ReplicaState` (Loading/Writable/Degraded/LoadFailed/Stale)
   + non-blocking banner + "Retry"→recovery `Load`. Silent-empty-board is the bad UX to avoid.
5. **Demo seeds a group** → B-3 chrome renders live for the first time; verify the B-3
   `.cached()` member-read-during-render carryforward here (`board/mod.rs:384`).
6. **Perf = three measures:** MANDATORY frame-budget = **E2E lens-ui on-device** (`measure.sh`
   rig, board of N *with a group*), NOT just the pure `lens-core` pack bench (which is a
   supporting CI bench); op-latency is an off-frame metric.

## Codex review dispositions (all 10 folded into the spec §0)

2 Critical + 6 Important + 2 Minor, all confirmed against code. The two that reversed grill
decisions: **#1 off-thread (Q4 reversed)** and **#9 tombstone-resurrection already prevented**
(`place_session` guards `tombstoned_at`, `board.rs:437` — the deferred guard was deleted).
Others: `new()` must take `Entity<FleetStore>`; `Loaded{rows,skipped}` not `.value` (surface
partial loads); `(ConnectionId,SessionId)` placed keys; `SQLITE_BUSY` on open → `LoadFailed`;
batched `PlaceSessions`; corrected FleetStore-observation rationale (card content flows via
each `SessionCardView`'s own observation, not FleetStore).

## Process lesson (already in [[board-b4a-design]])

**Check `AGENTS.md` + `.agents/*` MANDATORY rules BEFORE designing threading/I-O.** The entire
Q4 grill (frame-budget math, "inline fine below ~1000 items") was built on ignorance of the
codified off-thread-I/O mandate. Grill against the repo's own constraints first; cross-family
review is the net for what the grill misses.
