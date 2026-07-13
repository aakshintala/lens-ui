# State-Model Engine P1 — Pure Reducer + Render Transforms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Cross-family plan review APPLIED (2026-07-08, codex/gpt-5.5, verdict NEEDS-REVISION → resolved).**
> 12 findings, all addressed and marked `REVIEW#n` in-code: #1 turn-bump ordering (finalize
> before bump), #2 collision-free synthetic ids (`items.len()`-derived, not clock), #3 `map_item`
> returns `Option` (resources → no item), #4 `response.error` → `last_task_error` scalar (not a
> double-inserting item), #5 match `message_id` before clearing the preview, #6 reconnect keeps
> `turn`/`current_agent`, #7 `current_agent` from completed `FunctionCall`, #8 `TodoStatus::Unknown`
> passthrough, #9 `session.created` → `ChildSessionChanged`, #10 `test-util` `decode_all` seam
> (Task 1b), #11 `collaboration_mode` guard test, #12 `unpaired_calls` left empty.

**Goal:** Build the pure canonical reducer `reduce(&mut SessionState, &ServerStreamEvent, &dyn Clock) -> SmallVec<[StreamUpdate; 2]>` plus the §4.3 render transforms in `lens-core/reduce` — the *contract-proving* phase that folds every modeled `lens_client::stream::ServerStreamEvent` into the P0 domain, TDD against the golden SSE corpus, deterministic under an injected clock.

**Architecture:** `reduce()` is a pure function: it mutates the `SessionState` it is handed and returns *semantic deltas* (`StreamUpdate` markers) for the future actor/replica to act on. No threads, no gpui, no SQLite, no wall-clock read. In-progress text/reasoning accumulate in `StreamScratch` (RAM-only); finalized units become durable `Item`s keyed by `id`. Session-field events fold into `SessionState` scalars/collections. The reconnect/bootstrap synthetics (`Reconnected`/`SnapshotRestored`) fold snapshot **scalars only** — no transcript side-effects.

