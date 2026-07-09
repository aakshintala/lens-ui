# State-model P3-1: Actor Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the off-thread `ActiveSession` actor and its foreground `SessionStore` gpui replica, proving the single-writer actor↔replica seam end-to-end with a value-carrying `StreamUpdate` delta stream.

**Architecture:** A blocking OS-thread actor (`lens-core`, gpui-free) owns canonical `SessionState`, is the sole writer, and `crossbeam::Select`s over the lens-client event receiver + a command receiver. Per event it `reduce()`s, write-throughs to the P2 stores, and emits a value-carrying `StreamUpdate` (focused/`Detailed`) or `SummaryUpdate` (background/`Summary`) over an `async-channel`. A foreground `cx.spawn` task in `lens-store` (gpui) drains that channel, greedily coalesces a batch, and applies each delta by pure copy-assignment into an `Entity<SessionState>` replica, then one `cx.notify()`.

**Tech Stack:** Rust (edition 2024, rust-version 1.91), `crossbeam-channel` (actor `Select`), `async-channel` (actor→replica bridge, re-exported by gpui's `smol`), `gpui` 0.2.2 (`lens-store` only), `rusqlite` (P2 stores, already present), `smallvec`, `serde` (with `rc` feature for `Arc<Item>`).

## Global Constraints

- Edition **2024**, `rust-version = "1.91"`; crates under `crates/` set `lints.workspace = true` (spikes deliberately do not).
- `crates/lens-client/src/generated.rs` is **NEVER** hand-edited (codegen artifact).
- **No `serde_json::Value` leaks to consumers** — typed wrappers only (established across lens-client).
- **`lens-core` has NO gpui dependency.** All gpui touch-points live in `lens-store`. (spec §3)
- Each phase lands **green and independently**: `cargo fmt --check`, `cargo clippy --all-targets` (0 warnings, `all = deny`), `cargo test` all pass.
- **No foreground blocking** — every network/disk I/O runs on the actor's OS thread or a gpui background executor; the foreground only does copy-assignment `apply` + `cx.notify()`. (AGENTS.md MANDATORY)
- **Benchmark-or-it's-not-done** on hot paths — `StreamUpdate::apply` cost is measured. The 120fps/90fps-regression contract applies to the foreground apply path. (spec §5)
- **MANDATORY cross-family review** at the lens-client touch (Task 1) and the actor run-loop (Task 5) — temporal/stateful code, `[[composer-delegation-profile]]`. Mind Cursor-credit spend (`[[review-spend-policy]]`) — consolidate where cheap.
- Authoritative decisions: spec `docs/superpowers/specs/2026-07-08-state-model-engine-design.md` §2.1 (D8–D14) and §2.2 (D15–D18). This plan implements the **P3 foundation subset**: D8, D9, D10, D13 + the walking skeleton (D7). Commands (D16, §7), lifecycle/quiesce/sleep/wake (D17, §3), and error-mapping (D18, §13.1) are **P3-2/P3-3** and out of scope here.

---

## File Structure

**New crate `crates/lens-store/`** (gpui replica layer):
- `Cargo.toml` — depends on `gpui`, `lens-core`, `async-channel`.
- `src/lib.rs` — `SessionStore` (`Entity<SessionState>` wrapper), `spawn_apply_bridge`, re-exports.

**New module tree `crates/lens-core/src/actor/`** (gpui-free actor):
- `mod.rs` — `ActiveSession`, `SessionCommand` (minimal: `Stop`), `ActorHandle`, `OutputMode`, spawn.
- `runloop.rs` — the `crossbeam::Select` run-loop + per-event reduce/persist/emit + coalescing.
- `summary.rs` — `SummaryUpdate` type + derive-from-state.

**Modified `crates/lens-core`:**
- `src/domain/session.rs` — `items: Vec<Arc<Item>>` (D8).
- `src/domain/item.rs` — (no shape change; `Item` gains no fields) — verify `Arc` serde.
- `src/reduce/update.rs` — `StreamUpdate` becomes **value-carrying** + `Rebased`/`SummaryReplaced` (D8/D9).
- `src/reduce/items.rs` — `push_item` wraps `Arc`, `Arc::make_mut` on update-in-place, carries payload in the emitted delta.
- `src/reduce/{folds,scratch,snapshot,mod}.rs` — thread the payload into each emitted `StreamUpdate` variant.
- `src/persist/{control,transcript}.rs` — call-site `&**arc` derefs where `&Item` is passed.
- `Cargo.toml` — `crossbeam-channel` dep, `serde` `rc` feature.

**Modified `crates/lens-client`:**
- `src/stream/reader.rs` — reader channel `mpsc::sync_channel` → `crossbeam_channel::bounded`, add `EventStream::receiver()` (Task 1).

**Modified workspace:**
- `Cargo.toml` — `crossbeam-channel` + `async-channel` in `[workspace.dependencies]` (if that table is used) or per-crate.

---

## Task 1: lens-client reader channel → crossbeam + `receiver()` accessor

The actor must `Select` over the event receiver, but `std::sync::mpsc` cannot be selected. Swap the reader's internal channel to `crossbeam_channel::bounded` (same backpressure + drop-unblock semantics) and expose a `receiver()` accessor. This is the **first modification of hardened/feature-complete lens-client** — re-verify `stop()` and drop-unblocks-blocked-sender, and this task's diff gets **MANDATORY cross-family review** (spec §7.2).

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs`
- Modify: `crates/lens-client/Cargo.toml` (add `crossbeam-channel`)
- Test: `crates/lens-client/src/stream/reader.rs` (existing `#[cfg(test)]` module — the ~8 `sync_channel` call-sites move to crossbeam)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `EventStream::receiver(&self) -> &crossbeam_channel::Receiver<ServerStreamEvent>` — the accessor the actor `Select`s over (Task 5).
  - Internal `run(...)` now takes `crossbeam_channel::Sender<ServerStreamEvent>`; `EVENT_CHANNEL_BOUND` unchanged (1024).
  - `EventStream::recv()`/`try_recv()`/`stop()` keep identical public signatures & semantics.

- [ ] **Step 1: Add the dependency**

In `crates/lens-client/Cargo.toml` under `[dependencies]`:
```toml
crossbeam-channel = "0.5"
```
Run: `cargo tree -p lens-client -i crossbeam-channel` → confirm it resolves.

- [ ] **Step 2: Write a failing test for the accessor + drop-unblock**

Add to the `#[cfg(test)]` module in `reader.rs`:
```rust
#[test]
fn receiver_accessor_yields_the_same_stream_as_recv() {
    // A scripted reopener that serves one canned event then EOF (reuse the
    // existing test scaffolding pattern in this module — see the other
    // `mpsc::sync_channel`-based tests for the Reopen mock).
    let stream = spawn_test_stream_with_one_event(); // helper already in this module
    // Draining via the accessor must observe the same event `recv()` would.
    let ev = stream.receiver().recv().expect("event via accessor");
    assert!(matches!(ev, ServerStreamEvent::Session(_) | ServerStreamEvent::Response(_)));
}

#[test]
fn dropping_stream_unblocks_a_parked_reader() {
    // Fill the bounded channel so the reader parks in send, then drop the
    // EventStream (drops the receiver) and confirm the thread joins (no leak).
    let stream = spawn_test_stream_that_floods(EVENT_CHANNEL_BOUND + 4);
    drop(stream); // receiver drop => next send errors => reader exits
    // The JoinHandle is owned by EventStream; if drop didn't unblock, this test
    // would hang. A watchdog assert keeps it honest:
    assert!(join_within(Duration::from_secs(2)), "reader failed to exit on drop");
}
```
> If `spawn_test_stream_with_one_event`/`spawn_test_stream_that_floods`/`join_within` don't already exist, factor them from the existing reader tests (which build a `Reopen` mock + `mpsc::sync_channel`). Keep the helpers crossbeam-based after Step 3.

- [ ] **Step 3: Swap the channel type**

In `reader.rs`:
```rust
// top of file
use crossbeam_channel::{bounded, Receiver, Sender};
// remove: use std::sync::mpsc;
```
```rust
pub struct EventStream {
    rx: Receiver<ServerStreamEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _handle: JoinHandle<()>,
}

impl EventStream {
    pub(crate) fn spawn<Re: Reopen + 'static>(
        resp: reqwest::blocking::Response,
        reopener: Re,
    ) -> crate::error::Result<Self> {
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        // ... unchanged: build stop flag, spawn thread calling run(...)
    }

    /// The raw event receiver, for a consumer that multiplexes it with other
    /// channels (the lens-core actor `Select`s over this + its command channel).
    pub fn receiver(&self) -> &Receiver<ServerStreamEvent> {
        &self.rx
    }

    pub fn recv(&self) -> Option<ServerStreamEvent> {
        self.rx.recv().ok()
    }
    pub fn try_recv(&self) -> Option<ServerStreamEvent> {
        self.rx.try_recv().ok()
    }
    // stop() unchanged.
}
```
Change `run`'s signature and every internal helper that took `&mpsc::SyncSender<ServerStreamEvent>` / `mpsc::SyncSender<...>` to `&Sender<ServerStreamEvent>` / `Sender<...>`. `Sender::send` returns `Result<(), SendError<_>>` — the existing `send(...).is_err()`/`?`-style handling maps 1:1 (crossbeam `SendError` on a dropped receiver mirrors `mpsc` behavior: the parked send unblocks and errors). Update every `mpsc::sync_channel(EVENT_CHANNEL_BOUND)` in the test module to `bounded(EVENT_CHANNEL_BOUND)`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lens-client stream::reader`
Expected: PASS, including the two new tests and all pre-existing reader tests (reconnect ordering, seq-dedup, bootstrap). No hang.

- [ ] **Step 5: Full gate**

Run: `cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets && cargo test -p lens-client`
Expected: fmt clean, 0 clippy warnings, all tests pass. Confirm `git diff --stat crates/lens-client/src/generated.rs` is empty.

- [ ] **Step 6: Commit + request review**

```bash
git add crates/lens-client/Cargo.toml crates/lens-client/src/stream/reader.rs
git commit -m "refactor(lens-client): reader channel std::mpsc -> crossbeam + receiver() accessor (P3 §7.2)"
```
Then request a **cross-family review of this diff** (temporal/stateful; the backpressure + stop/drop-unblock semantics). Do not proceed to Task 5's ingest until this review is clean.

---

## Task 2: `Vec<Arc<Item>>` + value-carrying `StreamUpdate` + `Rebased` (D8/D9)

Ratify D8: `StreamUpdate` carries its just-reduced value; `items` becomes `Vec<Arc<Item>>` so item **bodies are shared** actor↔replica (the replica's copy is a pointer spine, not a second copy of transcript bytes). Add the once-at-attach `Rebased(Box<SessionState>)` (D9). This restructures the P1 reducer's emit-sites — mechanical but wide.

**Files:**
- Modify: `crates/lens-core/src/domain/session.rs` (`items: Vec<Arc<Item>>`)
- Modify: `crates/lens-core/Cargo.toml` (`serde` `rc` feature)
- Modify: `crates/lens-core/src/reduce/update.rs` (value-carrying enum)
- Modify: `crates/lens-core/src/reduce/items.rs` (`push_item` Arc + payload)
- Modify: `crates/lens-core/src/reduce/{folds,scratch,snapshot,mod}.rs` (thread payloads)
- Modify: `crates/lens-core/src/persist/{control,transcript}.rs` (call-site derefs)
- Test: the existing `#[cfg(test)]` modules across `reduce/` + `domain/session.rs`

**Interfaces:**
- Consumes: existing `reduce(&mut SessionState, &ServerStreamEvent, &dyn Clock) -> Updates`.
- Produces (the new `StreamUpdate`, consumed by `SessionStore::apply` in Task 3):
```rust
pub enum StreamUpdate {
    // transcript deltas (value-carrying)
    ItemAppended(std::sync::Arc<Item>),
    ItemUpdated { index: usize, item: std::sync::Arc<Item> },
    ScratchChanged(std::sync::Arc<StreamScratch>),
    // scalar / collection folds — carry the just-reduced value
    StatusChanged(SessionStatusValue),
    UsageChanged(Cost),                       // cumulative_cost snapshot
    ModelChanged { llm_model: Option<String>, model_override: Option<String> },
    ReasoningEffortChanged(Option<String>),
    CollaborationModeChanged(Option<String>),
    ModelOptionsChanged(Option<Vec<ModelOption>>),
    TodosChanged(Vec<Todo>),
    SkillsChanged(Vec<SkillSummary>),
    SandboxChanged(Option<SandboxStatus>),
    TerminalPendingChanged(bool),
    ElicitationsChanged(Vec<Elicitation>),
    ChildSessionChanged,                      // marker; child topology is §9 (no payload wired here)
    PresenceChanged(Vec<PresenceViewer>),
    ResourcesChanged,                         // marker; no canonical item (D-P1-4)
    AgentChanged { agent_id: AgentId, agent_name: Option<String> },
    TitleChanged(Option<String>),
    LastTokensChanged(Option<u64>),
    ContextWindowChanged(Option<u64>),
    // reconnect / bootstrap lifecycle (passthrough for the UI banner)
    Reconnecting { attempt: u32 },
    Reconnected,
    Disconnected,
    SnapshotRestored,                         // scalar chrome bulk-restore already folded in state
    // D9: once-at-attach full baseline
    Rebased(Box<SessionState>),
}
pub type Updates = SmallVec<[StreamUpdate; 2]>;
```
> **Design note (ratified by Task 4's skeleton) — `ScratchChanged`.** It carries `Arc<StreamScratch>`, a whole-scratch snapshot, NOT a delta. Scratch is the *current* open message/reasoning only (cleared on `Completed`), so it is bounded per turn; large tool dumps arrive as finalized `OutputItemDone` **Items**, never as preview deltas, so scratch stays small. The actor **coalesces** scratch emissions per drain-batch (Task 5), so the `Arc::new(scratch.clone())` cost is one-per-frame, not one-per-token. Reversible to delta-append behind the same `apply` seam if a perf pass demands it.

- [ ] **Step 1: `serde` `rc` feature + `items: Vec<Arc<Item>>`**

`crates/lens-core/Cargo.toml`: ensure `serde = { version = "...", features = ["derive", "rc"] }` (the `rc` feature is required for `Arc<Item>` (de)serialization inside `SessionState`).

`domain/session.rs`:
```rust
use std::sync::Arc;
// ...
pub items: Vec<Arc<Item>>,
```
And in `SessionState::new`, `items: Vec::new(),` is unchanged (empty Vec).

- [ ] **Step 2: Update the `populated_session_roundtrips` test to build `Arc<Item>`**

In `domain/session.rs` tests, wrap the pushed item:
```rust
s.items.push(Arc::new(Item { /* unchanged fields */ }));
```
Run: `cargo test -p lens-core domain::session` → Expected: FAIL to **compile** first (the `reduce`/`persist` call-sites break); that is the signal to proceed to Steps 3–5.

- [ ] **Step 3: `push_item` — Arc wrap + `make_mut` + carry payload**

`reduce/items.rs`:
```rust
use std::sync::Arc;

pub(crate) fn push_item(
    state: &mut SessionState,
    id: ItemId,
    kind: ItemKind,
    seq: Option<u64>,
    clock: &dyn Clock,
) -> Updates {
    let ctx = current_ctx(&state.stream);
    if let Some(idx) = state.items.iter().position(|it| it.id == id) {
        // Arc::make_mut clones iff the body is shared with a replica (rare —
        // updates are far less common than appends); appends stay zero-copy.
        let existing = Arc::make_mut(&mut state.items[idx]);
        existing.kind = kind;
        existing.seq = seq.or(existing.seq);
        smallvec![StreamUpdate::ItemUpdated { index: idx, item: Arc::clone(&state.items[idx]) }]
    } else {
        let item = Arc::new(Item { id, seq, ctx, created_at: clock.now_millis(), kind });
        state.items.push(Arc::clone(&item));
        smallvec![StreamUpdate::ItemAppended(item)]
    }
}
```

- [ ] **Step 4: Thread payloads through every folding site**

Each place that emitted a bare marker now emits the reduced value. Reference map (grep `StreamUpdate::` across `reduce/`):
- `reduce/mod.rs`: `StreamUpdate::StatusChanged` → `StreamUpdate::StatusChanged(state.status.clone())`; `ScratchChanged` (3 sites) → `StreamUpdate::ScratchChanged(Arc::new(state.stream.clone()))`; `ResourcesChanged`/`Reconnecting`/`Reconnected`/`Disconnected`/`SnapshotRestored` keep their (now-defined) shapes.
- `reduce/folds.rs` (`fold_session_field`): each scalar fold emits the field it wrote — e.g. model fold → `ModelChanged { llm_model: state.llm_model.clone(), model_override: state.model_override.clone() }`; todos → `TodosChanged(state.todos.clone())`; usage → `UsageChanged(state.cumulative_cost.clone())` + `LastTokensChanged(state.last_total_tokens)`; presence → `PresenceChanged(state.presence.clone())`; sandbox/effort/elicitations/skills/title/terminal_pending/collaboration_mode/model_options analogously.
- `reduce/items.rs` (`agent_changed`): the `AgentChanged` transcript marker path stays (pushes an item via `push_item`) **and** the session-field `AgentChanged { agent_id, agent_name }` fold emits its payload where `agent_id`/`agent_name` are set.
- `reduce/snapshot.rs` (`fold_snapshot`, `on_reconnected`): these bulk-fold; keep emitting `SnapshotRestored` (scalar chrome already written into `state`). Do **not** emit per-field deltas here (matches the reducer's existing snapshot contract).

> Mechanical rule: **the emitted value is a `.clone()` of the state field the arm just wrote.** No new logic. Where an arm wrote several fields (usage), emit one delta per field group.

- [ ] **Step 5: Fix persist call-site derefs**

`persist/control.rs` + `persist/transcript.rs`: wherever a `&Item` was passed from `state.items` (or an item variable that is now `Arc<Item>`), pass `item.as_ref()` / `&**item`. `upsert_item(ordinal, item: &Item)` and `reconcile(items: &[Item])` signatures are unchanged (disk owns plain `Item`); only the call sites deref the `Arc`. `load_items()`/`load_session()` still return plain `Item`/items-empty — the actor wraps into `Arc` when it builds in-RAM state (Task 5 / Rebased).

- [ ] **Step 6: Update `reduce/` unit + corpus tests for the new payloads**

Wherever a test asserted `&[StreamUpdate::ItemAppended { index: 0 }]` or a bare marker, update to the value-carrying form, e.g.:
```rust
assert!(matches!(&out[..], [StreamUpdate::ItemAppended(it)] if it.id.as_str() == "item_1"));
```
The corpus determinism + `happy_path` shape tests read `state.items` via `Arc` deref — no change beyond the `matches!` payload updates. Add one focused test:
```rust
#[test]
fn appended_delta_carries_the_same_arc_body_as_state() {
    // reduce an OutputItemDone; the emitted ItemAppended's Arc must be
    // pointer-equal to the one now in state.items (bodies are shared).
    let (mut s, ev, clock) = one_output_item_done_fixture();
    let out = reduce(&mut s, &ev, &clock);
    let StreamUpdate::ItemAppended(arc) = &out[0] else { panic!("expected ItemAppended") };
    assert!(std::sync::Arc::ptr_eq(arc, s.items.last().unwrap()));
}
```

- [ ] **Step 7: Gate**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`
Expected: fmt clean, 0 warnings, all P1/P2 tests pass with the new payloads. Confirm the P1 corpus determinism test still passes (replay twice → identical state) and the `reduce` micro-bench still runs (`cargo bench -p lens-core` if the bench harness exists — value-carrying clones must not blow the ~1.36µs/turn budget by more than the added `.clone()`s; note the new number).

- [ ] **Step 8: Commit**

```bash
git add crates/lens-core/Cargo.toml crates/lens-core/src/domain/session.rs crates/lens-core/src/reduce crates/lens-core/src/persist
git commit -m "refactor(lens-core): value-carrying StreamUpdate + items: Vec<Arc<Item>> + Rebased (D8/D9)"
```

---

## Task 3: `lens-store` crate + `SessionStore` replica + `apply`

The foreground gpui replica: an `Entity<SessionState>` wrapper whose only mutation path is `apply(StreamUpdate)` = **pure copy-assignment** (deposit the already-reduced value into the named field; never re-derive, never run `reduce`). O(1) per delta (item bodies are `Arc`).

**Files:**
- Create: `crates/lens-store/Cargo.toml`
- Create: `crates/lens-store/src/lib.rs`
- Test: `crates/lens-store/src/lib.rs` (`#[cfg(test)]` with `#[gpui::test]`)

**Interfaces:**
- Consumes: `lens_core::reduce::StreamUpdate` (Task 2), `lens_core::domain::SessionState`.
- Produces:
  - `SessionStore { entity: Entity<SessionState> }` with `SessionStore::new(cx, initial: SessionState) -> Self`, `read(cx) -> &SessionState`, `entity() -> &Entity<SessionState>`.
  - `fn apply(state: &mut SessionState, update: StreamUpdate)` — the pure copy-assignment fn (free fn so it is unit-testable without gpui, and callable from inside `entity.update`).

- [ ] **Step 1: Crate manifest**

`crates/lens-store/Cargo.toml`:
```toml
[package]
name = "lens-store"
version = "0.1.0"
edition = "2024"
rust-version = "1.91"
license = "MIT"
authors = ["Amogh Akshintala"]

[dependencies]
gpui = "0.2.2"
lens-core = { path = "../lens-core" }
async-channel = "2"

[lints]
workspace = true
```
Run: `cargo build -p lens-store` → Expected: compiles (empty lib).

- [ ] **Step 2: Failing test — `apply` copy-assignment (no gpui needed)**

`crates/lens-store/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::{AgentId, ConnectionId, SessionId, SessionState};
    use lens_core::reduce::StreamUpdate;
    use lens_core::domain::scalars::SessionStatusValue;

    fn state() -> SessionState {
        SessionState::new(ConnectionId::new("c"), SessionId::new("conv"), AgentId::new("ag"))
    }

    #[test]
    fn apply_status_is_copy_assignment() {
        let mut s = state();
        apply(&mut s, StreamUpdate::StatusChanged(SessionStatusValue::Running));
        assert_eq!(s.status, SessionStatusValue::Running);
    }

    #[test]
    fn apply_item_appended_pushes_shared_body() {
        let mut s = state();
        let item = std::sync::Arc::new(sample_item("item_1")); // helper builds an Item
        apply(&mut s, StreamUpdate::ItemAppended(std::sync::Arc::clone(&item)));
        assert_eq!(s.items.len(), 1);
        assert!(std::sync::Arc::ptr_eq(&s.items[0], &item));
    }

    #[test]
    fn apply_rebased_replaces_whole_state() {
        let mut s = state();
        let mut baseline = state();
        baseline.title = Some("rebased".into());
        apply(&mut s, StreamUpdate::Rebased(Box::new(baseline)));
        assert_eq!(s.title.as_deref(), Some("rebased"));
    }
}
```
Run: `cargo test -p lens-store` → Expected: FAIL (`apply` not defined).

- [ ] **Step 3: Implement `apply`**

`crates/lens-store/src/lib.rs`:
```rust
use lens_core::domain::SessionState;
use lens_core::reduce::StreamUpdate;

/// Deposit an already-reduced delta into the replica by pure copy-assignment.
/// NEVER re-derives, NEVER runs `reduce`, NEVER does I/O. O(1) per delta
/// (item bodies are `Arc`, so appends/updates move a pointer). (spec D8)
pub fn apply(state: &mut SessionState, update: StreamUpdate) {
    use StreamUpdate::*;
    match update {
        ItemAppended(item) => state.items.push(item),
        ItemUpdated { index, item } => {
            if index < state.items.len() {
                state.items[index] = item;
            }
        }
        ScratchChanged(scratch) => state.stream = (*scratch).clone(),
        StatusChanged(v) => state.status = v,
        UsageChanged(c) => state.cumulative_cost = c,
        ModelChanged { llm_model, model_override } => {
            state.llm_model = llm_model;
            state.model_override = model_override;
        }
        ReasoningEffortChanged(v) => state.reasoning_effort = v,
        CollaborationModeChanged(v) => state.collaboration_mode = v,
        ModelOptionsChanged(v) => state.model_options = v,
        TodosChanged(v) => state.todos = v,
        SkillsChanged(v) => state.skills = v,
        SandboxChanged(v) => state.sandbox_status = v,
        TerminalPendingChanged(v) => state.terminal_pending = v,
        ElicitationsChanged(v) => state.pending_elicitations = v,
        PresenceChanged(v) => state.presence = v,
        AgentChanged { agent_id, agent_name } => {
            state.agent_id = agent_id;
            state.agent_name = agent_name;
        }
        TitleChanged(v) => state.title = v,
        LastTokensChanged(v) => state.last_total_tokens = v,
        ContextWindowChanged(v) => state.context_window = v,
        Rebased(baseline) => *state = *baseline,
        // markers with no replica-visible payload in P3-1
        ChildSessionChanged | ResourcesChanged | SnapshotRestored
        | Reconnecting { .. } | Reconnected | Disconnected => {}
    }
}
```
> Match must be **exhaustive** (no `_ =>`) so a future `StreamUpdate` variant forces a compile error here — the seam stays honest. Lifecycle markers (`Reconnecting`/`Reconnected`/`Disconnected`/`SnapshotRestored`) are consumed by a UI banner subscriber in a later plan; here they are `apply`-noops (state chrome was already folded actor-side).

- [ ] **Step 4: `SessionStore` gpui wrapper + a `#[gpui::test]`**

```rust
use gpui::{App, Context, Entity};

pub struct SessionStore {
    entity: Entity<SessionState>,
}

impl SessionStore {
    pub fn new(cx: &mut App, initial: SessionState) -> Self {
        Self { entity: cx.new(|_cx| initial) }
    }
    pub fn entity(&self) -> &Entity<SessionState> {
        &self.entity
    }
    pub fn read<'a>(&self, cx: &'a App) -> &'a SessionState {
        self.entity.read(cx)
    }
    /// Apply one delta on the foreground and notify observers. Called by the
    /// drain bridge (Task 4).
    pub fn apply_on(&self, cx: &mut App, update: StreamUpdate) {
        self.entity.update(cx, |state, cx| {
            apply(state, update);
            cx.notify();
        });
    }
}
```
Test (verify harness calls against `gpui-0.2.2/src/app/test_context.rs`):
```rust
#[gpui::test]
fn store_applies_and_reads_back(cx: &mut gpui::TestAppContext) {
    let store = cx.update(|cx| SessionStore::new(cx, state()));
    cx.update(|cx| store.apply_on(cx, StreamUpdate::StatusChanged(SessionStatusValue::Running)));
    let status = cx.read(|cx| store.read(cx).status.clone());
    assert_eq!(status, SessionStatusValue::Running);
}
```

- [ ] **Step 5: Gate + commit**

Run: `cargo fmt -p lens-store && cargo clippy -p lens-store --all-targets && cargo test -p lens-store`
Expected: all pass.
```bash
git add crates/lens-store Cargo.toml
git commit -m "feat(lens-store): SessionStore replica + pure copy-assignment apply (D8)"
```

---

## Task 4: Walking skeleton — off-thread → foreground bridge (D7)

Prove the full seam with a **fake event**: a plain OS thread reduces a scripted event into `SessionState`, emits a `Rebased` baseline then a `StreamUpdate` over an `async-channel`; a foreground `cx.spawn` task drains (greedy-coalesced) and applies into the `SessionStore`, then `cx.notify()`; a `run_until_parked` observes the result. This ratifies the value-carrying-delta + `Arc<Item>` representation + the blocking-thread↔`cx.spawn` handoff end-to-end (spec §4 P3 Task 1).

**Files:**
- Modify: `crates/lens-store/src/lib.rs` (add `spawn_apply_bridge`)
- Test: `crates/lens-store/src/lib.rs` (`#[gpui::test]`)

**Interfaces:**
- Consumes: `SessionStore` (Task 3), `StreamUpdate` (Task 2).
- Produces:
  - `pub fn spawn_apply_bridge(store: SessionStore, rx: async_channel::Receiver<StreamUpdate>, cx: &mut App) -> gpui::Task<()>` — the foreground drain loop: `recv().await` one, greedily `try_recv()` the backlog, apply the whole batch under **one** `entity.update` + `cx.notify()`, loop until the channel closes.

- [ ] **Step 1: Failing integration test**

```rust
#[gpui::test]
async fn skeleton_off_thread_event_reaches_foreground(cx: &mut gpui::TestAppContext) {
    use lens_core::domain::scalars::SessionStatusValue;
    let (tx, rx) = async_channel::bounded::<StreamUpdate>(1024);

    let store = cx.update(|cx| SessionStore::new(cx, state()));
    let entity = store.entity().clone();
    let _bridge = cx.update(|cx| spawn_apply_bridge(store, rx, cx));

    // A plain OS thread stands in for the actor: emit a Rebased baseline, then a delta.
    std::thread::spawn(move || {
        let mut s = state();
        s.title = Some("baseline".into());
        tx.send_blocking(StreamUpdate::Rebased(Box::new(s))).unwrap();
        tx.send_blocking(StreamUpdate::StatusChanged(SessionStatusValue::Running)).unwrap();
        // drop(tx) closes the channel and ends the bridge loop.
    });

    cx.run_until_parked();
    let (title, status) = cx.read(|cx| {
        let s = entity.read(cx);
        (s.title.clone(), s.status.clone())
    });
    assert_eq!(title.as_deref(), Some("baseline"), "Rebased baseline applied");
    assert_eq!(status, SessionStatusValue::Running, "delta applied after baseline");
}
```
Run: `cargo test -p lens-store skeleton` → Expected: FAIL (`spawn_apply_bridge` not defined).

- [ ] **Step 2: Implement the drain bridge**

```rust
use gpui::Task;

/// Foreground drain: event-driven wakeup (`recv().await`) + greedy `try_recv`
/// coalescing so a burst of deltas costs ONE `cx.notify()`/frame. Ends when the
/// actor drops its sender (channel closed). Detach the returned Task to run it.
pub fn spawn_apply_bridge(
    store: SessionStore,
    rx: async_channel::Receiver<StreamUpdate>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        while let Ok(first) = rx.recv().await {
            let mut batch = smallvec::SmallVec::<[StreamUpdate; 8]>::new();
            batch.push(first);
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            let applied = store.entity().update(cx, |state, cx| {
                for u in batch.drain(..) {
                    apply(state, u);
                }
                cx.notify();
            });
            if applied.is_err() {
                break; // replica entity released — nothing to update
            }
        }
    })
}
```
> `cx.spawn`'s closure signature in gpui 0.2.2 is `async move |cx: &mut AsyncApp|` (single arg — the `this`-carrying form is `Entity::update` inside). Verify the exact arity against `gpui-0.2.2/src/app/context.rs:237`; the two-arg `|this, cx|` form seen in the markdown spike is `Context::spawn` on an entity — here we hold `store` by move and call `store.entity().update(cx, ...)`, which returns `Result` (Err ⇒ entity released ⇒ break). `smallvec` is already a transitive dep via lens-core; add it to `lens-store` Cargo.toml if the import fails.

- [ ] **Step 3: Run the test**

Run: `cargo test -p lens-store skeleton -- --nocapture`
Expected: PASS. Both assertions hold; the test does not hang (channel close ends the loop).

- [ ] **Step 4: Add a coalescing assertion**

```rust
#[gpui::test]
async fn skeleton_coalesces_a_burst_into_few_notifies(cx: &mut gpui::TestAppContext) {
    let (tx, rx) = async_channel::bounded::<StreamUpdate>(1024);
    let store = cx.update(|cx| SessionStore::new(cx, state()));
    let entity = store.entity().clone();
    // Observe notify count.
    let notifies = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let n2 = notifies.clone();
    let _sub = cx.update(|cx| cx.observe(&entity, move |_, _| n2.set(n2.get() + 1)));
    let _bridge = cx.update(|cx| spawn_apply_bridge(store, rx, cx));

    std::thread::spawn(move || {
        for i in 0..500u64 {
            tx.send_blocking(StreamUpdate::LastTokensChanged(Some(i))).unwrap();
        }
    });
    cx.run_until_parked();
    let last = cx.read(|cx| entity.read(cx).last_total_tokens);
    assert_eq!(last, Some(499));
    assert!(notifies.get() < 500, "500 deltas coalesced into {} notifies", notifies.get());
}
```
Run: `cargo test -p lens-store skeleton_coalesces` → Expected: PASS (final value correct; notifies ≪ 500).

- [ ] **Step 5: Gate + commit**

Run: `cargo fmt -p lens-store && cargo clippy -p lens-store --all-targets && cargo test -p lens-store`
```bash
git add crates/lens-store
git commit -m "feat(lens-store): walking-skeleton apply bridge — off-thread event to foreground replica (D7)"
```

---

## Task 5: Actor run-loop — `crossbeam::Select` ingest + persist write-through (D13, spec §4 P3(a))

Replace the fake thread with the real `ActiveSession` actor in `lens-core`: an OS thread that `Select`s over the lens-client event receiver (Task 1) + a `SessionCommand` receiver, and per event does `reduce → persist write-through → emit`. Commands beyond `Stop` are P3-2; here `Stop` proves the Select + graceful shutdown. Tests drive it with scripted crossbeam senders (no server) — reuse the `Reopen`-style determinism.

**Files:**
- Create: `crates/lens-core/src/actor/mod.rs`
- Create: `crates/lens-core/src/actor/runloop.rs`
- Modify: `crates/lens-core/src/lib.rs` (`pub mod actor;`)
- Modify: `crates/lens-core/Cargo.toml` (`crossbeam-channel`, `async-channel`)
- Test: `crates/lens-core/src/actor/runloop.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `crossbeam_channel::Receiver<ServerStreamEvent>` (from `EventStream::receiver()`, Task 1), `lens_core::reduce::reduce`, `lens_core::persist::{ControlStore, TranscriptStore}`, `async_channel::Sender<StreamUpdate>` (to the bridge, Task 4).
- Produces:
```rust
pub enum SessionCommand { Stop }             // extended in P3-2

pub struct ActiveSession { /* thread-owned */ }

pub struct ActorHandle {
    pub commands: crossbeam_channel::Sender<SessionCommand>,
    join: std::thread::JoinHandle<()>,
}
impl ActorHandle {
    pub fn stop_and_join(self);              // send Stop, join the thread
}

/// Spawn the actor thread. `events` is the crossbeam receiver from lens-client;
/// `updates` is the async-channel sender to the foreground bridge.
pub fn spawn_actor(
    mut state: SessionState,
    events: crossbeam_channel::Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    stores: ActorStores,     // { control: Box<dyn ControlStore>, transcript: Box<dyn TranscriptStore> }
    clock: std::sync::Arc<dyn Clock>,
) -> ActorHandle;
```

- [ ] **Step 1: Deps + module wiring**

`crates/lens-core/Cargo.toml` `[dependencies]`: add `crossbeam-channel = "0.5"` and `async-channel = "2"`.
`crates/lens-core/src/lib.rs`: add `pub mod actor;`.
`actor/mod.rs`: declare `mod runloop;` and re-export `SessionCommand`, `ActorHandle`, `ActorStores`, `spawn_actor`, `OutputMode`.

- [ ] **Step 2: Failing test — an event reduces, persists, and emits**

`actor/runloop.rs` tests (use `MemoryStores` or the SQLite stores over a `tempfile` dir — mirror `persist/transcript.rs` tests):
```rust
#[test]
fn actor_reduces_persists_and_emits_on_event() {
    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, up_rx) = async_channel::bounded(64);
    let stores = test_stores(); // ControlStore + TranscriptStore over a tempdir
    let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores.clone_boxed(), test_clock());

    ev_tx.send(one_output_item_done_event()).unwrap(); // a scripted OutputItemDone
    // The emit is async; block-recv the first update with a timeout.
    let update = up_rx.recv_blocking().expect("actor emitted an update");
    assert!(matches!(update, StreamUpdate::ItemAppended(_)));

    handle.stop_and_join();
    // Persist write-through: the item is on disk.
    let loaded = stores.transcript.load_items().unwrap();
    assert_eq!(loaded.rows.len(), 1);
}
```
Run: `cargo test -p lens-core actor::runloop` → Expected: FAIL (`spawn_actor` not defined).

- [ ] **Step 3: The `Select` run-loop**

`actor/runloop.rs`:
```rust
use crossbeam_channel::{Receiver, Select};

pub fn spawn_actor(
    mut state: SessionState,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    mut stores: ActorStores,
    clock: std::sync::Arc<dyn Clock>,
) -> ActorHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<SessionCommand>(64);
    let join = std::thread::Builder::new()
        .name("lens-actor".into())
        .spawn(move || run(state, events, cmd_rx, updates, stores, clock.as_ref()))
        .expect("actor thread");
    ActorHandle { commands: cmd_tx, join }
}

fn run(
    mut state: SessionState,
    events: Receiver<ServerStreamEvent>,
    commands: Receiver<SessionCommand>,
    updates: async_channel::Sender<StreamUpdate>,
    mut stores: ActorStores,
    clock: &dyn Clock,
) {
    loop {
        let mut sel = Select::new();
        let ev_idx = sel.recv(&events);
        let cmd_idx = sel.recv(&commands);
        let oper = sel.select();
        match oper.index() {
            i if i == cmd_idx => match oper.recv(&commands) {
                Ok(SessionCommand::Stop) | Err(_) => break, // Stop or command sender dropped
            },
            i if i == ev_idx => match oper.recv(&events) {
                Ok(event) => {
                    // Greedy-drain any queued events this wakeup, reduce each,
                    // collect + coalesce the resulting deltas, then send.
                    let mut batch = reduce(&mut state, &event, clock);
                    while let Ok(next) = events.try_recv() {
                        batch.extend(reduce(&mut state, &next, clock));
                    }
                    persist_write_through(&mut stores, &state, &batch);
                    for u in coalesce(batch) {
                        if updates.send_blocking(u).is_err() {
                            return; // foreground bridge gone
                        }
                    }
                }
                Err(_) => break, // event stream closed (reader thread exited)
            },
            _ => unreachable!(),
        }
    }
}
```

- [ ] **Step 4: `persist_write_through` + `coalesce`**

```rust
/// Write the deltas of this batch to disk. Items → TranscriptStore by ordinal;
/// a scalar/collection change → one control upsert of the whole session row.
fn persist_write_through(stores: &mut ActorStores, state: &SessionState, batch: &[StreamUpdate]) {
    let mut touched_scalar = false;
    for u in batch {
        match u {
            StreamUpdate::ItemAppended(item) => {
                let ordinal = (state.items.len() as i64) - 1; // appended at tail
                let _ = stores.transcript.upsert_item(ordinal, item.as_ref());
            }
            StreamUpdate::ItemUpdated { index, item } => {
                let _ = stores.transcript.upsert_item(*index as i64, item.as_ref());
            }
            StreamUpdate::ScratchChanged(_) | StreamUpdate::Reconnecting { .. }
            | StreamUpdate::Reconnected | StreamUpdate::Disconnected => {}
            _ => touched_scalar = true,
        }
    }
    if touched_scalar {
        let _ = stores.control.upsert_session(state, clock_now_ms_placeholder(state));
    }
}

/// Drop superseded scratch/scalar deltas within one batch (keep the last of each
/// kind); item deltas always survive (order-significant transcript growth).
fn coalesce(batch: Updates) -> Updates { /* keep items in order; last-wins per scalar kind */ }
```
> `upsert_item` ordinal: use a monotonic per-session counter the actor owns (initialize from `load_items().rows.len()` at attach) rather than `state.items.len()` (which is a byte-windowed tail post-D11) — for P3-1 the tail == full history, so `state.items.len()-1` is correct; add a `TODO(P3-3)` that wake/eviction replaces this with the owned ordinal cursor. Errors from persist are swallowed→introspection-ring in P3-2/§13.1; for P3-1 log-and-continue (never panic, never block the emit).

- [ ] **Step 5: `Stop` graceful-shutdown test**

```rust
#[test]
fn actor_stops_on_command_even_while_idle() {
    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);   // no events ever
    let (up_tx, _up_rx) = async_channel::bounded(64);
    let handle = spawn_actor(fresh_state(), ev_rx, up_tx, test_stores().clone_boxed(), test_clock());
    // Select must service the command channel even with the event channel silent.
    handle.stop_and_join(); // must return promptly, not hang
}
```

- [ ] **Step 6: Gate + commit + review**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`
Expected: all pass; the `Stop`-while-idle test returns promptly (proves `Select` services commands during event silence).
```bash
git add crates/lens-core/Cargo.toml crates/lens-core/src/lib.rs crates/lens-core/src/actor
git commit -m "feat(lens-core): ActiveSession actor — crossbeam Select ingest + persist write-through (D13)"
```
**MANDATORY cross-family review** of the run-loop diff (temporal/stateful — the Select bookkeeping, greedy drain, backpressure `send_blocking`, and shutdown paths).