**Tech Stack:** Rust (edition 2024, workspace `rust-version = 1.91`), `serde`/`serde_json`, `smallvec` (new dep, for the `SmallVec<[StreamUpdate; 2]>` return), `lens-client` (path dep — the typed `stream::ServerStreamEvent` input), `criterion` (new dev-dep, reducer-throughput bench per AGENTS.md benchmark-or-it's-not-done).

## Global Constraints

- **Design source of truth:** `docs/design/app-architecture-and-state-model.md` §4 (LOCKED) + spec `docs/specs/2026-07-08-state-model-engine-design.md` §4 "P1".
- **`lens-core` has NO gpui dependency.** No threads, no SQLite in P1 — pure logic only.
- **`reduce()` is pure and deterministic** (§4.1 "does no I/O"): every timestamp comes from the injected `Clock`, never a direct wall-clock read. Replay the same event sequence under the same clock twice ⇒ **identical** `SessionState`. This is the P1 gate.
- **The UI never panics** (AGENTS.md MANDATORY): the reducer is total over every `ServerStreamEvent` arm; unmodeled/degraded shapes fold to a marker or a catch-all item, never `panic!`/`unwrap` on external data.
- **Ground-truth discipline:** wire-event arms are TDD'd against captured bytes (`docs/spikes/captures/2026-06-26-sse/` + `…-live-recapture/`). The reconnect/bootstrap arms are crate-synthetic (not in the wire corpus) → hand-authored synthetic-event fixtures.
- **Production lint bar:** `lints.workspace = true`. Zero warnings.
- **`generated.rs` in `lens-client` is untouched.**
- **Gate every task:** `cargo test -p lens-core` · `cargo clippy -p lens-core --all-targets` (zero warnings) · `cargo fmt --check`.
- **`StreamUpdate` is DRAFTED here, ratified at the P3 skeleton (spec D6).** P1 freezes the **semantic deltas** (which event ⇒ which markers). The apply-side payload representation may still refine at P3 — so P1 emits markers and asserts them; it does **not** build an `apply()` (no replica exists yet).

---

## File Structure

```
crates/lens-core/
  Cargo.toml                 # MODIFY — add smallvec dep + criterion dev-dep + [[bench]]
  benches/
    reduce_throughput.rs     # NEW — criterion bench: full-corpus replay throughput
  src/
    lib.rs                   # MODIFY — `pub mod clock; pub mod reduce;` + re-exports
    clock.rs                 # NEW — Clock trait + ManualClock test double (the injected-time seam)
    domain/
      session.rs             # MODIFY — add `terminal_pending: bool` field (§4.1 fold has no P0 home)
    reduce/
      mod.rs                 # NEW — reduce() top-level dispatch + StreamUpdate enum
      update.rs              # NEW — StreamUpdate enum + its unit tests
      scratch.rs             # NEW — text + reasoning accumulation over StreamScratch
      items.rs               # NEW — wire stream::Item -> domain ItemKind; dedup-by-id; ctx stamping
      folds.rs               # NEW — session-field folds + status/usage normalization
      snapshot.rs            # NEW — SnapshotRestored / Reconnected / lifecycle folds
      transforms.rs          # NEW — §4.3 pure render transforms over &[Item]
```

Root workspace `Cargo.toml` globs `members = ["crates/*"]` — no members edit.

---

## Decisions (contract-proving findings — REVIEW THESE FIRST)

P1 is the phase where the domain contract meets real bytes. The following are judgment
calls made against the actual `lens_client::stream` shapes; **cross-family review of this
plan is REQUIRED before build** (the P0 precedent caught a real blocker at exactly this
gate). Each is flagged in-code with a `// P1-DECISION` comment.

- **D-P1-1 — `reduce` signature + `Clock` seam.**
  `pub fn reduce(state: &mut SessionState, event: &ServerStreamEvent, clock: &dyn Clock) -> SmallVec<[StreamUpdate; 2]>`.
  `Clock { fn now_millis(&self) -> i64 }` lives in `lens-core::clock` with a `ManualClock`
  test double. Determinism (the gate) is impossible without injecting time — the reducer
  never calls `SystemTime::now()`.

- **D-P1-2 — `StreamUpdate` = marker-style draft (spec D6).** Variants name the changed
  field-group / transcript delta, not a payload. There is **no `apply()` in P1** (no replica
  exists); tests assert the reducer *emits the right markers*. The apply-side payload
  representation is ratified at the P3 skeleton.

- **D-P1-3 — Item-mapping gap (the headline finding).** `lens_client::stream::Item` models
  only 5 concrete variants + `Other { item_type, id }`; domain `ItemKind` has 11. Mapping:
  - **Faithful:** `Message`, `FunctionCall`, `FunctionCallOutput`, `Error`.
  - **Reducer-synthesized (not from `OutputItemDone`):** `Reasoning` (bracket close),
    `Compaction` (from `response.compaction.completed`), `AgentChanged` (from
    `session.agent_changed`).
  - **Under-fed by the current wrapper → catch-all:** `Item::Other { item_type, id }` maps
    to `ItemKind::NativeTool { tool_type: item_type, data: Value::Null }` so no transcript
    item is silently dropped. Full fidelity for `native_tool` payload, `slash_command`, and
    `terminal_command` is **deferred** until `lens-client` widens `stream::Item` (a P1
    handoff blocker of the same class as PresenceViewer — recorded in the handoff notes).

- **D-P1-4 — `ItemKind::ResourceEvent` NOT materialized at P1.** The live stream carries
  resources as `SessionEvent::ResourceCreated` (payload dropped by the lens-client wrapper —
  it is a unit variant) / `ResourceDeleted { resource_id, resource_type }`, and `/items`
  carries `stream::Item::ResourceEvent { resource_id, resource_type, event_type }` — **none**
  provides the `SessionResourceObject` the domain `ItemKind::ResourceEvent` wraps. So P1
  folds resource events to a coarse `StreamUpdate::ResourcesChanged` marker only (no item,
  no scalar). Materializing resource items is deferred until the wrapper is widened. Flag.

- **D-P1-5 — Presence fills `user_id` only.** `lens_client::stream::PresenceViewer` exposes
  only `user_id()`; `joined_at`/`idle` were dropped by the wrapper (P0 handoff blocker). The
  reducer fills domain `PresenceViewer { user_id, joined_at: String::new(), idle: false }`
  and flags it. Resolve by widening the lens-client stream wrapper (or reading
  `lens_client::generated::PresenceViewer`). Viewers with no `user_id` are skipped.

- **D-P1-6 — `FunctionCall` field cleanup.** Wire `agent` is the `resp_…` id while
  `status == "in_progress"`, the agent *name* once `completed` (documented wire wart). The
  reducer sets `agent_name = agent.filter(|_| status == "completed")`. `arguments` (a raw
  JSON *string* on the wire — the state model owns parsing, §2.3) is parsed to `Value`;
  on parse failure it falls back to `Value::String(raw)` (never dropped).

- **D-P1-7 — `FunctionCallOutput.arguments = Value::Null`.** The wire output item omits
  arguments and §2.3 keeps call/output as separate items paired at *render* time — so the
  reducer does not back-fill from the call. Flag.

- **D-P1-8 — Status normalization.** `SessionEvent::Status.status` is
  `lens_client::stream::SessionStatusValue` (Idle/Launching/Running/Waiting/Failed/Unknown);
  it maps 1:1 by `match` to the **domain** `SessionStatusValue` (a distinct type, same
  shape). Snapshot's coarse `lens_client::sessions::SessionStatus` (Idle/Running/Failed)
  maps Idle→Idle, Running→Running, Failed→Failed.

- **D-P1-9 — Usage normalization (two representations → canonical).**
  - Live `session.usage { context_tokens, context_window, total_cost_usd }`:
    `context_tokens → last_total_tokens`, `context_window → context_window`,
    `total_cost_usd → cumulative_cost.total_cost_usd`. (`i64`→`u64` via `.max(0) as u64`;
    negative is impossible on the wire but total, never panicking.)
  - Snapshot usage: `last_total_tokens`, `context_window`, `total_cost_usd`, and
    `usage_by_model` → `cumulative_cost.cumulative_usage.usage_by_model` (each
    `lens_client` `ModelUsage` → domain `ModelUsage` by getter). Flag the exact
    field semantics for review.

- **D-P1-10 — `Compaction` item from `response.compaction.completed { total_tokens }`** →
  push `ItemKind::Compaction { summary: String::new(), token_count }` (wire omits summary).
  `compaction.in_progress`/`failed` → marker only, no item. Flag.

- **D-P1-11 — Reasoning: reducer's `ReasoningAcc` is authoritative.** `ReasoningStarted`
  opens `open_reasoning`; `ReasoningTextDelta`→`full_text`, `ReasoningSummaryTextDelta`→
  `summary_text`. On the synthetic `ReasoningClosed`, finalize `ItemKind::Reasoning` from
  the **accumulated scratch** (the event also carries the text, but the reducer's own
  accumulation is the source of truth for determinism), `encrypted: false`, then clear
  `open_reasoning`. A `ReasoningClosed` with no open bracket is a no-op.

- **D-P1-12 — Text finalization: `OutputItemDone(message)` is canonical; scratch is
  preview.** `OutputTextDelta` accumulates `open_message` (emits `ScratchChanged` for the
  live bubble). The canonical `Message` item is created by `OutputItemDone` whose item is a
  `message`. On `response.completed`: if `open_message` was **not** already finalized by a
  matching `OutputItemDone` (matched by `message_id`, or — when absent — by "any message
  item finalized this response"), the reducer finalizes the scratch into a `Message` item
  (the terminal-observed-streaming fallback); then it always clears `open_message`. Dedup by
  `id` guarantees no double-insert. Flag the reconciliation rule.

- **D-P1-13 — Dedup/identity by `id`.** Item insertion scans `state.items` for the same
  `id`: present ⇒ update in place (`StreamUpdate::ItemUpdated { index }`), absent ⇒ append
  (`ItemAppended { index }`). `seq` is stamped onto the `Item` (SSE overlap hint) but is
  never an identity key. Replayed `/items` dedups against hydrated items the same way.

- **D-P1-14 — `BlockContext` attribution (P1 simplification).** The reducer tracks a
  `current_agent: Option<String>` and a `turn: u32`, both in `StreamScratch` (D-P1-17).
  `current_agent` is set by `session.agent_changed`, by a completed `FunctionCall.agent_name`
  (REVIEW#7 — updated in the `OutputItemDone` routing before the item is stamped), and by
  `SnapshotRestored`. `turn` is incremented on each `response.completed` (**after** the
  response's items finalize — REVIEW#1). Every created item is stamped
  `BlockContext { agent: current_agent.clone(), depth: 0, turn }`. **`depth` is fixed at 0 in
  P1** — sub-agent depth tracking needs child-session topology (a §9 registry concern); flagged.

- **D-P1-15 — `SnapshotRestored` fold = scalars only.** Fold the snapshot's bucket-B chrome
  (`status` normalized, `llm_model`, `model_override`, `reasoning_effort`, `context_window`,
  `last_total_tokens`, `total_cost_usd`, `usage_by_model`, `skills`, `agent_id`, `agent_name`,
  `archived`, `title`, `labels`, `host_id`, `runner_id`, `workspace`, `git_branch`,
  `parent_session_id`, `permission_level`). **No transcript side-effects** — do **not** read
  `snapshot.items()` here and do **not** push an `AgentChanged` marker (no transition
  happened). The embedded history is replayed by lens-client as subsequent `OutputItemDone`
  events (typed-client §7 ordering). Emits one coarse `StreamUpdate::SnapshotRestored`.

- **D-P1-16 — `Reconnected { gap }` clears `StreamScratch` iff `gap != Some(0)`** (§4.2):
  drop `open_message`/`open_reasoning`/`unpaired_calls` (mid-stream text never persisted is
  gone). It does **NOT** clear `pending_user` (spec P3b — user intent; P3's actor owns that
  reconcile). `Reconnecting`/`Disconnected` pass through as lifecycle markers, no state
  change. Emits `StreamUpdate::Reconnected` (+ `ScratchChanged` if it cleared anything).

- **D-P1-17 — Reduce-local counters live in `StreamScratch`.** `BlockContext` attribution
  needs `turn`/`current_agent` to persist across events without a new persisted field. Add two
  fields to `StreamScratch`: `pub turn: u32` and `pub current_agent: Option<String>` (a P0-domain
  touch — P1 may extend the aggregate; `StreamScratch` is RAM-only, never persisted).
  **REVIEW#6 — reconnect rule (do NOT clear these on `Reconnected`):** `on_reconnected` clears
  only the in-flight accumulators (`open_message`/`open_reasoning`/`unpaired_calls`, D-P1-16). It
  **keeps `turn`** (replay does not re-establish it, and a wrong turn is harmless attribution
  noise vs. a lost counter) and **keeps `current_agent`** (the immediately-following
  `SnapshotRestored` overwrites it authoritatively). This corrects the earlier "cleared like the
  rest of scratch" wording, which contradicted the `on_reconnected` implementation.

- **D-P1-18 — `terminal_pending` needs a `SessionState` home.** The §4.1 fold list includes
  `session.terminal_pending`, but P0 `SessionState` has no such field. Add
  `pub terminal_pending: bool` (default `false`). `child_session.updated` and
  `session.created` (child spawn) have **no** P0 home and are a §9 (multi-session registry)
  concern — P1 folds them to a `StreamUpdate::ChildSessionChanged` marker only, no field. Flag.

- **D-P1-19 — No-op / marker-only events.** `Heartbeat`, `SessionHeartbeat`,
  `ChangedFilesInvalidated`, `TerminalActivity`, `Interrupted`, `Superseded`, `InputConsumed`,
  `Created`, `ResourceCreated`, `ResourceDeleted`, `response.in_progress`, `response.failed`,
  `response.incomplete`, `response.cancelled` produce a marker (or `[]` for pure liveness like
  heartbeats) and no field write in P1. `InputConsumed` / `Superseded` reconciliation is a P3
  actor concern (optimistic-send FIFO, spec §7). Flag each.

---

## `StreamUpdate` draft (the frozen semantic-delta surface — D-P1-2 / spec D6)

Defined in `src/reduce/update.rs`, re-exported from the crate root.

```rust
use smallvec::SmallVec;

/// The reducer's output: which part of `SessionState` a `reduce()` call changed.
/// DRAFT (spec D6): marker-style at P1 (no payload, no `apply()` — no replica exists
/// yet). The P3 walking skeleton ratifies whether apply carries a payload or re-reads a
/// shared snapshot. `SmallVec<[_; 2]>` because most events touch 0–2 groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamUpdate {
    // ── transcript deltas ──
    /// A new canonical item was appended at `index`.
    ItemAppended { index: usize },
    /// An existing canonical item at `index` was updated in place (dedup-by-id hit).
    ItemUpdated { index: usize },
    /// `StreamScratch` (in-progress message/reasoning) changed — the live preview bubble.
    ScratchChanged,

    // ── scalar / collection folds ──
    StatusChanged,
    UsageChanged,
    ModelChanged,
    ReasoningEffortChanged,
    CollaborationModeChanged,
    ModelOptionsChanged,
    TodosChanged,
    SkillsChanged,
    SandboxChanged,
    TerminalPendingChanged,
    ElicitationsChanged,
    ChildSessionChanged,
    PresenceChanged,
    ResourcesChanged,
    /// `agent_id`/`agent_name` changed AND an `AgentChanged` transcript marker was pushed.
    AgentChanged,
    TitleChanged,

    // ── reconnect / bootstrap lifecycle (passthrough for the UI banner) ──
    Reconnecting { attempt: u32 },
    Reconnected,
    Disconnected,
    /// Coarse: the snapshot chrome scalars were bulk-restored (bootstrap or reconnect).
    SnapshotRestored,
}

pub type Updates = SmallVec<[StreamUpdate; 2]>;
```

---

## Shared test helpers (define once in a `#[cfg(test)]` module reused across tasks)

All wire-event tests build events through the **`decode_all` seam** (Task 1b) rather than
constructing the private wrapper types by hand. Define these in a small test-support module
(e.g. `src/reduce/testutil.rs`, `#[cfg(test)]`) that every task's tests import:

```rust
use lens_client::stream::{decode_all, ServerStreamEvent, SessionEvent, ResponseEvent};

/// Decode a single SSE frame (event + JSON data) into exactly one typed event.
pub(crate) fn parse_one(event: &str, data: &str) -> ServerStreamEvent {
    let sse = format!("event: {event}\ndata: {data}\n\n");
    let mut evs = decode_all(sse.as_bytes());
    assert_eq!(evs.len(), 1, "expected exactly one event for {event}");
    evs.pop().unwrap()
}
pub(crate) fn parse_session(event: &str, data: &str) -> ServerStreamEvent { parse_one(event, data) }
pub(crate) fn parse_response(event: &str, data: &str) -> ServerStreamEvent { parse_one(event, data) }

/// A fresh empty state for `(conn_1, conv_1, ag)`.
pub(crate) fn fresh_state() -> crate::domain::SessionState {
    crate::domain::SessionState::new(
        crate::domain::ConnectionId::new("conn_1"),
        crate::domain::SessionId::new("conv_1"),
        crate::domain::AgentId::new("ag"),
    )
}

/// Build a `SessionSnapshot` fixture from JSON (public `Deserialize`, REVIEW#10).
pub(crate) fn snapshot_fixture(json: serde_json::Value) -> lens_client::sessions::SessionSnapshot {
    serde_json::from_value(json).expect("snapshot fixture must deserialize")
}
```

> Confirm the exact SSE frame wire format `parse_one` must emit by checking the lens-client
> `SseParser` tests (`crates/lens-client/src/stream/sse.rs`) — match its `event:`/`data:` framing
> and blank-line terminator exactly. If `decode_all` already accepts the raw captured `.sse`
> bytes, `parse_one` just wraps one frame in that same format.

---

## Task 1: Crate wiring — `smallvec`, `Clock` seam, `StreamUpdate`, `reduce()` stub, domain-field additions

**Files:**
- Modify: `crates/lens-core/Cargo.toml` (add `smallvec`, `criterion` dev-dep, `[[bench]]`)
- Create: `crates/lens-core/src/clock.rs`
- Create: `crates/lens-core/src/reduce/mod.rs`
- Create: `crates/lens-core/src/reduce/update.rs`
- Modify: `crates/lens-core/src/lib.rs` (declare + re-export `clock`, `reduce`)
- Modify: `crates/lens-core/src/domain/session.rs` (add `terminal_pending: bool`; init in `new()`)
- Modify: `crates/lens-core/src/domain/item.rs` (add `turn: u32`, `current_agent: Option<String>` to `StreamScratch`)

**Interfaces:**
- Produces: `Clock` trait (`fn now_millis(&self) -> i64`), `ManualClock`; `StreamUpdate` enum + `Updates` alias; `pub fn reduce(state: &mut SessionState, event: &ServerStreamEvent, clock: &dyn Clock) -> Updates` (stub → `Updates::new()`); `StreamScratch.turn`, `StreamScratch.current_agent`; `SessionState.terminal_pending`.

- [ ] **Step 1: Add deps to `crates/lens-core/Cargo.toml`**

```toml
[dependencies]
lens-client = { path = "../lens-client" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
smallvec = "1"

[dev-dependencies]
criterion = { version = "0.5", default-features = false }
# REVIEW#10: the byte-decode test seam — reducer tests/benches decode raw `.sse` corpus
# bytes into `Vec<ServerStreamEvent>` via lens-client's test-util feature.
lens-client = { path = "../lens-client", features = ["test-util"] }

[[bench]]
name = "reduce_throughput"
harness = false
```

> **Note:** the `dev-dependencies` `lens-client` line ENABLES the `test-util` feature on the
> same path dep for tests/benches (Cargo unions features) — production `lens-core` does not
> pull it. If Cargo warns about the duplicate key, instead add `test-util` under a
> `[features]`-forwarding dev setup; the simplest working form is the two entries above.

- [ ] **Step 1b: Add the `test-util` decode seam to `lens-client`** (REVIEW#10) — `parse_event`
  and `SseParser` are `pub(crate)`, and the wire wrapper structs have no public constructors, so
  `lens-core` cannot honestly build events from bytes without this. Add a feature-gated public
  helper (NOT in `generated.rs`):

  In `crates/lens-client/Cargo.toml`:
  ```toml
  [features]
  # ... existing features ...
  test-util = []   # exposes byte-decode helpers for downstream reducer tests
  ```

  In `crates/lens-client/src/stream/mod.rs` (or wherever the stream module re-exports live):
  ```rust
  /// Decode a complete SSE byte buffer into the typed event sequence — the same
  /// path the reader thread uses (`SseParser` + `parse_event`). Test/bench only.
  #[cfg(feature = "test-util")]
  pub fn decode_all(bytes: &[u8]) -> Vec<ServerStreamEvent> {
      let mut p = crate::stream::sse::SseParser::default();
      let mut frames = p.push(bytes);
      frames.extend(p.finish());
      frames.iter().map(crate::stream::event::parse_event).collect()
  }
  ```
  Gate: `cargo build -p lens-client --features test-util` compiles; `cargo test -p lens-client`
  (no feature) unaffected; `generated.rs` untouched. Commit this with Task 1.

- [ ] **Step 2: Write `src/clock.rs`**

```rust
//! The injected-time seam (§4.1). `reduce()` stamps `Item.created_at` from a
//! `Clock` so replay is deterministic — it never reads the wall clock directly.

/// A monotonic-ish millisecond clock. The production impl reads `SystemTime`
/// (added by the P3 actor); tests use `ManualClock` for deterministic replay.
pub trait Clock {
    /// Epoch milliseconds.
    fn now_millis(&self) -> i64;
}

/// Test/replay double: returns a fixed instant (settable). Deterministic — the
/// P1 replay gate needs "reduce the same events under the same clock twice ⇒
/// identical state", which a wall clock cannot satisfy.
#[derive(Clone, Debug)]
pub struct ManualClock {
    now: std::cell::Cell<i64>,
}

impl ManualClock {
    pub fn new(now_millis: i64) -> Self {
        Self { now: std::cell::Cell::new(now_millis) }
    }
    /// Advance the clock (for tests that assert ordering by `created_at`).
    pub fn set(&self, now_millis: i64) {
        self.now.set(now_millis);
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> i64 {
        self.now.get()
    }
}
```

- [ ] **Step 3: Write `src/reduce/update.rs`** — the full `StreamUpdate` enum (copy the "`StreamUpdate` draft" block above verbatim) plus this unit test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_smallvec_stays_inline_for_two() {
        let mut u: Updates = SmallVec::new();
        u.push(StreamUpdate::StatusChanged);
        u.push(StreamUpdate::ItemAppended { index: 0 });
        assert_eq!(u.len(), 2);
        assert!(!u.spilled(), "the [_; 2] inline cap must hold the common case");
    }
}
```

- [ ] **Step 4: Write `src/reduce/mod.rs`** — the dispatch stub (real arms land in later tasks):

```rust
//! §4.1 canonical reducer — pure, deterministic, no I/O. Folds one
//! `ServerStreamEvent` into `SessionState` and returns semantic `StreamUpdate`s.

pub mod update;
mod folds;
mod items;
mod scratch;
mod snapshot;
pub mod transforms;

pub use update::{StreamUpdate, Updates};

use crate::clock::Clock;
use crate::domain::SessionState;
use lens_client::stream::ServerStreamEvent;
use smallvec::SmallVec;

/// Fold one event into `state`; return which parts changed (§4.1). Total over
/// every event arm — never panics on external data (AGENTS.md).
pub fn reduce(
    state: &mut SessionState,
    event: &ServerStreamEvent,
    clock: &dyn Clock,
) -> Updates {
    // Arms are filled in Tasks 2–9; unhandled events are a no-op for now.
    let _ = (state, event, clock);
    SmallVec::new()
}
```

Create empty module files so it compiles: `src/reduce/{folds,items,scratch,snapshot,transforms}.rs` each with a `//!` doc line (bodies added in later tasks).

- [ ] **Step 5: Extend `StreamScratch` in `src/domain/item.rs`** (D-P1-17) — add two RAM-only reduce counters:

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamScratch {
    pub open_message: Option<MessageAcc>,
    pub open_reasoning: Option<ReasoningAcc>,
    pub unpaired_calls: HashMap<CallId, ItemId>,
    /// Reduce-local (§4.1 attribution): current turn, bumped on `response.completed`.
    pub turn: u32,
    /// Reduce-local: current agent name for `BlockContext` stamping.
    pub current_agent: Option<String>,
}
```

Update the existing `stream_scratch_default_is_empty_and_roundtrips` test to also assert `s.turn == 0 && s.current_agent.is_none()`.

- [ ] **Step 6: Add `terminal_pending` to `SessionState` in `src/domain/session.rs`** (D-P1-18) — add the field near `sandbox_status` and initialize `terminal_pending: false` in `new()`:

```rust
    pub sandbox_status: Option<SandboxStatus>,
    /// Live `session.terminal_pending` fold (§4.1). RAM+persisted scalar.
    pub terminal_pending: bool,
```

- [ ] **Step 7: Wire `src/lib.rs`**

```rust
pub mod clock;
pub mod domain;
pub mod reduce;

pub use clock::{Clock, ManualClock};
pub use domain::*;
pub use reduce::{reduce, StreamUpdate, Updates};
```

- [ ] **Step 8: Create the bench skeleton `benches/reduce_throughput.rs`** (real body in Task 11) so the `[[bench]]` target resolves:

```rust
//! Reducer throughput over the golden corpus (AGENTS.md benchmark-or-it's-not-done).
//! Real corpus wiring in Task 11.
fn main() {}
```

- [ ] **Step 9: Verify it builds + a smoke test**

Add to `src/reduce/mod.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState};

    fn empty_state() -> SessionState {
        SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("ag_1"),
        )
    }

    #[test]
    fn reduce_stub_is_a_noop() {
        let mut s = empty_state();
        let clock = ManualClock::new(1_700_000_000_000);
        let ev = ServerStreamEvent::Reconnecting { attempt: 1 };
        let out = reduce(&mut s, &ev, &clock);
        assert!(out.is_empty());
    }
}
```

Run: `cargo test -p lens-core && cargo clippy -p lens-core --all-targets && cargo fmt --check`
Expected: PASS, zero warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/lens-core
git commit -m "feat(lens-core): P1 task 1 — reduce scaffold, Clock seam, StreamUpdate draft"
```

---

## Task 2: Session-field scalar folds + status normalization (`reduce/folds.rs`)

Folds the simple session chrome events into `SessionState` scalars/collections. TDD from
byte fixtures. No items produced.

**Files:**
- Modify: `crates/lens-core/src/reduce/folds.rs`
- Modify: `crates/lens-core/src/reduce/mod.rs` (dispatch `ServerStreamEvent::Session(..)` field arms here)

**Interfaces:**
- Consumes: `lens_client::stream::{SessionEvent, SessionStatusValue as WireStatus}`; domain `SessionState`, `SessionStatusValue`, `Todo`, `TodoStatus`, `SkillSummary`, `SandboxStatus`.
- Produces: `pub(crate) fn fold_session_field(state, ev: &SessionEvent) -> Updates` (the non-item, non-usage, non-presence, non-child arms); `pub fn normalize_status(WireStatus) -> SessionStatusValue`.

- [ ] **Step 1: Write failing tests** (`folds.rs` tests) — status, model, reasoning_effort, todos, sandbox, terminal_pending, skills:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState, SessionStatusValue, TodoStatus};
    use crate::reduce::{reduce, StreamUpdate};
    use lens_client::stream::{ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus, TodoItem, TodoItemStatus};

    fn st() -> SessionState {
        SessionState::new(ConnectionId::new("c"), SessionId::new("conv"), AgentId::new("ag"))
    }
    fn clock() -> ManualClock { ManualClock::new(1_700_000_000_000) }

    #[test]
    fn status_running_folds_and_marks() {
        let mut s = st();
        let u = reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::Status {
            status: WireStatus::Running, response_id: None, background_task_count: None,
        }), &clock());
        assert_eq!(s.status, SessionStatusValue::Running);
        assert_eq!(&u[..], &[StreamUpdate::StatusChanged]);
    }

    #[test]
    fn model_and_effort_fold() {
        let mut s = st();
        reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::Model { model: "opus".into() }), &clock());
        assert_eq!(s.llm_model.as_deref(), Some("opus"));
        reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::ReasoningEffort {
            reasoning_effort: Some("high".into()),
        }), &clock());
        assert_eq!(s.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn todos_replace_wholesale() {
        let mut s = st();
        // REVIEW#10: `TodoItem` has private fields — build the event from bytes via the
        // `parse_session` shared helper (decode_all seam), not a hand-built wrapper.
        let ev = parse_session("session.todos",
            r#"{"conversation_id":"c","todos":[{"content":"Fix bug","status":"in_progress","activeForm":"Fixing bug"}]}"#);
        let u = reduce(&mut s, &ev, &clock());
        assert_eq!(s.todos.len(), 1);
        assert_eq!(s.todos[0].content, "Fix bug");
        assert_eq!(s.todos[0].status, TodoStatus::InProgress);
        assert_eq!(&u[..], &[StreamUpdate::TodosChanged]);
    }

    #[test]
    fn terminal_pending_folds() {
        let mut s = st();
        reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::TerminalPending { pending: true }), &clock());
        assert!(s.terminal_pending);
    }
}
```

> **Test-construction rule (RESOLVED, REVIEW#10):** wire events whose payloads use private
> wrapper types (`TodoItem`, `PresenceViewer`, `ChildSession`, `ElicitationParams`) are built
> from bytes via the `parse_session`/`parse_response` shared helpers (over the `decode_all`
> seam, Task 1b). Events whose fields are public (`Status`, `Model`, `Usage`, `TerminalPending`,
> …) may be constructed directly as shown. Both are honest — `decode_all` is the same
> `SseParser` + `parse_event` path the reader thread uses.

- [ ] **Step 2: Run — expect FAIL** (`fold_session_field` unimplemented).

Run: `cargo test -p lens-core reduce::folds`
Expected: FAIL (compile error / no field write).

- [ ] **Step 3: Implement `normalize_status` + `fold_session_field`** in `folds.rs`:

```rust
use crate::domain::{SandboxStatus, SessionState, SessionStatusValue, SkillSummary, Todo, TodoStatus};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::{SessionEvent, SessionStatusValue as WireStatus, TodoItemStatus};
use smallvec::smallvec;