---

## Task 6: Dual-mode `Detailed | Summary` + promote/demote (D10)

A full replica (items + `Detailed` deltas) exists **only for focused sessions**; background-warm Active sessions get a coarse `SummaryUpdate` feed from the actor. The actor supports two output modes; **promote** (focus) emits a `Rebased` baseline then `Detailed`; **demote** (blur) drops items and reverts to `Summary`. This plan builds the **capability + the promote/demote primitive**; the trigger policy (focus set / active-set LRU) is the §9 registry (seam only).

**Files:**
- Create: `crates/lens-core/src/actor/summary.rs`
- Modify: `crates/lens-core/src/actor/{mod,runloop}.rs`
- Test: `crates/lens-core/src/actor/runloop.rs`

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Copy, PartialEq, Eq)] pub enum OutputMode { Detailed, Summary }

/// Coarse card-summary — a type DISTINCT from StreamUpdate (spec §6). Two
/// producers (actor here; §10 poll later). apply = copy-assignment of scalars.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryUpdate {
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub last_total_tokens: Option<u64>,
    pub host_id: Option<HostId>,
    pub needs_attention: bool,           // pending elicitation OR error status
    pub subagent_active: bool,           // any child session running
}
impl SummaryUpdate { pub fn from_state(s: &SessionState) -> Self; }

// SessionCommand gains promote/demote:
pub enum SessionCommand { Stop, Promote, Demote }
```
The actor holds `mode: OutputMode` and a second `async_channel::Sender<SummaryUpdate>`. In `Summary` mode it emits a `SummaryUpdate` (coalesced, ms–s cadence) instead of the full `Detailed` batch; on `Promote` it sends `StreamUpdate::Rebased(Box::new(state.clone()))` then switches to `Detailed`; on `Demote` it switches to `Summary` (the foreground drops the full items when it tears down the `Detailed` bridge — the actor keeps canonical state).

- [ ] **Step 1: `SummaryUpdate::from_state` failing test**

```rust
#[test]
fn summary_flags_needs_attention_on_pending_elicitation() {
    let mut s = fresh_state();
    s.pending_elicitations.push(sample_elicitation());
    let sum = SummaryUpdate::from_state(&s);
    assert!(sum.needs_attention);
}
```
Run → Expected: FAIL (`from_state` not defined). Implement `from_state` (pure projection). Run → PASS.

- [ ] **Step 2: Mode-switch test — Summary emits SummaryUpdate, Promote emits Rebased**

```rust
#[test]
fn summary_mode_emits_summary_not_detailed_then_promote_rebases() {
    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, up_rx) = async_channel::bounded(64);
    let (sum_tx, sum_rx) = async_channel::bounded(64);
    // Spawn in Summary mode (background-warm).
    let handle = spawn_actor_dual(fresh_state(), ev_rx, up_tx, sum_tx,
        OutputMode::Summary, test_stores().clone_boxed(), test_clock());

    ev_tx.send(status_running_event()).unwrap();
    assert!(matches!(sum_rx.recv_blocking().unwrap(), SummaryUpdate { .. })); // coarse feed
    assert!(up_rx.try_recv().is_err(), "no Detailed deltas in Summary mode");

    handle.commands.send(SessionCommand::Promote).unwrap();
    assert!(matches!(up_rx.recv_blocking().unwrap(), StreamUpdate::Rebased(_))); // baseline on promote
    handle.stop_and_join();
}
```
Run → Expected: FAIL. Implement the mode branch in `run` (Step 3). Run → PASS.

- [ ] **Step 3: Wire mode into the run-loop**

Extend `run` to hold `mode` + both senders; in the event arm, `match mode { Detailed => send StreamUpdate batch, Summary => send one coalesced SummaryUpdate::from_state(&state) }`; add `Promote`/`Demote` command arms (Promote: send `Rebased`, set `Detailed`; Demote: set `Summary`). Keep the `Summary` emit coalesced (last-wins within a drain-batch).

- [ ] **Step 4: Gate + commit**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`
```bash
git add crates/lens-core/src/actor
git commit -m "feat(lens-core): actor dual-mode Detailed|Summary + promote/demote primitive (D10)"
```