/// Map the 6-value wire status to the domain status (D-P1-8). Distinct types, same shape.
pub fn normalize_status(w: WireStatus) -> SessionStatusValue {
    match w {
        WireStatus::Idle => SessionStatusValue::Idle,
        WireStatus::Launching => SessionStatusValue::Launching,
        WireStatus::Running => SessionStatusValue::Running,
        WireStatus::Waiting => SessionStatusValue::Waiting,
        WireStatus::Failed => SessionStatusValue::Failed,
        WireStatus::Unknown => SessionStatusValue::Unknown,
    }
}

fn map_todo_status(w: TodoItemStatus) -> TodoStatus {
    match w {
        TodoItemStatus::Pending => TodoStatus::Pending,
        TodoItemStatus::InProgress => TodoStatus::InProgress,
        TodoItemStatus::Completed => TodoStatus::Completed,
        TodoItemStatus::Unknown => TodoStatus::Unknown, // REVIEW#8: preserve churn signal
    }
}

/// The non-item, non-usage, non-presence, non-child session-field arms. Returns
/// `None` for arms handled elsewhere so `reduce` can route them.
pub(crate) fn fold_session_field(state: &mut SessionState, ev: &SessionEvent) -> Option<Updates> {
    Some(match ev {
        SessionEvent::Status { status, .. } => {
            state.status = normalize_status(*status);
            smallvec![StreamUpdate::StatusChanged]
        }
        SessionEvent::Model { model } => {
            state.llm_model = Some(model.clone());
            smallvec![StreamUpdate::ModelChanged]
        }
        SessionEvent::ReasoningEffort { reasoning_effort } => {
            state.reasoning_effort = reasoning_effort.clone();
            smallvec![StreamUpdate::ReasoningEffortChanged]
        }
        SessionEvent::ModelOptions => smallvec![StreamUpdate::ModelOptionsChanged],
        SessionEvent::Todos { todos } => {
            state.todos = todos.iter().map(|t| Todo {
                content: t.content().to_string(),
                status: map_todo_status(t.status()),
                active_form: t.active_form().to_string(),
            }).collect();
            smallvec![StreamUpdate::TodosChanged]
        }
        SessionEvent::Skills => {
            // P1-DECISION: lens-client `session.skills` wrapper is a unit variant (payload
            // dropped) — no names available. Mark changed; leave `state.skills` untouched.
            smallvec![StreamUpdate::SkillsChanged]
        }
        SessionEvent::SandboxStatus { stage, error } => {
            state.sandbox_status = Some(SandboxStatus {
                stage: stage.clone(),
                detail: error.clone(),
            });
            smallvec![StreamUpdate::SandboxChanged]
        }
        SessionEvent::TerminalPending { pending } => {
            state.terminal_pending = *pending;
            smallvec![StreamUpdate::TerminalPendingChanged]
        }
        // Marker-only (D-P1-19): no P1 field home / liveness only.
        SessionEvent::TerminalActivity { .. } => smallvec![StreamUpdate::TerminalPendingChanged],
        SessionEvent::ChangedFilesInvalidated { .. }
        | SessionEvent::Interrupted { .. }
        | SessionEvent::Superseded { .. }
        | SessionEvent::InputConsumed { .. } => return Some(smallvec![]),
        // REVIEW#9: child spawn — D-P1-18 marker (no P1 field home; §9 owns child topology).
        SessionEvent::Created { .. } => smallvec![StreamUpdate::ChildSessionChanged],
        SessionEvent::ResourceCreated | SessionEvent::ResourceDeleted { .. } => {
            smallvec![StreamUpdate::ResourcesChanged] // D-P1-4
        }
        SessionEvent::Heartbeat { .. } => return Some(smallvec![]),
        // Handled elsewhere:
        SessionEvent::Usage { .. }
        | SessionEvent::Presence { .. }
        | SessionEvent::ChildSessionUpdated { .. }
        | SessionEvent::AgentChanged { .. } => return None,
    })
}
```

- [ ] **Step 4: Route in `reduce/mod.rs`** — add to `reduce()` before the fallthrough:

```rust
    if let ServerStreamEvent::Session(ev) = event {
        if let Some(updates) = folds::fold_session_field(state, ev) {
            return updates;
        }
    }
```

- [ ] **Step 5: Run — expect PASS.**

Run: `cargo test -p lens-core reduce::folds`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-core
git commit -m "feat(lens-core): P1 task 2 — session-field scalar folds + status normalization"
```

---

## Task 3: Usage fold + normalization (`reduce/folds.rs`)

**Files:** Modify `crates/lens-core/src/reduce/folds.rs`; route `SessionEvent::Usage` in `mod.rs`.

**Interfaces:**
- Produces: `pub(crate) fn fold_usage(state, context_tokens, context_window, total_cost_usd) -> Updates`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn usage_folds_into_canonical_cost() {
    let mut s = st();
    let u = reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::Usage {
        context_tokens: Some(1200), context_window: Some(200_000), total_cost_usd: Some(0.42),
    }), &clock());
    assert_eq!(s.last_total_tokens, Some(1200));
    assert_eq!(s.context_window, Some(200_000));
    assert_eq!(s.cumulative_cost.total_cost_usd, Some(0.42));
    assert_eq!(&u[..], &[StreamUpdate::UsageChanged]);
}

#[test]
fn usage_negative_wire_ints_never_panic() {
    let mut s = st();
    reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::Usage {
        context_tokens: Some(-5), context_window: None, total_cost_usd: None,
    }), &clock());
    assert_eq!(s.last_total_tokens, Some(0)); // clamped, total
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** in `folds.rs` and route the arm:

```rust
pub(crate) fn fold_usage(
    state: &mut SessionState,
    context_tokens: Option<i64>,
    context_window: Option<i64>,
    total_cost_usd: Option<f64>,
) -> Updates {
    if let Some(ct) = context_tokens {
        state.last_total_tokens = Some(ct.max(0) as u64);
    }
    if let Some(cw) = context_window {
        state.context_window = Some(cw.max(0) as u64);
    }
    if let Some(cost) = total_cost_usd {
        state.cumulative_cost.total_cost_usd = Some(cost);
    }
    smallvec![StreamUpdate::UsageChanged]
}
```

Add to `fold_session_field`'s `Usage` arm (change it from `return None` to call `fold_usage`), or route in `mod.rs` — keep all `SessionEvent` folding in `fold_session_field` for one dispatch site. Wire `SessionEvent::Usage { context_tokens, context_window, total_cost_usd } => fold_usage(state, *context_tokens, *context_window, *total_cost_usd)`.

- [ ] **Step 4: Run — PASS.** `cargo test -p lens-core reduce::folds`

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 3 — usage fold + normalization`

---

## Task 4: Presence, elicitation, child-session, agent-changed folds (`reduce/folds.rs` + `items.rs`)

**Files:** Modify `folds.rs`; `items.rs` (for the `AgentChanged` item push — depends on Task 7's item-insert helper, so **sequence Task 4 after Task 7** or stub the insert). **Order note:** implement the presence/elicitation/child arms here; defer the `AgentChanged` *item insertion* to after Task 7 lands `push_item`. This task delivers presence + elicitation + child + the `agent_id`/`agent_name` scalar fold (marker only); the transcript marker is completed in Task 8.

**Interfaces:**
- Consumes: `lens_client::stream::{PresenceViewer as WireViewer, ChildSession}`; domain `PresenceViewer`, `Elicitation`.
- Produces: presence/child arms in `fold_session_field`; `AgentChanged` scalar fold.

- [ ] **Step 1: Failing tests** — presence (user_id only, D-P1-5), agent scalar fold, child marker:

```rust
#[test]
fn presence_fills_user_id_only() {
    let mut s = st();
    // build via bytes so the private wrapper is populated
    let ev = parse_session("session.presence", r#"{"viewers":[{"user_id":"u_1"}]}"#);
    let u = reduce(&mut s, &ev, &clock());
    assert_eq!(s.presence.len(), 1);
    assert_eq!(s.presence[0].user_id, "u_1");
    assert_eq!(s.presence[0].joined_at, ""); // P1-DECISION: wrapper drops joined_at/idle
    assert!(!s.presence[0].idle);
    assert_eq!(&u[..], &[StreamUpdate::PresenceChanged]);
}

#[test]
fn agent_changed_updates_scalars() {
    let mut s = st();
    let u = reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::AgentChanged {
        agent_id: "ag_2".into(), agent_name: "debby".into(),
    }), &clock());
    assert_eq!(s.agent_id.as_str(), "ag_2");
    assert_eq!(s.agent_name.as_deref(), Some("debby"));
    assert!(u.contains(&StreamUpdate::AgentChanged));
}
```

Add a `parse_session(event, data)` helper — see the Task 2 review note; if `parse_event` is
unreachable, construct the `SessionEvent::Presence { viewers }` directly IF a public
constructor exists, else drive the public `EventStream` in the corpus test (Task 11) for the
byte-level guarantee and use a minimal hand-built `WireViewer` here. **RESOLVE IN REVIEW.**

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** the arms in `fold_session_field`:

```rust
        SessionEvent::Presence { viewers } => {
            state.presence = viewers.iter().filter_map(|v| {
                v.user_id().map(|uid| crate::domain::PresenceViewer {
                    user_id: uid.to_string(),
                    joined_at: String::new(), // P1-DECISION D-P1-5: wrapper drops these
                    idle: false,
                })
            }).collect();
            smallvec![StreamUpdate::PresenceChanged]
        }
        SessionEvent::AgentChanged { agent_id, agent_name } => {
            state.agent_id = crate::domain::AgentId::new(agent_id.clone());
            state.agent_name = Some(agent_name.clone());
            state.stream.current_agent = Some(agent_name.clone());
            // Transcript marker pushed in Task 8 (needs push_item); scalar fold here.
            smallvec![StreamUpdate::AgentChanged]
        }
        SessionEvent::ChildSessionUpdated { .. } => smallvec![StreamUpdate::ChildSessionChanged],
```

Elicitation folds are `ResponseEvent` arms (`response.elicitation_request/resolved`), handled
in Task 6's response routing — note here that `ElicitationRequest` pushes an `Elicitation`
onto `state.pending_elicitations` and `ElicitationResolved` removes by `elicitation_id`
(`StreamUpdate::ElicitationsChanged`). Move the elicitation code to Task 6 where response
events are wired; keep this task to session-field arms.

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 4 — presence, agent-changed scalar, child-session folds`

---

## Task 5: Text accumulation (`reduce/scratch.rs`)

`OutputTextDelta` → `open_message` (`MessageAcc`); finalize into a `Message` item is Task 7's
concern (needs `push_item`). This task owns the **accumulation** into scratch + `ScratchChanged`.

**Files:** Modify `crates/lens-core/src/reduce/scratch.rs`; route `ResponseEvent::OutputTextDelta` in `mod.rs`.

**Interfaces:**
- Produces: `pub(crate) fn accumulate_text(scratch: &mut StreamScratch, delta: &str, message_id: Option<&str>, index: Option<usize>) -> Updates`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn text_deltas_accumulate_in_scratch() {
    let mut s = st();
    reduce(&mut s, &resp_text("Hel", None, None), &clock());
    reduce(&mut s, &resp_text("lo", None, None), &clock());
    let acc = s.stream.open_message.as_ref().unwrap();
    assert_eq!(acc.text, "Hello");
    // last reduce emitted a ScratchChanged
}

// helper
fn resp_text(delta: &str, message_id: Option<&str>, index: Option<usize>) -> ServerStreamEvent {
    ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
        delta: delta.into(),
        message_id: message_id.map(str::to_string),
        index,
        last: None,
    })
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** `accumulate_text` + route:

```rust
use crate::domain::item::{MessageAcc, StreamScratch};
use crate::reduce::{StreamUpdate, Updates};
use smallvec::smallvec;

pub(crate) fn accumulate_text(
    scratch: &mut StreamScratch,
    delta: &str,
    message_id: Option<&str>,
    index: Option<usize>,
) -> Updates {
    let acc = scratch.open_message.get_or_insert_with(|| MessageAcc {
        message_id: message_id.map(str::to_string),
        text: String::new(),
        block_index: index.unwrap_or(0),
    });
    acc.text.push_str(delta);
    if let Some(i) = index {
        acc.block_index = i;
    }
    smallvec![StreamUpdate::ScratchChanged]
}
```