---

## Task 7: Foreground apply micro-bench + end-to-end skeleton gate

Close the spec §5 perf contract: `StreamUpdate::apply` is O(1) and the off-thread→foreground path carries no foreground blocking.

**Files:**
- Create: `crates/lens-store/benches/apply.rs` (criterion, `bench` feature — mirror lens-client's `benches/` + `bench` feature gate)
- Modify: `crates/lens-store/Cargo.toml`

- [ ] **Step 1: Bench `apply`**

Bench applying `ItemAppended` (Arc push) and `StatusChanged` (scalar) over a state with 10k resident items; assert (by inspection of the number, recorded in the commit message) that per-`apply` cost is flat / ns-scale and independent of transcript length (Arc push is O(1) amortized).

- [ ] **Step 2: Run + record**

Run: `cargo bench -p lens-store --features bench`
Record the number in the commit message and in STATUS. Expected: sub-µs/apply, flat vs item count.

- [ ] **Step 3: Commit**

```bash
git add crates/lens-store
git commit -m "bench(lens-store): StreamUpdate::apply is O(1) — foreground apply-path perf gate"
```

---

## Self-Review Checklist (run before handoff)

**1. Spec coverage (§2.1/§4 P3 foundation subset):**
- D7 walking skeleton → Task 4. D8 value-carrying + `Vec<Arc<Item>>` + `apply` → Tasks 2–3. D9 `Rebased` at attach → Tasks 2/3/4. D10 dual-mode + promote/demote → Task 6. D13 crossbeam `Select` ingest + lens-client channel swap → Tasks 1/5. §5 perf → Task 7.
- **Explicitly out of scope (later plans):** D11 byte-windowed eviction + paged load (P3-3, needs the Task 0 spike's `reconcile` contract); D12 spike (running separately); D16 command semantics/optimistic-send reconcile (P3-2); D17 quiesce/sleep/wake (P3-3); D18 §13.1 error mapping (P3-2). The actor run-loop's persist write-through uses `state.items.len()` for the ordinal with a `TODO(P3-3)` where the owned ordinal cursor + eviction land.

**2. Placeholder scan:** the `coalesce` body (Task 5 Step 4) and the ordinal cursor are the two spots with prose over code — the coalesce contract is stated (keep items in order, last-wins per scalar kind); the executor writes the ~10-line body. `clock_now_ms_placeholder` must be replaced with the injected clock's `now_millis()` threaded into `persist_write_through`. Flagged, not hidden.

**3. Type consistency:** `StreamUpdate` variants defined in Task 2 are matched exhaustively in `apply` (Task 3) and produced in `reduce` (Task 2) + the run-loop (Task 5). `SessionCommand` grows `Stop` (Task 5) → `Promote`/`Demote` (Task 6). `EventStream::receiver()` (Task 1) is the input to `spawn_actor` (Task 5). `async_channel::Sender<StreamUpdate>` connects Task 5's actor to Task 4's bridge.

**4. Open verification points for the executor** (verify against the cited source, don't guess):
- `cx.spawn` closure arity in gpui 0.2.2 (`gpui-0.2.2/src/app/context.rs:237` vs `app.rs:1417`) — Task 4 Step 2.
- `#[gpui::test]` async harness + `run_until_parked` + `cx.new`/`cx.update`/`cx.read` (`gpui-0.2.2/src/app/test_context.rs`) — Tasks 3/4.
- `crossbeam_channel::Select` `recv`/`select`/`oper.recv` API (0.5) — Task 5.