In `reduce/mod.rs` route `ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { delta, message_id, index, .. }) => scratch::accumulate_text(&mut state.stream, delta, message_id.as_deref(), *index)`.

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 5 — streaming text accumulation`

---

## Task 6: Reasoning bracketing + response lifecycle + elicitation (`reduce/scratch.rs` + `folds.rs`)

`ReasoningStarted` → open; deltas append; the synthetic `ReasoningClosed` finalizes a
`Reasoning` item (needs `push_item` from Task 7 — so **land the accumulation + close-detection
here, and the item push in Task 8** OR sequence Task 7 before this). Also: `response.completed`
bumps `turn` + finalizes scratch (Task 7 push), `response.in_progress`/`failed`/`incomplete`/
`cancelled` markers, compaction markers, and the elicitation folds.

**Files:** Modify `scratch.rs` (reasoning acc), `folds.rs` (response markers + elicitation); route `ResponseEvent` in `mod.rs`.

**Interfaces:**
- Produces: `pub(crate) fn accumulate_reasoning(scratch, kind, delta) -> Updates`; `pub(crate) fn fold_response_marker(state, ev: &ResponseEvent) -> Option<Updates>` (the arms that are marker/elicitation only; item-producing arms — `OutputItemDone`, `ReasoningClosed` finalize, `CompactionCompleted`, `Completed` finalize — routed in Task 7/8).

- [ ] **Step 1: Failing tests** — reasoning accumulation, elicitation add/remove:

```rust
#[test]
fn reasoning_deltas_accumulate() {
    let mut s = st();
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted), &clock());
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "be".into() }), &clock());
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "cause".into() }), &clock());
    assert_eq!(s.stream.open_reasoning.as_ref().unwrap().full_text, "because");
}

#[test]
fn elicitation_request_then_resolved() {
    let mut s = st();
    let req = parse_response("response.elicitation_request",
        r#"{"elicitation_id":"e1","params":{"mode":"url","message":"ok?","url":"/a"}}"#);
    reduce(&mut s, &req, &clock());
    assert_eq!(s.pending_elicitations.len(), 1);
    assert_eq!(s.pending_elicitations[0].id.as_str(), "e1");
    let res = ServerStreamEvent::Response(ResponseEvent::ElicitationResolved { elicitation_id: "e1".into() });
    let u = reduce(&mut s, &res, &clock());
    assert!(s.pending_elicitations.is_empty());
    assert_eq!(&u[..], &[StreamUpdate::ElicitationsChanged]);
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** `accumulate_reasoning` (in `scratch.rs`) + response markers + elicitation (in `folds.rs`):

```rust
// scratch.rs
pub(crate) enum ReasoningKind { Full, Summary }
pub(crate) fn accumulate_reasoning(scratch: &mut StreamScratch, kind: ReasoningKind, delta: &str) -> Updates {
    let acc = scratch.open_reasoning.get_or_insert_with(Default::default);
    match kind {
        ReasoningKind::Full => acc.full_text.push_str(delta),
        ReasoningKind::Summary => acc.summary_text.push_str(delta),
    }
    smallvec![StreamUpdate::ScratchChanged]
}
```

```rust
// folds.rs — elicitation + response markers
use crate::domain::{Elicitation, ElicitationId, ElicitationParams as DomainElicParams, SessionId};
use lens_client::stream::ResponseEvent;

pub(crate) fn fold_response_marker(state: &mut SessionState, ev: &ResponseEvent) -> Option<Updates> {
    Some(match ev {
        ResponseEvent::InProgress => smallvec![StreamUpdate::StatusChanged], // P1-DECISION: liveness marker
        ResponseEvent::Failed | ResponseEvent::Incomplete | ResponseEvent::Cancelled => smallvec![],
        ResponseEvent::CompactionInProgress | ResponseEvent::CompactionFailed => smallvec![],
        // REVIEW#4: fold response.error into the `last_task_error` scalar banner (ErrorInfo,
        // "present iff Failed"). NOT a transcript item — the byte-verified error-item path is
        // `OutputItemDone(Error)`; pushing from both would double-insert. This preserves the
        // external error data without that hazard.
        ResponseEvent::Error { code, message, .. } => {
            state.last_task_error = Some(crate::domain::ErrorInfo {
                code: code.clone(),
                message: message.clone(),
            });
            smallvec![StreamUpdate::StatusChanged]
        }
        ResponseEvent::ElicitationRequest { elicitation_id, params } => {
            state.pending_elicitations.push(Elicitation {
                id: ElicitationId::new(elicitation_id.clone()),
                target_session_id: state.id.clone(),
                params: DomainElicParams {
                    mode: params.mode().to_string(),
                    message: params.message().to_string(),
                    url: params.url().map(str::to_string),
                    phase: params.phase().map(str::to_string),
                    policy_name: params.policy_name().map(str::to_string),
                    content_preview: params.content_preview().map(str::to_string),
                },
            });
            smallvec![StreamUpdate::ElicitationsChanged]
        }
        ResponseEvent::ElicitationResolved { elicitation_id } => {
            state.pending_elicitations.retain(|e| e.id.as_str() != elicitation_id);
            smallvec![StreamUpdate::ElicitationsChanged]
        }
        // item-producing / scratch-finalizing arms handled in Task 7/8:
        ResponseEvent::OutputItemDone { .. }
        | ResponseEvent::Completed
        | ResponseEvent::ReasoningClosed { .. }
        | ResponseEvent::CompactionCompleted { .. }
        | ResponseEvent::OutputTextDelta { .. }
        | ResponseEvent::ReasoningStarted
        | ResponseEvent::ReasoningTextDelta { .. }
        | ResponseEvent::ReasoningSummaryTextDelta { .. } => return None,
    })
}
```

Route `ReasoningStarted` (opens acc — `scratch.open_reasoning.get_or_insert_with(Default::default)`, emit `ScratchChanged`), `ReasoningTextDelta`/`ReasoningSummaryTextDelta` (call `accumulate_reasoning`), and `fold_response_marker` in `mod.rs`.

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 6 — reasoning accumulation + response markers + elicitation folds`

---

## Task 7: Item creation, dedup-by-id, `BlockContext` stamping (`reduce/items.rs`)

The core of the transcript. Maps `lens_client::stream::Item` → domain `ItemKind`, stamps
`BlockContext` + `created_at` (from the clock), and inserts with dedup-by-`id`.

**Files:** Modify `crates/lens-core/src/reduce/items.rs`; route `ResponseEvent::OutputItemDone` in `mod.rs`.

**Interfaces:**
- Consumes: `lens_client::stream::Item as WireItem`; domain `Item`, `ItemKind`, `ContentBlock`, `BlockContext`, ids.
- Produces:
  - `pub(crate) fn map_item(wire: &WireItem) -> Option<(ItemId, ItemKind)>` (the wire→domain mapping, D-P1-3/6/7; `None` for resources per D-P1-4/REVIEW#3);
  - `pub(crate) fn push_item(state: &mut SessionState, id: ItemId, kind: ItemKind, seq: Option<u64>, clock: &dyn Clock) -> Updates` (dedup-by-id insert/update + ctx stamp);
  - `pub(crate) fn current_ctx(scratch: &StreamScratch) -> BlockContext`.

- [ ] **Step 1: Failing tests** — function_call arguments parsed, agent_name sanitized, dedup-by-id, ctx stamp:

```rust
#[test]
fn function_call_parses_arguments_and_sanitizes_agent() {
    let mut s = st();
    let ev = parse_response("response.output_item.done",
        r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{\"path\":\"a.rs\"}","call_id":"toolu_1","agent":"coder"}}"#);
    let u = reduce(&mut s, &ev, &clock());
    assert_eq!(s.items.len(), 1);
    match &s.items[0].kind {
        ItemKind::FunctionCall { call_id, arguments, agent_name, status, .. } => {
            assert_eq!(call_id.as_str(), "toolu_1");
            assert_eq!(arguments["path"], "a.rs"); // parsed to Value
            assert_eq!(agent_name.as_deref(), Some("coder")); // completed ⇒ name kept
            assert_eq!(status, "completed");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(&u[..], &[StreamUpdate::ItemAppended { index: 0 }]);
    assert_eq!(s.items[0].created_at, 1_700_000_000_000); // clock-stamped
}

#[test]
fn in_progress_function_call_drops_resp_id_agent() {
    let mut s = st();
    let ev = parse_response("response.output_item.done",
        r#"{"item":{"id":"fc_2","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"c","agent":"resp_abc"}}"#);
    reduce(&mut s, &ev, &clock());
    match &s.items[0].kind {
        ItemKind::FunctionCall { agent_name, .. } => assert_eq!(*agent_name, None), // D-P1-6
        other => panic!("{other:?}"),
    }
}

#[test]
fn duplicate_id_updates_in_place() {
    let mut s = st();
    let first = parse_response("response.output_item.done",
        r#"{"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"c"}}"#);
    reduce(&mut s, &first, &clock());
    let second = parse_response("response.output_item.done",
        r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{}","call_id":"c"}}"#);
    let u = reduce(&mut s, &second, &clock());
    assert_eq!(s.items.len(), 1); // no double-insert (D-P1-13)
    assert_eq!(&u[..], &[StreamUpdate::ItemUpdated { index: 0 }]);
}

#[test]
fn unmodeled_item_maps_to_native_tool_catchall() {
    let mut s = st();
    let ev = parse_response("response.output_item.done",
        r#"{"item":{"id":"x_9","type":"native_tool","kind":"web_search_call"}}"#);
    reduce(&mut s, &ev, &clock());
    match &s.items[0].kind {
        ItemKind::NativeTool { tool_type, .. } => assert_eq!(tool_type, "native_tool"), // D-P1-3
        other => panic!("{other:?}"),
    }
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** `items.rs`:

```rust
use crate::clock::Clock;
use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind, StreamScratch};
use crate::domain::{CallId, ErrorSource, ItemId, Role, SessionState};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::Item as WireItem;
use serde_json::Value;
use smallvec::smallvec;

pub(crate) fn current_ctx(scratch: &StreamScratch) -> BlockContext {
    BlockContext {
        agent: scratch.current_agent.clone(),
        depth: 0, // P1-DECISION D-P1-14: sub-agent depth deferred to §9
        turn: scratch.turn,
    }
}

fn role_of(role: &str) -> Role {
    match role { "user" => Role::User, _ => Role::Assistant }
}

fn parse_args(raw: &str) -> Value {
    // D-P1-6: wire `arguments` is a raw JSON string; the state model owns parsing.
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// REVIEW#3: returns `None` for wire items that produce NO transcript item (resources —
/// D-P1-4). The `OutputItemDone` routing turns `None` into a `ResourcesChanged` marker.
pub(crate) fn map_item(wire: &WireItem) -> Option<(ItemId, ItemKind)> {
    let id = ItemId::new(wire.id().to_string());
    let kind = match wire {
        WireItem::Message { role, content, .. } => ItemKind::Message {
            role: role_of(role),
            content: content.iter().map(|b| ContentBlock {
                kind: b.block_type().to_string(),
                text: b.text().map(str::to_string),
                data: Value::Null,
            }).collect(),
        },
        WireItem::FunctionCall { call_id, name, arguments, status, agent, .. } => ItemKind::FunctionCall {
            call_id: CallId::new(call_id.clone()),
            name: name.clone(),
            arguments: parse_args(arguments),
            status: status.clone(),
            agent_name: agent.clone().filter(|_| status == "completed"), // D-P1-6
        },
        WireItem::FunctionCallOutput { call_id, output, .. } => ItemKind::FunctionCallOutput {
            call_id: CallId::new(call_id.clone()),
            output: output.clone(),
            arguments: Value::Null, // D-P1-7: paired at render, not back-filled
        },
        WireItem::Error { source, code, message, .. } => ItemKind::Error {
            source: source.as_deref().map(map_error_source).unwrap_or(ErrorSource::Unknown),
            code: code.clone().unwrap_or_default(),
            message: message.clone().unwrap_or_default(),
        },
        // D-P1-4: resources are NOT materialized as items in P1 (no SessionResourceObject
        // available from the wire) → None ⇒ ResourcesChanged marker.
        WireItem::ResourceEvent { .. } => return None,
        // D-P1-3 catch-all: native tools / unmodeled wire items keep a transcript slot;
        // full payload deferred until lens-client widens `stream::Item`.
        WireItem::Other { item_type, .. } => ItemKind::NativeTool {
            tool_type: item_type.clone(),
            data: Value::Null,
        },
    };
    Some((id, kind))
}

fn map_error_source(s: &str) -> ErrorSource {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(ErrorSource::Unknown)
}

/// Dedup-by-id insert (D-P1-13). Present ⇒ update in place; absent ⇒ append.
pub(crate) fn push_item(
    state: &mut SessionState,
    id: ItemId,
    kind: ItemKind,
    seq: Option<u64>,
    clock: &dyn Clock,
) -> Updates {
    let ctx = current_ctx(&state.stream);
    if let Some(idx) = state.items.iter().position(|it| it.id == id) {
        let existing = &mut state.items[idx];
        existing.kind = kind;
        existing.seq = seq.or(existing.seq);
        smallvec![StreamUpdate::ItemUpdated { index: idx }]
    } else {
        state.items.push(Item { id, seq, ctx, created_at: clock.now_millis(), kind });
        smallvec![StreamUpdate::ItemAppended { index: state.items.len() - 1 }]
    }
}
```

Route `OutputItemDone { item }` in `mod.rs`:

```rust
ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) => {
    match items::map_item(item) {
        // D-P1-4 / REVIEW#3: resource items produce no transcript item.
        None => smallvec![StreamUpdate::ResourcesChanged],
        Some((id, kind)) => {
            // REVIEW#7 / D-P1-14: a completed FunctionCall's sanitized agent_name becomes the
            // current attribution agent for this and subsequent items.
            if let crate::domain::ItemKind::FunctionCall { agent_name: Some(a), .. } = &kind {
                state.stream.current_agent = Some(a.clone());
            }
            // REVIEW#5 / D-P1-12: the canonical Message supersedes the streaming preview ONLY
            // when it is the SAME message — match by message_id (None ⇒ untracked single
            // preview for this turn, safe to clear). An unrelated keyed preview is preserved.
            if let crate::domain::ItemKind::Message { .. } = &kind {
                let clears = state.stream.open_message.as_ref().is_some_and(|acc| {
                    acc.message_id.is_none() || acc.message_id.as_deref() == Some(id.as_str())
                });
                if clears {
                    state.stream.open_message = None;
                }
            }
            items::push_item(state, id, kind, None, clock)
        }
    }
}
```

> **Note (D-P1-6 destructuring):** `WireItem::Message` also has an `id` field; use `..` to
> skip fields not needed. `status == "completed"` compares `&String` to `&str` — use
> `status == "completed"` (Rust coerces) or `status.as_str() == "completed"`.

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 7 — item mapping, dedup-by-id, BlockContext stamping`

---

## Task 8: Finalizers — message/reasoning close, compaction item, agent-changed marker, turn bump (`reduce/mod.rs` + `items.rs`)

Ties the accumulators to canonical items and the response boundary.

**Files:** Modify `mod.rs` (route `Completed`, `ReasoningClosed`, `CompactionCompleted`); `items.rs` (finalize helpers + `AgentChanged` marker push).

**Interfaces:**
- Produces: `pub(crate) fn finalize_message(state, clock) -> Updates`; `pub(crate) fn finalize_reasoning(state, clock) -> Updates`; `pub(crate) fn push_agent_changed(state, from: AgentId, to: AgentId, clock) -> Updates`.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn completed_bumps_turn_and_finalizes_unpersisted_message() {
    let mut s = st();
    reduce(&mut s, &resp_text("hi", None, None), &clock());
    // no OutputItemDone(message) arrived → terminal-observed fallback finalizes on completed
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::Completed), &clock());
    assert_eq!(s.stream.turn, 1);
    assert_eq!(s.items.len(), 1);
    assert!(matches!(s.items[0].kind, ItemKind::Message { .. }));
    assert_eq!(s.items[0].ctx.turn, 0); // REVIEW#1: stamped with the PRE-bump turn
    assert!(s.stream.open_message.is_none());
}

#[test]
fn synthetic_ids_are_unique_across_same_clock_finalizes() {
    // REVIEW#2: two turns finalized at the SAME fixed clock must NOT dedup-collide.
    let mut s = st();
    let clk = clock(); // fixed instant
    reduce(&mut s, &resp_text("first", None, None), &clk);
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::Completed), &clk);
    reduce(&mut s, &resp_text("second", None, None), &clk);
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::Completed), &clk);
    assert_eq!(s.items.len(), 2, "same-clock synthetic ids collided → dedup ate one");
    assert_ne!(s.items[0].id, s.items[1].id);
}

#[test]
fn output_item_done_unrelated_keyed_message_preserves_open_preview() {
    // REVIEW#5: a done message whose id ≠ the KEYED open preview must not clear the preview.
    let mut s = st();
    reduce(&mut s, &resp_text("streaming…", Some("msg_A"), None), &clock());
    let done_other = parse_response("response.output_item.done",
        r#"{"item":{"id":"msg_B","type":"message","role":"assistant","content":[{"type":"output_text","text":"other"}]}}"#);
    reduce(&mut s, &done_other, &clock());
    assert!(s.stream.open_message.is_some(), "unrelated msg_B must not clear the msg_A preview");
}

#[test]
fn completed_does_not_double_insert_when_output_item_done_won() {
    let mut s = st();
    reduce(&mut s, &resp_text("hi", None, None), &clock());
    let done = parse_response("response.output_item.done",
        r#"{"item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}"#);
    reduce(&mut s, &done, &clock()); // canonical message; clears open_message
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::Completed), &clock());
    assert_eq!(s.items.len(), 1); // D-P1-12: no double
}

#[test]
fn reasoning_closed_finalizes_item_from_scratch() {
    let mut s = st();
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted), &clock());
    reduce(&mut s, &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "why".into() }), &clock());
    let closed = ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
        full_text: "why".into(), summary_text: "".into(),
    });
    reduce(&mut s, &closed, &clock());
    assert!(s.stream.open_reasoning.is_none());
    assert!(matches!(&s.items[0].kind, ItemKind::Reasoning { full_text, .. } if full_text == "why"));
}

#[test]
fn compaction_completed_pushes_item() {
    let mut s = st();
    let ev = ServerStreamEvent::Response(ResponseEvent::CompactionCompleted { total_tokens: Some(8421) });
    reduce(&mut s, &ev, &clock());
    assert!(matches!(s.items[0].kind, ItemKind::Compaction { token_count: Some(8421), .. }));
}

#[test]
fn agent_changed_pushes_transcript_marker_with_synthesized_from() {
    let mut s = st(); // agent_id = "ag" initially
    let u = reduce(&mut s, &ServerStreamEvent::Session(SessionEvent::AgentChanged {
        agent_id: "ag_2".into(), agent_name: "debby".into(),
    }), &clock());
    let marker = s.items.iter().find_map(|it| match &it.kind {
        ItemKind::AgentChanged { from, to, .. } => Some((from.as_str().to_string(), to.as_str().to_string())),
        _ => None,
    });
    assert_eq!(marker, Some(("ag".into(), "ag_2".into()))); // `from` synthesized from prior state
    assert!(u.contains(&StreamUpdate::AgentChanged));
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** finalizers in `items.rs` + wire routing:

> **REVIEW#2 — synthetic ids must be collision-free AND deterministic.** A clock-only id
> (`msg_local_<now>`) collides under the fixed-clock replay gate and under real same-millisecond
> events, so a later synthetic item would dedup-*update* an earlier one. Fix: derive the id from
> `state.items.len()` — strictly increasing across appends (P1 never removes items; dedup-updates
> reuse an existing id and never mint a synthetic), so it is unique *and* a pure function of the
> event sequence (deterministic under any clock). Helper:
>
> ```rust
> // items.rs — deterministic, collision-free local id for reducer-synthesized items.
> fn local_id(kind: &str, state: &SessionState) -> ItemId {
>     ItemId::new(format!("{kind}_local_{}", state.items.len()))
> }
> ```

```rust
// items.rs
use crate::domain::item::{ItemKind, ReasoningAcc};
use crate::domain::{AgentId, ContentBlock};

pub(crate) fn finalize_message(state: &mut SessionState, clock: &dyn Clock) -> Updates {
    let Some(acc) = state.stream.open_message.take() else { return smallvec![]; };
    // REVIEW#2: prefer the server message_id when present; else a collision-free local id.
    let id = acc.message_id.clone().map(ItemId::new).unwrap_or_else(|| local_id("msg", state));
    let kind = ItemKind::Message {
        role: Role::Assistant,
        content: vec![ContentBlock { kind: "output_text".into(), text: Some(acc.text), data: Value::Null }],
    };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn finalize_reasoning(state: &mut SessionState, clock: &dyn Clock) -> Updates {
    let Some(acc): Option<ReasoningAcc> = state.stream.open_reasoning.take() else { return smallvec![]; };
    let id = local_id("reasoning", state); // REVIEW#2
    let kind = ItemKind::Reasoning { full_text: acc.full_text, summary_text: acc.summary_text, encrypted: acc.encrypted };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn push_compaction(state: &mut SessionState, total_tokens: Option<i64>, clock: &dyn Clock) -> Updates {
    let id = local_id("compaction", state); // REVIEW#2
    let kind = ItemKind::Compaction { summary: String::new(), token_count: total_tokens.map(|t| t.max(0) as u64) };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn push_agent_changed(state: &mut SessionState, from: AgentId, to: AgentId, clock: &dyn Clock) -> Updates {
    let at = clock.now_millis();
    let id = local_id("agent_changed", state); // REVIEW#2 (was to+now → collides)
    push_item(state, id, ItemKind::AgentChanged { from, to, at }, None, clock)
}
```

Wire in `mod.rs`:
- `ResponseEvent::Completed`: **finalize FIRST, then bump turn** (REVIEW#1 — the finalized message
  must keep the *current* turn; bumping first stamps it with the next turn):
  ```rust
  let mut u = items::finalize_message(state, clock); // keeps current turn in its ctx
  state.stream.turn += 1;                            // next response's items get turn+1
  u.push(StreamUpdate::StatusChanged);
  u
  ```
  Add an assertion in `completed_bumps_turn...` that the finalized item's `ctx.turn` equals the
  pre-bump turn (0 on the first response), not the post-bump value.
- `ResponseEvent::ReasoningClosed { .. }`: `items::finalize_reasoning(state, clock)`.
- `ResponseEvent::CompactionCompleted { total_tokens }`: `items::push_compaction(state, *total_tokens, clock)`.
- In the `SessionEvent::AgentChanged` arm (folds.rs), before overwriting `state.agent_id`, capture `let from = state.agent_id.clone();` then after the scalar update call `items::push_agent_changed(state, from, to, clock)` and merge its `ItemAppended` into the returned updates. **This means `fold_session_field` needs the `clock`** — change its signature to `fold_session_field(state, ev, clock)` and thread the clock. (Task 2's callers update accordingly.)

> **Turn-bump note:** finalize the message at the *current* turn, THEN `turn += 1`, so the
> next response's items get the next turn number. Test `completed_bumps_turn...` asserts
> `turn == 1` after one completed.

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 8 — finalizers: message/reasoning/compaction/agent-changed`

---

## Task 9: Reconnect + bootstrap folds (`reduce/snapshot.rs`)

The crate-synthetic arms — **hand-authored synthetic fixtures** (not in the wire corpus).

**Files:** Modify `crates/lens-core/src/reduce/snapshot.rs`; route `Reconnecting`/`Reconnected`/`SnapshotRestored`/`Disconnected` in `mod.rs`.

**Interfaces:**
- Consumes: `lens_client::sessions::SessionSnapshot` (via its getters).
- Produces: `pub(crate) fn fold_snapshot(state, snap: &SessionSnapshot) -> Updates`; `pub(crate) fn on_reconnected(state, gap: Option<u64>) -> Updates`.

- [ ] **Step 1: Failing tests** — scratch clear on gap, snapshot scalar-only, no transcript side-effects:

```rust
#[test]
fn reconnected_with_gap_clears_scratch_not_pending_user() {
    let mut s = st();
    reduce(&mut s, &resp_text("partial", None, None), &clock()); // open_message set
    s.pending_user.push(/* a PendingUserMessage */ pending("p1", "hey"));
    let u = reduce(&mut s, &ServerStreamEvent::Reconnected { gap: None }, &clock());
    assert!(s.stream.open_message.is_none()); // cleared (D-P1-16)
    assert_eq!(s.pending_user.len(), 1);      // NOT cleared (spec P3b)
    assert!(u.contains(&StreamUpdate::Reconnected));
}

#[test]
fn reconnected_gap_zero_keeps_scratch() {
    let mut s = st();
    reduce(&mut s, &resp_text("partial", None, None), &clock());
    reduce(&mut s, &ServerStreamEvent::Reconnected { gap: Some(0) }, &clock());
    assert!(s.stream.open_message.is_some()); // provably contiguous ⇒ keep
}

#[test]
fn snapshot_restored_folds_scalars_only_no_items() {
    let mut s = st();
    let snap = snapshot_fixture(); // status=running, model=opus, agent_id=ag_9, has embedded items
    let u = reduce(&mut s, &ServerStreamEvent::SnapshotRestored(Box::new(snap)), &clock());
    assert_eq!(s.status, SessionStatusValue::Running);
    assert_eq!(s.llm_model.as_deref(), Some("opus"));
    assert_eq!(s.agent_id.as_str(), "ag_9");
    assert!(s.items.is_empty()); // D-P1-15: NO transcript side-effects
    assert!(!s.items.iter().any(|i| matches!(i.kind, ItemKind::AgentChanged { .. }))); // no marker
    assert_eq!(&u[..], &[StreamUpdate::SnapshotRestored]);
}
```

> **`snapshot_fixture()`:** `SessionSnapshot` has private fields and no public constructor —
> build it by `serde_json::from_value::<SessionSnapshot>(json!({...}))` if it derives
> `Deserialize` (it does — it is the typed read type). Include an `items` array to prove they
> are ignored. **RESOLVE IN REVIEW:** confirm `SessionSnapshot: Deserialize` is reachable and
> public from `lens_client::sessions`.

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** `snapshot.rs`:

```rust
use crate::domain::{AgentId, SessionState};
use crate::reduce::folds::normalize_status;
use crate::reduce::{StreamUpdate, Updates};
use lens_client::sessions::{SessionSnapshot, SessionStatus};
use smallvec::smallvec;

fn map_snapshot_status(s: SessionStatus) -> crate::domain::SessionStatusValue {
    use crate::domain::SessionStatusValue as V;
    match s { SessionStatus::Idle => V::Idle, SessionStatus::Running => V::Running, SessionStatus::Failed => V::Failed }
}

/// D-P1-15: scalar restore ONLY — no transcript side-effects, no AgentChanged marker.
pub(crate) fn fold_snapshot(state: &mut SessionState, snap: &SessionSnapshot) -> Updates {
    state.status = map_snapshot_status(snap.status());
    state.agent_id = AgentId::new(snap.agent_id().to_string());
    state.agent_name = snap.agent_name().map(str::to_string);
    state.stream.current_agent = state.agent_name.clone();
    state.llm_model = snap.llm_model().map(str::to_string);
    state.model_override = snap.model_override().map(str::to_string);
    state.reasoning_effort = snap.reasoning_effort().map(str::to_string);
    state.context_window = snap.context_window().map(|v| v.max(0) as u64);
    state.last_total_tokens = snap.last_total_tokens().map(|v| v.max(0) as u64);
    state.cumulative_cost.total_cost_usd = snap.total_cost_usd();
    state.title = snap.title().map(str::to_string);
    state.labels = snap.labels().clone();
    state.host_id = snap.host_id().map(|h| crate::domain::HostId::new(h.to_string()));
    state.runner_id = snap.runner_id().map(|r| crate::domain::RunnerId::new(r.to_string()));
    state.workspace = snap.workspace().map(str::to_string);
    state.git_branch = snap.git_branch().map(str::to_string);
    state.parent_session_id = snap.parent_session_id().map(|p| crate::domain::SessionId::new(p.to_string()));
    state.permission_level = snap.permission_level().and_then(|p| u8::try_from(p).ok());
    state.archived = snap.archived();
    // usage_by_model + skills fold — see D-P1-9; map each getter into domain types.
    state.cumulative_cost.cumulative_usage.usage_by_model = snap.usage_by_model().iter()
        .map(|(k, mu)| (k.clone(), crate::domain::ModelUsage {
            input_tokens: Some(mu.input_tokens().max(0) as u64),
            output_tokens: Some(mu.output_tokens().max(0) as u64),
            total_tokens: Some(mu.total_tokens().max(0) as u64),
            cache_creation_input_tokens: Some(mu.cache_creation_input_tokens().max(0) as u64),
            cache_read_input_tokens: Some(mu.cache_read_input_tokens().max(0) as u64),
            total_cost_usd: Some(mu.total_cost_usd()),
        })).collect();
    state.skills = snap.skills().iter().map(|sk| crate::domain::SkillSummary {
        name: sk.name().to_string(),
        description: Some(sk.description().to_string()).filter(|d| !d.is_empty()),
    }).collect();
    // NOTE: snap.items() is deliberately NOT read here (D-P1-15) — history is replayed as
    // subsequent OutputItemDone events by lens-client (§7 ordering).
    smallvec![StreamUpdate::SnapshotRestored]
}

pub(crate) fn on_reconnected(state: &mut SessionState, gap: Option<u64>) -> Updates {
    let mut u: Updates = smallvec![StreamUpdate::Reconnected];
    if gap != Some(0) {
        // D-P1-16: clear transient scratch; KEEP pending_user (user intent, spec P3b).
        let had = state.stream.open_message.is_some() || state.stream.open_reasoning.is_some()
            || !state.stream.unpaired_calls.is_empty();
        state.stream.open_message = None;
        state.stream.open_reasoning = None;
        state.stream.unpaired_calls.clear();
        if had { u.push(StreamUpdate::ScratchChanged); }
    }
    u
}
```

Route in `mod.rs`:

```rust
ServerStreamEvent::Reconnecting { attempt } => smallvec![StreamUpdate::Reconnecting { attempt: *attempt }],
ServerStreamEvent::Reconnected { gap } => snapshot::on_reconnected(state, *gap),
ServerStreamEvent::SnapshotRestored(snap) => snapshot::fold_snapshot(state, snap),
ServerStreamEvent::Disconnected { .. } => smallvec![StreamUpdate::Disconnected],
ServerStreamEvent::Unknown { .. } => smallvec![], // forward-compat: no-op
```

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 9 — reconnect + bootstrap snapshot folds`

---

## Task 10: §4.3 render transforms (`reduce/transforms.rs`)

Pure read-only projections over `&[Item]`. Framework-neutral, no mutation.

**Files:** Modify `crates/lens-core/src/reduce/transforms.rs`.

**Interfaces:**
- Produces: `pub fn hide_reasoning(items: &[Item]) -> Vec<&Item>`; `pub fn merge_text_for_display(items: &[Item]) -> Vec<Item>`; `pub fn only_agent<'a>(items: &'a [Item], agent: &str) -> Vec<&'a Item>`; `pub fn by_depth(items: &[Item]) -> BTreeMap<u32, Vec<&Item>>`; `pub fn with_agent_changed_markers(items: &[Item], keep: bool) -> Vec<&Item>`. (`flatten_sub_agents` is deferred — needs child-session topology, §9; stub returns the input with a `// P1-DECISION deferred` note and a test asserting identity passthrough.)

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn hide_reasoning_drops_reasoning_items() {
    let items = vec![msg_item("m1", "hi"), reasoning_item("r1"), msg_item("m2", "bye")];
    let out = hide_reasoning(&items);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|i| !matches!(i.kind, ItemKind::Reasoning { .. })));
}

#[test]
fn only_agent_filters_by_ctx() {
    let mut a = msg_item("m1", "x"); a.ctx.agent = Some("coder".into());
    let mut b = msg_item("m2", "y"); b.ctx.agent = Some("researcher".into());
    let items = vec![a, b];
    let out = only_agent(&items, "coder");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id.as_str(), "m1");
}

#[test]
fn with_agent_changed_markers_can_drop_them() {
    let items = vec![msg_item("m1","x"), agent_changed_item("ac1"), msg_item("m2","y")];
    assert_eq!(with_agent_changed_markers(&items, true).len(), 3);
    assert_eq!(with_agent_changed_markers(&items, false).len(), 2);
}
```

- [ ] **Step 2: Run — FAIL.**

- [ ] **Step 3: Implement** the transforms (all pure, no `SessionState`):

```rust
use crate::domain::item::{Item, ItemKind};
use std::collections::BTreeMap;

pub fn hide_reasoning(items: &[Item]) -> Vec<&Item> {
    items.iter().filter(|i| !matches!(i.kind, ItemKind::Reasoning { .. })).collect()
}

pub fn with_agent_changed_markers(items: &[Item], keep: bool) -> Vec<&Item> {
    items.iter().filter(|i| keep || !matches!(i.kind, ItemKind::AgentChanged { .. })).collect()
}

pub fn only_agent<'a>(items: &'a [Item], agent: &str) -> Vec<&'a Item> {
    items.iter().filter(|i| i.ctx.agent.as_deref() == Some(agent)).collect()
}

pub fn by_depth(items: &[Item]) -> BTreeMap<u32, Vec<&Item>> {
    let mut m: BTreeMap<u32, Vec<&Item>> = BTreeMap::new();
    for i in items { m.entry(i.ctx.depth).or_default().push(i); }
    m
}

/// Coalesce adjacent assistant `Message` items into one for display. Returns owned
/// clones (it synthesizes merged content) — the only transform that does.
pub fn merge_text_for_display(items: &[Item]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for it in items {
        if let (Some(prev), ItemKind::Message { role, content }) = (out.last_mut(), &it.kind) {
            if let ItemKind::Message { role: prole, content: pcontent } = &mut prev.kind {
                if prole == role {
                    pcontent.extend(content.iter().cloned());
                    continue;
                }
            }
        }
        out.push(it.clone());
    }
    out
}

/// DEFERRED (D-P1: sub-agent topology is §9). Identity passthrough in P1.
pub fn flatten_sub_agents(items: &[Item]) -> Vec<&Item> {
    items.iter().collect()
}
```

- [ ] **Step 4: Run — PASS.**

- [ ] **Step 5: Commit** `feat(lens-core): P1 task 10 — §4.3 render transforms`

---

## Task 11: Corpus replay determinism + full-event coverage + throughput bench (`reduce/mod.rs` tests + `benches/`)

The P1 gate: replay every captured `.stream.sse` through the public `EventStream` decode +
`reduce`, twice, and assert identical `SessionState`; assert no arm silently no-ops a modeled
event; measure reducer throughput.

**Files:** Add a `tests` submodule / integration test that loads the corpus; write `benches/reduce_throughput.rs`.

**Interfaces:**
- Consumes: the public lens-client stream-decode path. **RESOLVE IN REVIEW / TASK 1:** the reducer needs a `&[ServerStreamEvent]` from raw `.sse` bytes. Determine the reachable public API — either (a) `lens_client::stream::EventStream` fed from a byte reader, or (b) a `pub` decode helper. If neither is public, add a minimal `pub fn decode_all(bytes: &[u8]) -> Vec<ServerStreamEvent>` test helper to `lens-client` (a one-line wrapper over the existing `pub(crate)` `SseParser` + `parse_event`, gated `#[cfg(any(test, feature = "test-util"))]`). This is the single lens-client touch P1 may need; keep it out of `generated.rs`.

- [ ] **Step 1: Corpus determinism test**

```rust
#[test]
fn corpus_replay_is_deterministic() {
    for path in glob_corpus() { // helper: all *.stream.sse / *.sse under the two capture dirs
        let events = decode_corpus(&path);
        let mut a = fresh_state();
        let mut b = fresh_state();
        let clock = ManualClock::new(1_700_000_000_000);
        for ev in &events { reduce(&mut a, ev, &clock); }
        for ev in &events { reduce(&mut b, ev, &clock); }
        assert_eq!(a, b, "non-deterministic replay for {path:?}");
    }
}

#[test]
fn deferred_wire_type_is_a_noop() {
    // REVIEW#11: session.collaboration_mode is DEFERRED in lens-client (→ Unknown). The
    // reducer must no-op it and leave the domain field None until lens-client models it.
    let mut s = fresh_state();
    let clock = ManualClock::new(1_700_000_000_000);
    let u = reduce(&mut s, &ServerStreamEvent::Unknown {
        event_type: "session.collaboration_mode".into(),
    }, &clock);
    assert!(u.is_empty());
    assert_eq!(s.collaboration_mode, None);
}

#[test]
fn happy_path_produces_expected_transcript_shape() {
    let events = decode_corpus("docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse");
    let mut s = fresh_state();
    let clock = ManualClock::new(1_700_000_000_000);
    for ev in &events { reduce(&mut s, ev, &clock); }
    // The captured turn has a function_call + output + a final assistant message.
    assert!(s.items.iter().any(|i| matches!(i.kind, ItemKind::FunctionCall { .. })));
    assert!(s.items.iter().any(|i| matches!(i.kind, ItemKind::FunctionCallOutput { .. })));
    assert!(s.items.iter().any(|i| matches!(i.kind, ItemKind::Message { .. })));
    // No two items share an id (dedup held).
    let mut ids: Vec<_> = s.items.iter().map(|i| i.id.as_str().to_string()).collect();
    ids.sort(); let n = ids.len(); ids.dedup();
    assert_eq!(ids.len(), n, "duplicate item ids leaked");
}
```

Include the corpus files via `include_bytes!` or read at runtime from the repo path (tests run
from the crate dir — use a `CARGO_MANIFEST_DIR`-relative path up to the repo root). Prefer
`include_bytes!` for the two or three representative fixtures (`happy_path.stream.sse`,
`interrupt.stream.sse`, `reasoning_effort_high.stream.sse`, plus 2–3 live-recapture files that
exercise agent-switch, todos, child-session) to keep the test hermetic.

- [ ] **Step 2: Run — expect FAIL until `decode_corpus` exists; add it, then PASS.**

Run: `cargo test -p lens-core corpus`
Expected: PASS (deterministic; expected shape holds).

- [ ] **Step 3: Write the throughput bench `benches/reduce_throughput.rs`**

```rust
use criterion::{criterion_group, criterion_main, Criterion};
use lens_core::{reduce, ManualClock};
// decode a representative corpus file once, then time a full replay per iteration.

fn bench_full_replay(c: &mut Criterion) {
    let events = /* decode happy_path.stream.sse via the same helper */;
    let clock = ManualClock::new(1_700_000_000_000);
    c.bench_function("reduce_happy_path_full_replay", |b| {
        b.iter(|| {
            let mut s = fresh_state();
            for ev in &events { criterion::black_box(reduce(&mut s, ev, &clock)); }
        });
    });
}
criterion_group!(benches, bench_full_replay);
criterion_main!(benches);
```

- [ ] **Step 4: Run the bench once, record the baseline**

Run: `cargo bench -p lens-core`
Expected: completes; record ns/replay in the commit body (AGENTS.md benchmark-or-it's-not-done). The reducer is pure CPU — expect low-µs per full turn; the frame budget (§performance) has ample headroom.

- [ ] **Step 5: Full gate + commit**

Run: `cargo test -p lens-core && cargo clippy -p lens-core --all-targets && cargo fmt --check`
Expected: PASS, zero warnings.

```bash
git add crates/lens-core
git commit -m "test(lens-core): P1 task 11 — corpus replay determinism + throughput bench"
```

---

## Self-Review (run against the spec §4 P1 before build)

**Spec coverage:**
- text accumulation → Task 5; finalize on completed → Task 8. ✓
- tool pairing by call_id → Task 7 (separate items; `unpaired_calls` scratch tracked). ✓ *(Note: P1 keeps call/output as separate items paired at render per §2.3 — the `unpaired_calls` map is populated but pairing is a render concern; flag whether P1 must populate it or defer to render.)*
- reasoning bracketing → Task 6 (acc) + Task 8 (close). ✓
- BlockContext attribution → Task 7 (`current_ctx`, depth=0 flagged). ✓
- identity/ordering/dedup by id → Task 7. ✓
- session-field folds (status/usage/todos/model/model_options/reasoning_effort/collaboration_mode/skills/elicitation/child/presence/sandbox/terminal_pending/agent_changed) → Tasks 2–4, 6, 8. ✓ *(collaboration_mode: `SessionEvent` has NO collaboration_mode variant in lens-client — it is a DEFERRED wire type. So P1 cannot fold it. FLAG: the domain field `collaboration_mode` stays `None` until lens-client models `session.collaboration_mode`.)*
- AgentChanged item insertion (synthesize from) → Task 8. ✓
- SnapshotRestored scalar-only fold → Task 9. ✓
- normalization (two status vocabularies + two usage reps) → Task 2 (`normalize_status`) + Task 3 + Task 9. ✓
- §4.3 render transforms → Task 10. ✓
- determinism gate → Task 11. ✓

**Gaps surfaced by self-review — ALL RESOLVED in the cross-family (codex/gpt-5.5) plan review:**
1. **`collaboration_mode` is un-foldable in P1** (REVIEW#11) — `session.collaboration_mode` is in
   `DEFERRED_EVENT_TYPES` (lens-client routes it to `Unknown`). The domain field stays `None`.
   **Add a guard test** (Task 11 coverage): `reduce` of
   `ServerStreamEvent::Unknown { event_type: "session.collaboration_mode".into() }` returns `[]`
   and leaves `collaboration_mode == None`. Documented in the widening backlog.
2. **`unpaired_calls` population** (REVIEW#12) — **RESOLVED: leave it empty in P1.** Call/output
   pairing is a pure-render concern over `&[Item]` (scan by `call_id`); the reducer does not
   populate the `CallId → ItemId` map. The field stays for a future render-time optimization.
3. **Byte-level test reachability** (REVIEW#10) — **RESOLVED:** a `test-util`-gated
   `pub fn decode_all(&[u8]) -> Vec<ServerStreamEvent>` in lens-client (Task 1b). `SessionSnapshot`
   derives `Deserialize` and is public, so snapshot fixtures build via `serde_json::from_value`.

**Placeholder scan:** the `parse_session`/`parse_response`/`snapshot_fixture`/`fresh_state`
helpers are now DEFINED in the "Shared test helpers" section (over the `decode_all` seam);
`glob_corpus`/`decode_corpus` are defined in Task 11 (also over `decode_all`). No unresolved
placeholder remains — every named helper has a concrete definition in the plan.

**Type consistency:** `Updates = SmallVec<[StreamUpdate; 2]>` used uniformly; `reduce` sig
stable across tasks; `push_item`/`map_item`/`finalize_*` names consistent; `fold_session_field`
gains a `clock` param in Task 8 (noted at both the definition and the call sites).

---

## Handoff notes for P2/P3 (recorded at P1 exit)

- **`StreamUpdate` is a P1 draft** (spec D6) — ratify the apply-side representation at the P3
  walking skeleton; markers may gain payloads or become a shared-snapshot re-read.
- **lens-client wrapper-widening backlog** (contract-proving output): `stream::Item` needs
  `native_tool`/`slash_command`/`terminal_command`/full-resource payloads; `stream::PresenceViewer`
  needs `joined_at`/`idle`; `SessionEvent` needs `session.collaboration_mode` + a payload-bearing
  `Skills`/`ResourceCreated`. Until then P1 degrades those (NativeTool catch-all, empty presence
  meta, `None` collaboration_mode, marker-only resources/skills). Each is flagged in-code.
- **`depth` is fixed at 0** — sub-agent depth + `flatten_sub_agents` need child-session topology
  (§9 registry). Deferred.
- **Optimistic-send reconcile** (`InputConsumed`/`Superseded`, spec §7/P3b) is a P3 actor
  concern — P1 folds them to markers only and does NOT touch `pending_user`.
