# lens-core §3 ActorFeed Gate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the lens-core §3.1–§3.4 ActorFeed merge gate — one FIFO `ActorFeed` channel, scheduler dual-mode spawn-in-Summary, seed-on-spawn + emit-on-Demote, and enriched `SummaryUpdate` — as a separately-reviewed, separately-merged milestone before any lens-ui view code.

**Architecture:** Today `ActorOutput` holds two independently-buffered senders (`updates` + `summaries`). Catch-up and deferred-commit emit `TranscriptAdvanced` on `updates` regardless of `OutputMode`, and `Promote` emits `Rebased` on `updates` before flipping mode — so a Summary-mode actor with nonempty catch-up legitimately interleaves the two streams and a lagging consumer can reorder them. Merge to one `async_channel::Sender<ActorFeed>`; enrich `SummaryUpdate` for card chrome (incl. `last_completed_turn` + snapshot-folded `harness`); spawn background sessions directly in `Summary`; seed after catch-up and emit on `Demote`.

**Tech Stack:** Rust, gpui 0.2.2, async_channel, crossbeam_channel, rusqlite; lens-core actor model.

## Global Constraints

- **One-way-door merge gate:** this milestone lands **separately reviewed + separately merged BEFORE any lens-ui / lens-app view code**. Reversing the public channel/struct later is expensive.
- **Scope = design §3.1–§3.4 ONLY.** Do **not** implement §3.5 Ready policy, FleetStore, poller, SessionCard, board, slot API, FakeFleet, or the §6.1 acceptance test.
- **Gate command:** `cargo clippy --workspace --all-targets -- -D warnings` must be clean; also `cargo fmt --check`.
- **`generated.rs` is untouched** — never hand-edit codegen.
- **`lens-drive` green is necessary but not sufficient** — it is Detailed-only; gate evidence requires the new lens-core Summary/interleave tests in Task 5.
- **Cross-family + Opus review required** on the final diff (actor-touching, Opus-level). Author implementation via composer-2.5; review from a different model family.
- **TDD every code change:** failing test → run (expect FAIL) → minimal impl → run (expect PASS) → commit.
- **Injected `Clock` only** in actor/reduce paths; no wall-clock in tests.
- **Capacity is the caller's choice** — recommend `async_channel::bounded(64)`; actor accepts `Sender` and uses `send_blocking` (lossless backpressure).
- **Spec SSOT:** `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md` §3.1–§3.4, §7, §9, Appendix A rounds 4/5/6. Where this plan and the spec disagree, the spec wins — surface the conflict.

---

## File Structure

| Path | Role |
| --- | --- |
| `crates/lens-core/src/actor/feed.rs` | **Create.** `enum ActorFeed { Summary(SummaryUpdate), Detailed(StreamUpdate) }`. |
| `crates/lens-core/src/actor/mod.rs` | Re-export `ActorFeed`; keep existing re-exports. |
| `crates/lens-core/src/actor/summary.rs` | Enrich `SummaryUpdate` + `from_state` (+ `activity_summary` helper). |
| `crates/lens-core/src/domain/session.rs` | Add `harness: Option<String>` (+ `SessionState::new` init). |
| `crates/lens-core/src/reduce/snapshot.rs` | Fold `snap.harness()` into `state.harness` in `fold_snapshot`. |
| `crates/lens-core/src/actor/runloop.rs` | `ActorOutput.feed`; every emit site; spawn signatures; seed + Demote emit; test churn. |
| `crates/lens-core/src/actor/scheduler.rs` | `wake`/`reconnect` take `feed` + `initial_mode`; call `spawn_actor_dual`; test churn. |
| `crates/lens-drive/src/main.rs` | Build `bounded(64)` `ActorFeed` channel; match `ActorFeed::Detailed` in drain. |

**`ActorFeed` home justification:** new `actor/feed.rs`, re-exported from `actor/mod.rs`. Mirrors `summary.rs` (a small bridge-type module) and keeps the 4849-line `runloop.rs` from owning another public type. Not in `reduce/` — `ActorFeed` is an actor→foreground bridge enum, not a reducer output.

**Harness fold site (verified):** `fold_snapshot` in `crates/lens-core/src/reduce/snapshot.rs:20` — add the assignment next to the other scalar folds (after `state.agent_name = …` at line 23 is the natural spot). Wire API: `SessionSnapshot::harness(&self) -> &str` (`lens-client/src/sessions.rs:195`; field is `String` with `#[serde(default)]`, not `Option`).

**Harness persistence (deliberate omission):** do **not** add a `sessions.harness` control-store column or bump `SCHEMA_VERSION` in this gate. `CREATE TABLE IF NOT EXISTS` would not ALTER existing DBs anyway. Harness is RAM + snapshot-fold; after `wake`/`reconnect` from disk it is `None` until the next `SnapshotRestored`. Card chrome still works once the live stream delivers a snapshot. Control-store persist can follow later if wake-before-snapshot chrome needs it.

---

## Source facts locked for implementers

Verified against source on 2026-07-15:

| Fact | Location |
| --- | --- |
| `ActorOutput { updates, summaries, outcomes, mode }` | `runloop.rs:91-96` |
| `spawn_actor` creates `sum_tx` bounded(1) and drops `_sum_rx` | `runloop.rs:126-136` |
| Catch-up emits `TranscriptAdvanced` on `.updates` regardless of mode | `runloop.rs:416-424` |
| Deferred commit emits on `.updates` regardless of mode | `runloop.rs:804-810` |
| `Promote` sends `Rebased` on `.updates` then sets `Detailed` | `runloop.rs:523-532` |
| `Demote` only flips mode (no emit) | `runloop.rs:534-536` |
| Mode-exclusive main-batch emit | `runloop.rs:682-711` |
| `emit_pending_user` Summary path uses `.summaries` | `runloop.rs:1064-1077` |
| `run()` = catch-up then select (no seed) | `runloop.rs:1019-1060` |
| `stream.turn` bumped on `ResponseEvent::Completed` | `reduce/mod.rs:132-136` |
| `SummaryUpdate` today: 6 fields | `summary.rs:10-17` |
| `SessionState` fields used by enrichment (exact names): `llm_model`, `model_override`, `agent_name`, `cumulative_cost: Cost`, `context_window`, `last_total_tokens`, `sandbox_status: Option<SandboxStatus>`, `git_branch`, `workspace`, `reasoning_effort`, `todos: Vec<Todo>` (`active_form`, not `activeForm`), `stream.turn: u32` | `domain/session.rs`, `domain/controls.rs`, `domain/item.rs` |
| `FleetScheduler::wake`/`reconnect` call `spawn_actor` (hardcoded Detailed) | `scheduler.rs:43-106` |
| `ActorOutcome::SummaryConsumerGone` | `outcome.rs:28` |

**Semantic change after the merge:** the old tests that Demote under `spawn_actor` (summary rx dropped, updates rx live) and assert survival/`SummaryConsumerGone` while Promote still works are **impossible** with one channel — dropping the feed drops both projections. Rewrite those tests: Demote emits `ActorFeed::Summary` on the live feed; `SummaryConsumerGone` is exercised by dropping the **entire** feed receiver.

---

### Task 1: Enrich `SummaryUpdate` + `SessionState.harness` (§3.4)

**Files:**
- Modify: `crates/lens-core/src/domain/session.rs` (`SessionState` fields + `new`)
- Modify: `crates/lens-core/src/reduce/snapshot.rs:20-88` (`fold_snapshot`)
- Modify: `crates/lens-core/src/actor/summary.rs` (struct + `from_state`)
- Test: `crates/lens-core/src/actor/summary.rs` (new `#[cfg(test)]` module) and `crates/lens-core/src/reduce/snapshot.rs` (extend existing tests)

**Interfaces:**
- Consumes: `SessionState` fields listed above; `SessionSnapshot::harness(&self) -> &str`; `TodoStatus`, `ItemKind::FunctionCall { name, .. }`, `Cost`, `SandboxStatus`.
- Produces:
  - `SessionState.harness: Option<String>`
  - `SummaryUpdate` gains: `llm_model: Option<String>`, `model_override: Option<String>`, `agent_name: Option<String>`, `cumulative_cost: Cost`, `context_window: Option<u64>`, `sandbox_status: Option<SandboxStatus>`, `git_branch: Option<String>`, `workspace: Option<String>`, `reasoning_effort: Option<String>`, `activity_summary: String`, `last_completed_turn: u32`, `harness: Option<String>` (existing 6 fields retained).
  - `SummaryUpdate::from_state(s: &SessionState) -> Self` copies all of the above; `last_completed_turn = s.stream.turn`.
  - `fn activity_summary(s: &SessionState) -> String` (private in `summary.rs`): first `TodoStatus::InProgress` → that todo's `active_form`; else first in-flight tool name from `s.stream.unpaired_calls` / matching `ItemKind::FunctionCall`; else `String::new()`.

- [ ] **Step 1: Write the failing enrichment + harness-fold tests**

Add to `summary.rs` (new test module) and extend `snapshot.rs` tests:

```rust
// crates/lens-core/src/actor/summary.rs — append:
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::controls::{SandboxStatus, Todo, TodoStatus};
    use crate::domain::ids::{AgentId, CallId, ConnectionId, ItemId, SessionId};
    use crate::domain::item::{BlockContext, Item, ItemKind};
    use crate::domain::usage::Cost;
    use std::sync::Arc;

    fn base() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }

    #[test]
    fn from_state_copies_card_chrome_and_last_completed_turn() {
        let mut s = base();
        s.llm_model = Some("opus".into());
        s.model_override = Some("sonnet".into());
        s.agent_name = Some("coder".into());
        s.cumulative_cost = Cost {
            total_cost_usd: Some(1.25),
            ..Cost::default()
        };
        s.context_window = Some(200_000);
        s.last_total_tokens = Some(12_000);
        s.sandbox_status = Some(SandboxStatus {
            stage: "ready".into(),
            detail: None,
        });
        s.git_branch = Some("main".into());
        s.workspace = Some("/tmp/proj".into());
        s.reasoning_effort = Some("high".into());
        s.harness = Some("claude-native".into());
        s.stream.turn = 7;
        s.todos.push(Todo {
            content: "wire feed".into(),
            status: TodoStatus::InProgress,
            active_form: "wiring the feed".into(),
        });

        let u = SummaryUpdate::from_state(&s);
        assert_eq!(u.llm_model.as_deref(), Some("opus"));
        assert_eq!(u.model_override.as_deref(), Some("sonnet"));
        assert_eq!(u.agent_name.as_deref(), Some("coder"));
        assert_eq!(u.cumulative_cost.total_cost_usd, Some(1.25));
        assert_eq!(u.context_window, Some(200_000));
        assert_eq!(u.last_total_tokens, Some(12_000));
        assert_eq!(u.sandbox_status.as_ref().map(|sb| sb.stage.as_str()), Some("ready"));
        assert_eq!(u.git_branch.as_deref(), Some("main"));
        assert_eq!(u.workspace.as_deref(), Some("/tmp/proj"));
        assert_eq!(u.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(u.harness.as_deref(), Some("claude-native"));
        assert_eq!(u.last_completed_turn, 7);
        assert_eq!(u.activity_summary, "wiring the feed");
        assert_eq!(u.status, s.status);
        assert_eq!(u.title, s.title);
        assert_eq!(u.host_id, s.host_id);
        assert!(!u.needs_attention);
        assert!(!u.subagent_active);
    }

    #[test]
    fn activity_summary_falls_back_to_in_flight_tool_name() {
        let mut s = base();
        let call_id = CallId::new("call_1");
        let item_id = ItemId::new("fc_1");
        s.stream.unpaired_calls.insert(call_id.clone(), item_id.clone());
        s.items.push(Arc::new(Item {
            id: item_id,
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1,
            kind: ItemKind::FunctionCall {
                call_id,
                name: "bash".into(),
                arguments: serde_json::json!({}),
                status: "in_progress".into(),
                agent_name: None,
            },
        }));
        assert_eq!(SummaryUpdate::from_state(&s).activity_summary, "bash");
    }
}
```

```rust
// crates/lens-core/src/reduce/snapshot.rs — add test:
#[test]
fn fold_snapshot_copies_harness() {
    let mut s = crate::reduce::testutil::fresh_state();
    let snap = crate::reduce::testutil::snapshot_fixture(serde_json::json!({
        "id": "conv_1",
        "status": "running",
        "agent_id": "ag_1",
        "created_at": 1_700_000_000,
        "harness": "claude-sdk",
        "items": []
    }));
    super::fold_snapshot(&mut s, &snap);
    assert_eq!(s.harness.as_deref(), Some("claude-sdk"));
}
```

- [ ] **Step 2: Run tests — expect FAIL**

Run:
```bash
cargo test -p lens-core --lib actor::summary::tests::from_state_copies_card_chrome_and_last_completed_turn -- --nocapture
cargo test -p lens-core --lib reduce::snapshot::tests::fold_snapshot_copies_harness -- --nocapture
```
Expected: FAIL compile errors — `SummaryUpdate` missing fields / `SessionState` has no `harness` / `from_state` does not set the new fields.

- [ ] **Step 3: Minimal implementation**

`domain/session.rs` — add field + init:

```rust
// on SessionState, under Workspace & host (after sandbox_status is fine):
pub harness: Option<String>,

// in SessionState::new:
harness: None,
```

`reduce/snapshot.rs` — inside `fold_snapshot`, after `state.agent_name = …`:

```rust
state.harness = {
    let h = snap.harness();
    if h.is_empty() {
        None
    } else {
        Some(h.to_string())
    }
};
```

`actor/summary.rs` — replace struct + `from_state`:

```rust
use crate::domain::SessionState;
use crate::domain::controls::{SandboxStatus, TodoStatus};
use crate::domain::ids::HostId;
use crate::domain::item::ItemKind;
use crate::domain::scalars::SessionStatusValue;
use crate::domain::usage::Cost;

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryUpdate {
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub last_total_tokens: Option<u64>,
    pub host_id: Option<HostId>,
    pub needs_attention: bool,
    pub subagent_active: bool,
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub agent_name: Option<String>,
    pub cumulative_cost: Cost,
    pub context_window: Option<u64>,
    pub sandbox_status: Option<SandboxStatus>,
    pub git_branch: Option<String>,
    pub workspace: Option<String>,
    pub reasoning_effort: Option<String>,
    pub activity_summary: String,
    pub last_completed_turn: u32,
    pub harness: Option<String>,
}

fn activity_summary(s: &SessionState) -> String {
    if let Some(todo) = s
        .todos
        .iter()
        .find(|t| t.status == TodoStatus::InProgress)
    {
        return todo.active_form.clone();
    }
    for item_id in s.stream.unpaired_calls.values() {
        if let Some(item) = s.items.iter().find(|i| &i.id == item_id)
            && let ItemKind::FunctionCall { name, .. } = &item.kind
        {
            return name.clone();
        }
    }
    String::new()
}

impl SummaryUpdate {
    pub fn from_state(s: &SessionState) -> Self {
        Self {
            status: s.status,
            title: s.title.clone(),
            last_total_tokens: s.last_total_tokens,
            host_id: s.host_id.clone(),
            needs_attention: !s.pending_elicitations.is_empty()
                || s.status == SessionStatusValue::Failed,
            // TODO(§9): derive from child-session registry once it exists.
            subagent_active: false,
            llm_model: s.llm_model.clone(),
            model_override: s.model_override.clone(),
            agent_name: s.agent_name.clone(),
            cumulative_cost: s.cumulative_cost.clone(),
            context_window: s.context_window,
            sandbox_status: s.sandbox_status.clone(),
            git_branch: s.git_branch.clone(),
            workspace: s.workspace.clone(),
            reasoning_effort: s.reasoning_effort.clone(),
            activity_summary: activity_summary(s),
            last_completed_turn: s.stream.turn,
            harness: s.harness.clone(),
        }
    }
}
```

Fix any `SessionState { … }` struct literals / `serde` roundtrip tests that construct every field (the `new()` path is covered; `populated_session_roundtrips` uses `new` then mutates — OK). If a test constructs `SessionState` with a full literal, add `harness: None`.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p lens-core --lib actor::summary::tests -- --nocapture
cargo test -p lens-core --lib reduce::snapshot::tests::fold_snapshot_copies_harness -- --nocapture
cargo test -p lens-core --lib domain::session -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add \
  crates/lens-core/src/domain/session.rs \
  crates/lens-core/src/reduce/snapshot.rs \
  crates/lens-core/src/actor/summary.rs
git commit -m "$(cat <<'EOF'
feat(lens-core): enrich SummaryUpdate and fold harness (§3.4)

EOF
)"
```

---

### Task 2: Introduce `ActorFeed` and merge the two senders (§3.1)

**Files:**
- Create: `crates/lens-core/src/actor/feed.rs`
- Modify: `crates/lens-core/src/actor/mod.rs`
- Modify: `crates/lens-core/src/actor/runloop.rs` (`ActorOutput`, `spawn_actor`, `spawn_actor_dual`, every `.updates`/`.summaries` emit, all `#[cfg(test)]` recv sites)
- Modify: `crates/lens-core/src/actor/scheduler.rs` (signatures + tests — still pass `OutputMode::Detailed` via a temporary hardcoded path until Task 3)
- Modify: `crates/lens-drive/src/main.rs` (compile-fix only: feed channel + `ActorFeed::Detailed` match)

**Interfaces:**
- Consumes: `SummaryUpdate` (Task 1), `StreamUpdate` (`reduce/update.rs`).
- Produces:
  - `pub enum ActorFeed { Summary(SummaryUpdate), Detailed(StreamUpdate) }` in `actor/feed.rs`, re-exported from `actor/mod.rs`.
  - `ActorOutput { feed: async_channel::Sender<ActorFeed>, outcomes: …, mode: OutputMode }` (no `updates`/`summaries`).
  - `spawn_actor(state, events, feed, stores, clock, api) -> ActorHandle` — Detailed convenience; **no** internal `sum_tx` drop hack.
  - `spawn_actor_dual(state, events, feed, mode, stores, clock, api) -> ActorHandle` — single `feed` sender (drops the old separate `summaries` parameter).
  - Emit mapping: `.updates.send_blocking(u)` → `.feed.send_blocking(ActorFeed::Detailed(u))`; `.summaries.send_blocking(s)` → `.feed.send_blocking(ActorFeed::Summary(s))`.
  - `FleetScheduler::wake`/`reconnect` take `feed: async_channel::Sender<ActorFeed>` (still spawn Detailed until Task 3 adds `initial_mode`).

- [ ] **Step 1: Write a failing compile-anchored unit test for `ActorFeed`**

Create `feed.rs` first as a stub that does not yet exist — the test lives in `feed.rs`:

```rust
//! Unified actor → foreground feed (§3.1). One FIFO preserves send-order across
//! Summary/Detailed interleaves (catch-up TranscriptAdvanced, Promote Rebased).

use crate::actor::summary::SummaryUpdate;
use crate::reduce::StreamUpdate;

#[derive(Clone, Debug, PartialEq)]
pub enum ActorFeed {
    Summary(SummaryUpdate),
    Detailed(StreamUpdate),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, ConnectionId, HostId, SessionId};
    use crate::domain::scalars::SessionStatusValue;
    use crate::domain::session::SessionState;
    use crate::domain::usage::Cost;

    #[test]
    fn feed_variants_wrap_existing_bridge_types() {
        let s = SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        );
        let summary = ActorFeed::Summary(SummaryUpdate::from_state(&s));
        let detailed = ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: 0,
        });
        assert!(matches!(summary, ActorFeed::Summary(_)));
        assert!(matches!(
            detailed,
            ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })
        ));
        if let ActorFeed::Summary(u) = summary {
            assert_eq!(u.last_completed_turn, 0);
            assert!(u.activity_summary.is_empty());
            assert_eq!(u.cumulative_cost, Cost::default());
            assert_eq!(u.status, SessionStatusValue::Idle);
            assert!(u.host_id.is_none());
            let _ = HostId::new("unused"); // documents HostId remains on SummaryUpdate
        }
    }
}
```

Wire the module in `actor/mod.rs`:

```rust
mod api;
mod feed;
mod outcome;
mod runloop;
mod scheduler;
mod summary;
mod transport;

pub use api::{ClientSessionApi, CommandOutcome, SessionApi};
pub use feed::ActorFeed;
pub use outcome::ActorOutcome;
pub use runloop::{
    ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor, spawn_actor_dual,
};
pub use scheduler::{FleetScheduler, FleetSchedulerError};
pub use summary::SummaryUpdate;
pub use transport::{ActorTransport, ParkReason};
```

- [ ] **Step 2: Run the feed unit test — expect PASS for the enum alone**

```bash
cargo test -p lens-core --lib actor::feed::tests::feed_variants_wrap_existing_bridge_types -- --nocapture
```
Expected: PASS (enum + Task 1 types only). The rest of this task is the merge that will break the workspace until Step 3 completes.

- [ ] **Step 3: Merge `ActorOutput` + spawn + every emit site**

Replace `ActorOutput` and spawn helpers in `runloop.rs`:

```rust
struct ActorOutput {
    feed: async_channel::Sender<ActorFeed>,
    outcomes: async_channel::Sender<ActorOutcome>,
    mode: OutputMode,
}

/// Spawn the actor thread in `Detailed` mode.
pub fn spawn_actor(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> ActorHandle {
    spawn_actor_dual(
        state,
        events,
        feed,
        OutputMode::Detailed,
        stores,
        clock,
        api,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_actor_dual(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    mode: OutputMode,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> ActorHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<SessionCommand>(64);
    let (out_tx, out_rx) = async_channel::bounded(64);
    let join = std::thread::Builder::new()
        .name("lens-actor".into())
        .spawn(move || {
            run(
                state,
                events,
                cmd_rx,
                ActorOutput {
                    feed,
                    outcomes: out_tx,
                    mode,
                },
                stores,
                clock,
                api,
            )
        })
        .expect("actor thread");
    ActorHandle {
        commands: cmd_tx,
        outcomes: out_rx,
        join,
    }
}
```

Add `use crate::actor::feed::ActorFeed;` at the top of `runloop.rs`.

**Emit-site rewrites (exact current lines → new):**

1. Catch-up (`~416-424`):
```rust
if wrote_any
    && output
        .feed
        .send_blocking(ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: *next_ordinal - 1,
        }))
        .is_err()
{
    return CatchupResult::Aborted;
}
```

2. `Promote` (`~523-532`):
```rust
SessionCommand::Promote => {
    if output
        .feed
        .send_blocking(ActorFeed::Detailed(StreamUpdate::Rebased(
            scalars_baseline(state),
        )))
        .is_err()
    {
        return LoopControl::Break;
    }
    output.mode = OutputMode::Detailed;
    LoopControl::Continue
}
```

3. `apply_reduced_batch` Detailed arm (`~682-700`):
```rust
OutputMode::Detailed => {
    let had_snapshot = batch
        .iter()
        .any(|u| matches!(u, StreamUpdate::SnapshotRestored(_)));
    for u in coalesce(batch) {
        if ctx
            .output
            .feed
            .send_blocking(ActorFeed::Detailed(u))
            .is_err()
        {
            return (LoopControl::Break, false);
        }
    }
    if had_snapshot
        && ctx
            .output
            .feed
            .send_blocking(ActorFeed::Detailed(StreamUpdate::Rebased(
                scalars_baseline(ctx.state),
            )))
            .is_err()
    {
        return (LoopControl::Break, false);
    }
}
```

4. `apply_reduced_batch` Summary arm (`~702-711`):
```rust
OutputMode::Summary => {
    if ctx
        .output
        .feed
        .send_blocking(ActorFeed::Summary(SummaryUpdate::from_state(ctx.state)))
        .is_err()
    {
        ctx.ring.push(ActorOutcome::SummaryConsumerGone);
    }
}
```

5. `finish_deferred_transcript_commit` (`~804-810`):
```rust
if let Some(ord) = commit_terminal_prefix(stores, state, next_ordinal, ring)
    && output
        .feed
        .send_blocking(ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: ord,
        }))
        .is_err()
{
    return LoopControl::Break;
}
```

6. `emit_pending_user` (`~1064-1077`):
```rust
fn emit_pending_user(output: &ActorOutput, state: &SessionState) -> bool {
    match output.mode {
        OutputMode::Detailed => output
            .feed
            .send_blocking(ActorFeed::Detailed(StreamUpdate::PendingUserChanged(
                state.pending_user.clone(),
            )))
            .is_ok(),
        OutputMode::Summary => {
            let _ = output
                .feed
                .send_blocking(ActorFeed::Summary(SummaryUpdate::from_state(state)));
            true
        }
    }
}
```

**Mechanical test churn pattern** (apply to every `spawn_actor` / `spawn_actor_dual` call site in `runloop.rs` tests — ~60+ sites):

```rust
// BEFORE
let (up_tx, up_rx) = async_channel::bounded(64);
let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);
// … up_rx.recv_blocking() → StreamUpdate::…

// AFTER
let (feed_tx, feed_rx) = async_channel::bounded(64);
let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);
// unwrap Detailed:
match feed_rx.recv_blocking().unwrap() {
    ActorFeed::Detailed(u) => { /* former StreamUpdate match on u */ }
    other => panic!("expected Detailed, got {other:?}"),
}
```

```rust
// BEFORE spawn_actor_dual
let (up_tx, up_rx) = async_channel::bounded(64);
let (sum_tx, sum_rx) = async_channel::bounded(64);
let handle = spawn_actor_dual(state, ev_rx, up_tx, sum_tx, OutputMode::Summary, …);

// AFTER
let (feed_tx, feed_rx) = async_channel::bounded(64);
let handle = spawn_actor_dual(state, ev_rx, feed_tx, OutputMode::Summary, …);
// Summary frames:
match feed_rx.recv_blocking().unwrap() {
    ActorFeed::Summary(u) => { /* … */ }
    other => panic!("expected Summary, got {other:?}"),
}
```

**Rewrite obsolete dual-channel survival tests:**

- `demote_on_detailed_only_handle_does_not_kill_actor` — Demote now emits `ActorFeed::Summary` on the same feed; then Promote emits `ActorFeed::Detailed(Rebased(_))`. Assert both arrive in order; actor joins cleanly.
- `demote_then_send_works_in_summary_mode` second half (`spawn_actor` + Demote with "no summary consumer") — replace with: drop `feed_rx`, spawn Summary-mode actor, send an event, assert `ActorOutcome::SummaryConsumerGone` (and/or that the actor still accepts `Stop`). Do **not** assert Promote-still-works-after-partial-drop — that scenario no longer exists.

Helper for tests that drain Detailed-only:

```rust
fn recv_detailed(feed_rx: &async_channel::Receiver<ActorFeed>) -> StreamUpdate {
    match feed_rx.recv_blocking().unwrap() {
        ActorFeed::Detailed(u) => u,
        other => panic!("expected Detailed, got {other:?}"),
    }
}
```

**Scheduler compile fix (Task 2 slice — still Detailed-only):**

```rust
use crate::actor::feed::ActorFeed;
use crate::actor::runloop::{ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor};

pub fn wake(
    &mut self,
    conn: &ConnectionId,
    session_id: &SessionId,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> Result<(), FleetSchedulerError> {
    // … load state unchanged …
    let handle = spawn_actor(state, events, feed, stores, clock, api);
    self.registry.insert(session_id.clone(), handle);
    Ok(())
}

// reconnect: same `feed: async_channel::Sender<ActorFeed>` replacement for `updates`.
```

Update scheduler tests: `up_tx`/`up_rx` → `feed_tx`/`feed_rx`; match `ActorFeed::Detailed(StreamUpdate::…)`.

**lens-drive compile fix:**

```rust
use lens_core::actor::ActorFeed;
// …
let (feed_tx, feed_rx) = async_channel::bounded(64);
// pass feed_tx into attach_actor / reconnect_command
// …
fn drain_updates(
    rx: async_channel::Receiver<ActorFeed>,
    gate: &SnapshotGate,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(ActorFeed::Detailed(update)) => match update {
                StreamUpdate::Rebased(state) => {
                    print_state_line(&state);
                    gate.signal_if_waiting();
                }
                StreamUpdate::TranscriptAdvanced { committed_ordinal } => {
                    print_transcript_advanced_line(committed_ordinal)
                }
                _ => {}
            },
            Ok(ActorFeed::Summary(_)) => {
                // lens-drive is Detailed-only; ignore Summary frames if any appear.
            }
            Err(async_channel::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(async_channel::TryRecvError::Closed) => return,
        }
    }
}
```

Update `attach_actor` / `reconnect_command` parameter types from `Sender<StreamUpdate>` to `Sender<ActorFeed>`.

- [ ] **Step 4: Run lens-core + lens-drive tests — expect PASS**

```bash
cargo test -p lens-core --lib -- --nocapture
cargo test -p lens-drive -- --nocapture
```
Expected: PASS. (`lens-drive` may have no unit tests — compile check via `cargo check -p lens-drive` is enough if so.)

- [ ] **Step 5: Commit**

```bash
git add \
  crates/lens-core/src/actor/feed.rs \
  crates/lens-core/src/actor/mod.rs \
  crates/lens-core/src/actor/runloop.rs \
  crates/lens-core/src/actor/scheduler.rs \
  crates/lens-drive/src/main.rs
git commit -m "$(cat <<'EOF'
feat(lens-core): unify actor bridge on ActorFeed (§3.1)

EOF
)"
```

---

### Task 3: Scheduler dual-mode + spawn-in-Summary (§3.2)

**Files:**
- Modify: `crates/lens-core/src/actor/scheduler.rs:43-106` (`wake`/`reconnect`) + tests
- Test: `crates/lens-core/src/actor/scheduler.rs` (new tests)

**Interfaces:**
- Consumes: `spawn_actor_dual(…, feed, mode, …)` from Task 2; `OutputMode`; `ActorFeed`.
- Produces:
  - `FleetScheduler::wake(&mut self, conn, session_id, events, feed, initial_mode: OutputMode, stores, clock, api) -> Result<(), FleetSchedulerError>`
  - `FleetScheduler::reconnect(&mut self, conn, session_id, events, feed, initial_mode: OutputMode, stores, clock, api) -> Result<Option<SessionStatus>, FleetSchedulerError>`
  - Both call `spawn_actor_dual(state, events, feed, initial_mode, stores, clock, api)` — **not** `spawn_actor`.
  - Callers choose mode: background → `OutputMode::Summary`; lens-drive / focused → `OutputMode::Detailed`.

- [ ] **Step 1: Write the failing spawn-in-Summary test**

Scheduler tests already have `MockApi::with_scripts` / `empty_item_list` / `seed_active_session` — use those. Add a local status-event helper (copy the shape from `runloop.rs:1628-1634`); there is **no** `scheduler.command` — stop via `take_handle` + `stop_and_join`.

```rust
fn status_running_event() -> ServerStreamEvent {
    use lens_client::stream::{ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus};
    ServerStreamEvent::Session(SessionEvent::Status {
        status: WireStatus::Running,
        response_id: None,
        background_task_count: None,
    })
}

#[test]
fn wake_in_summary_emits_summary_not_summary_consumer_gone() {
    let dir = tempfile::tempdir().unwrap();
    let stores = test_stores(dir.path());
    seed_connection(&stores);
    seed_active_session(&stores);

    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let conn = ConnectionId::new("conn_1");
    let sid = SessionId::new("conv_1");
    let mut scheduler = FleetScheduler::new();

    let (api, _mock) = MockApi::with_scripts(
        VecDeque::new(),
        VecDeque::from([Ok(empty_item_list())]),
    );

    scheduler
        .wake(
            &conn,
            &sid,
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            api,
        )
        .expect("wake in Summary");

    // Pre-Task-4: no seed yet — drive a live status event so Summary mode emits.
    // Post-Task-4: the first frame may be the seed; either is ActorFeed::Summary
    // and must NOT be accompanied by SummaryConsumerGone.
    ev_tx.send(status_running_event()).unwrap();

    let frame = feed_rx.recv_blocking().expect("summary frame");
    assert!(
        matches!(frame, ActorFeed::Summary(_)),
        "spawn-in-Summary must emit Summary, got {frame:?}"
    );

    let handle = scheduler.handle(&sid).expect("running");
    assert!(
        !matches!(
            handle.outcomes.try_recv(),
            Ok(crate::actor::ActorOutcome::SummaryConsumerGone)
        ),
        "live Summary consumer must not observe SummaryConsumerGone"
    );

    scheduler
        .take_handle(&sid)
        .expect("handle")
        .stop_and_join();
}
```

Also update **every existing** `scheduler.wake(`/`reconnect(` call to pass `OutputMode::Detailed` (lens-drive + scheduler tests).

- [ ] **Step 2: Run test — expect FAIL**

```bash
cargo test -p lens-core --lib actor::scheduler::tests::wake_in_summary_emits_summary_not_summary_consumer_gone -- --nocapture
```
Expected: FAIL compile — `wake` has no `initial_mode` parameter / wrong arity.

- [ ] **Step 3: Implement dual-mode scheduler plumbing**

```rust
use crate::actor::runloop::{
    ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor_dual,
};

pub fn wake(
    &mut self,
    conn: &ConnectionId,
    session_id: &SessionId,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    initial_mode: OutputMode,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> Result<(), FleetSchedulerError> {
    if self.registry.contains_key(session_id) {
        return Err(FleetSchedulerError::AlreadyRunning);
    }
    let mut state =
        crate::persist::ControlStore::load_session(stores.control.as_ref(), conn, session_id)
            .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?
            .ok_or(FleetSchedulerError::SessionNotFound)?;
    state.lifecycle = SessionLifecycle::Active;
    let now = clock.now_millis();
    stores
        .control
        .upsert_session(&state, now)
        .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?;
    let handle = spawn_actor_dual(state, events, feed, initial_mode, stores, clock, api);
    self.registry.insert(session_id.clone(), handle);
    Ok(())
}

pub fn reconnect(
    &mut self,
    conn: &ConnectionId,
    session_id: &SessionId,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    initial_mode: OutputMode,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> Result<Option<SessionStatus>, FleetSchedulerError> {
    // … existing exited-handle reap + fetch_status + load_session unchanged …
    let handle = spawn_actor_dual(state, events, feed, initial_mode, stores, clock, api);
    self.parked.remove(session_id);
    self.registry.insert(session_id.clone(), handle);
    Ok(live_status)
}
```

**Do not** wake-in-Detailed-then-Demote. Catch-up runs before the command select (`runloop.rs:1019-1060`) and commands are deferred during catch-up — Detailed frames would escape before Demote, and the old `sum_tx` drop path is gone.

Update `lens-drive` call sites:

```rust
scheduler.reconnect(
    &conn.id,
    session_id,
    events,
    feed_tx,
    lens_core::actor::OutputMode::Detailed,
    stores,
    clock,
    api,
)
```

(`OutputMode` must be re-exported from `actor/mod.rs` — already is via `pub use runloop::{… OutputMode …}`.)

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p lens-core --lib actor::scheduler -- --nocapture
cargo check -p lens-drive
```
Expected: PASS / clean check.

- [ ] **Step 5: Commit**

```bash
git add \
  crates/lens-core/src/actor/scheduler.rs \
  crates/lens-drive/src/main.rs
git commit -m "$(cat <<'EOF'
feat(lens-core): scheduler dual-mode spawn via ActorFeed (§3.2)

EOF
)"
```

---

### Task 4: Seed-on-spawn + emit-on-Demote (§3.3)

**Files:**
- Modify: `crates/lens-core/src/actor/runloop.rs` (`run` after catch-up; `handle_command` Demote arm)
- Test: `crates/lens-core/src/actor/runloop.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `ActorFeed`, `SummaryUpdate::from_state`, `OutputMode`.
- Produces:
  - After successful startup catch-up in `run()`, if `output.mode == OutputMode::Summary`, emit `ActorFeed::Summary(SummaryUpdate::from_state(state))` before entering the select loop.
  - `Demote` flips `mode = Summary` then immediately emits the same Summary seed (symmetric with `Promote`'s `Rebased`).
  - On Demote emit failure: `LoopControl::Break` (feed consumer gone — same as Promote failure). On seed-on-spawn failure: push `ActorOutcome::SummaryConsumerGone` and still enter the loop (or return — prefer push + continue into loop so `Stop` still works; document the choice in the commit body). **Pin this plan's choice:** seed failure → push `SummaryConsumerGone`, do **not** abort the actor (mirrors Summary batch emit). Demote failure → `Break` (mirrors Promote).

- [ ] **Step 1: Write failing seed + Demote tests**

```rust
#[test]
fn summary_spawn_seeds_after_catchup() {
    let _dir = tempfile::tempdir().unwrap();
    let stores = test_stores(_dir.path());
    seed_connection(&stores);

    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let handle = spawn_actor_dual(
        fresh_state(),
        ev_rx,
        feed_tx,
        OutputMode::Summary,
        stores,
        test_clock(),
        noop_api(),
    );

    // Empty catch-up → no TranscriptAdvanced; seed must still arrive.
    match feed_rx.recv_blocking().expect("seed") {
        ActorFeed::Summary(u) => {
            assert_eq!(u.last_completed_turn, 0);
        }
        other => panic!("expected Summary seed, got {other:?}"),
    }
    handle.stop_and_join();
}

#[test]
fn demote_emits_summary_from_state() {
    let _dir = tempfile::tempdir().unwrap();
    let stores = test_stores(_dir.path());
    seed_connection(&stores);

    let mut state = fresh_state();
    state.title = Some("focused".into());
    state.stream.turn = 3;

    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let handle = spawn_actor(
        state,
        ev_rx,
        feed_tx,
        stores,
        test_clock(),
        noop_api(),
    );

    handle.commands.send(SessionCommand::Demote).unwrap();
    match feed_rx.recv_blocking().expect("demote summary") {
        ActorFeed::Summary(u) => {
            assert_eq!(u.title.as_deref(), Some("focused"));
            assert_eq!(u.last_completed_turn, 3);
        }
        other => panic!("expected Summary on Demote, got {other:?}"),
    }
    handle.stop_and_join();
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cargo test -p lens-core --lib actor::runloop::tests::summary_spawn_seeds_after_catchup -- --nocapture
cargo test -p lens-core --lib actor::runloop::tests::demote_emits_summary_from_state -- --nocapture
```
Expected: FAIL — timeout / no message on `recv_blocking` (no seed / Demote emits nothing).

- [ ] **Step 3: Implement seed + Demote emit**

In `handle_command`:

```rust
SessionCommand::Demote => {
    output.mode = OutputMode::Summary;
    if output
        .feed
        .send_blocking(ActorFeed::Summary(SummaryUpdate::from_state(state)))
        .is_err()
    {
        return LoopControl::Break;
    }
    LoopControl::Continue
}
```

In `run()`, immediately after successful catch-up (after the `if invoke_catchup_and_replay(…) == Break { return; }` block), before `loop { Select… }`:

```rust
if output.mode == OutputMode::Summary
    && output
        .feed
        .send_blocking(ActorFeed::Summary(SummaryUpdate::from_state(&state)))
        .is_err()
{
    ring.push(ActorOutcome::SummaryConsumerGone);
    drain_outcome_ring(&mut ring, &output.outcomes);
}
```

**Borrow note:** after `invoke_catchup_and_replay`, `state` and `output` are behind `ctx`. Either (a) emit using `ctx.output` / `ctx.state` before the select loop, or (b) emit before constructing `ctx` by restructuring so catch-up returns and then seed runs on the outer `state`/`output`. Prefer emitting via `ctx` after catch-up:

```rust
if invoke_catchup_and_replay(&mut ctx, false, false) == LoopControl::Break {
    return;
}
if ctx.output.mode == OutputMode::Summary
    && ctx
        .output
        .feed
        .send_blocking(ActorFeed::Summary(SummaryUpdate::from_state(ctx.state)))
        .is_err()
{
    ctx.ring.push(ActorOutcome::SummaryConsumerGone);
    drain_outcome_ring(ctx.ring, &ctx.output.outcomes);
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p lens-core --lib actor::runloop::tests::summary_spawn_seeds_after_catchup -- --nocapture
cargo test -p lens-core --lib actor::runloop::tests::demote_emits_summary_from_state -- --nocapture
```
Expected: PASS.

Update Task 3's `wake_in_summary_…` test if it now receives the seed **before** the status event Summary — drain/ignore the seed, then assert the live Summary (or assert the seed alone and drop the status send).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/actor/runloop.rs
git commit -m "$(cat <<'EOF'
feat(lens-core): seed Summary on spawn and emit on Demote (§3.3)

EOF
)"
```

---

### Task 5: Gate keystone tests (§3 preamble / §7 / §9)

**Files:**
- Test: `crates/lens-core/src/actor/runloop.rs` (primary)
- Test: `crates/lens-core/src/actor/scheduler.rs` (reconnect path if not already covered)
- Test: `crates/lens-core/src/actor/summary.rs` (enrichment already in Task 1 — re-run as gate checklist)

**Interfaces:**
- Consumes: all Task 1–4 surfaces (`ActorFeed`, seed, Demote emit, `spawn_actor_dual` Summary, enriched `SummaryUpdate`).
- Produces: the merge-gate evidence suite listed below — all must be green before any lens-ui work.

- [ ] **Step 1: Write the interleave keystone test (catch-up Detailed then seed Summary)**

```rust
#[test]
fn summary_mode_nonempty_catchup_then_seed_preserves_fifo_order() {
    let dir = tempfile::tempdir().unwrap();
    let stores = test_stores(dir.path());
    seed_connection(&stores);
    // Frontier on disk so catch-up fetches a nonempty page and emits TranscriptAdvanced.
    seed_message_item(&*stores.transcript, 0, "item_0", "item_0");
    assert_eq!(
        stores.transcript.store_frontier().unwrap(),
        Some((0, ItemId::new("item_0")))
    );

    let page = item_list_from_messages(&["item_1"], false);
    let (api, _mock) = MockApi::with_fetch_script(std::collections::VecDeque::from([Ok(page)]));

    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let handle = spawn_actor_dual(
        fresh_state(),
        ev_rx,
        feed_tx,
        OutputMode::Summary,
        stores,
        test_clock(),
        api,
    );

    let first = feed_rx.recv_blocking().expect("catch-up frame");
    let second = feed_rx.recv_blocking().expect("seed frame");
    assert!(
        matches!(
            first,
            ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })
        ),
        "first must be catch-up Detailed TranscriptAdvanced, got {first:?}"
    );
    assert!(
        matches!(second, ActorFeed::Summary(_)),
        "second must be §3.3 Summary seed, got {second:?}"
    );

    handle.stop_and_join();
}
```

- [ ] **Step 2: Run interleave test — expect FAIL only if seed/catch-up wiring regresses; otherwise it should already PASS after Task 4**

```bash
cargo test -p lens-core --lib actor::runloop::tests::summary_mode_nonempty_catchup_then_seed_preserves_fifo_order -- --nocapture
```
If FAIL with wrong order / missing Detailed: fix catch-up emit to use `ActorFeed::Detailed` (Task 2) or seed placement (Task 4). Do **not** weaken the assertion.

- [ ] **Step 3: Write remaining gate tests**

**3a — emit-on-Demote** (already in Task 4; keep as gate checklist item).

**3b — spawn-in-Summary emits Summary, not `SummaryConsumerGone`** (Task 3; after seed, assert first frame is `ActorFeed::Summary` and outcomes empty).

**3c — reconnect / deferred-transcript-commit on the unified channel**

Update the existing test `reconnected_greedy_drain_defers_live_commit_until_after_catchup` (`runloop.rs:4245`) as part of Task 2 churn, then strengthen its feed assertion in this task:

```rust
#[test]
fn reconnected_greedy_drain_defers_live_commit_until_after_catchup() {
    // … same fixture as today (frontier item_0, history page item_1/item_2,
    // Reconnected + live item_3) — but channels are ActorFeed:
    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

    ev_tx
        .send(ServerStreamEvent::Reconnected { gap: None })
        .unwrap();
    ev_tx
        .send(parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"item_3","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
        ))
        .unwrap();

    let mut saw_deferred_commit = false;
    while let Ok(frame) = feed_rx.recv_blocking() {
        if let ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: 3,
        }) = frame
        {
            saw_deferred_commit = true;
            break;
        }
    }
    assert!(
        saw_deferred_commit,
        "finish_deferred_transcript_commit must emit ActorFeed::Detailed(TranscriptAdvanced) on the unified feed"
    );

    handle.stop_and_join();
    // … same disk id order assertion: item_0..item_3 …
}
```

Also re-run `nested_buffered_reconnected_defers_live_until_nested_catchup` after the Task 2 unwrap churn — it covers the nested deferred path.
**3d — `SummaryUpdate` enrichment incl. `last_completed_turn`** — already Task 1; re-run:
```bash
cargo test -p lens-core --lib actor::summary::tests::from_state_copies_card_chrome_and_last_completed_turn
```

**3e — lagging-consumer order-safety**

```rust
#[test]
fn lagging_consumer_never_applies_stale_summary_after_promote() {
    let _dir = tempfile::tempdir().unwrap();
    let stores = test_stores(_dir.path());
    seed_connection(&stores);

    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    // Small capacity forces the producer to queue behind a slow consumer.
    let (feed_tx, feed_rx) = async_channel::bounded(2);
    let mut state = fresh_state();
    state.title = Some("bg".into());
    let handle = spawn_actor_dual(
        state,
        ev_rx,
        feed_tx,
        OutputMode::Summary,
        stores,
        test_clock(),
        noop_api(),
    );

    // Seed occupies one slot; push live Summary frames to fill the FIFO.
    let _seed = feed_rx.recv_blocking().unwrap(); // drain seed so producer can advance
    ev_tx.send(status_running_event()).unwrap();
    ev_tx.send(status_running_event()).unwrap();

    handle.commands.send(SessionCommand::Promote).unwrap();

    // After Promote, inject a Detailed-visible event (status again is fine once Detailed).
    ev_tx.send(status_running_event()).unwrap();

    // Drain everything the producer emitted, applying a tiny replica fold.
    #[derive(Debug, PartialEq)]
    enum Proj {
        SummaryTitle(Option<String>),
        Detailed,
    }
    let mut proj = Proj::SummaryTitle(Some("bg".into()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match feed_rx.try_recv() {
            Ok(ActorFeed::Summary(u)) => {
                // A Summary after we have entered Detailed projection would be a
                // regression only if it arrives *without* a later Demote. We never
                // Demote in this test — so Summary after Detailed is forbidden.
                assert_ne!(
                    proj,
                    Proj::Detailed,
                    "stale Summary must not land after Promote/Detailed"
                );
                proj = Proj::SummaryTitle(u.title);
            }
            Ok(ActorFeed::Detailed(StreamUpdate::Rebased(_))) => {
                proj = Proj::Detailed;
            }
            Ok(ActorFeed::Detailed(_)) => {
                assert_eq!(proj, Proj::Detailed, "Detailed deltas only after Rebased");
            }
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => break,
        }
        if proj == Proj::Detailed {
            // Keep draining briefly to catch a late Summary.
            std::thread::sleep(std::time::Duration::from_millis(20));
            while let Ok(frame) = feed_rx.try_recv() {
                assert!(
                    !matches!(frame, ActorFeed::Summary(_)),
                    "lagging Summary must not overtake Promote on the unified FIFO"
                );
            }
            break;
        }
    }
    assert_eq!(proj, Proj::Detailed);
    handle.stop_and_join();
}
```

- [ ] **Step 4: Run the full gate suite — expect PASS**

```bash
cargo test -p lens-core --lib \
  actor::runloop::tests::summary_mode_nonempty_catchup_then_seed_preserves_fifo_order \
  actor::runloop::tests::demote_emits_summary_from_state \
  actor::runloop::tests::summary_spawn_seeds_after_catchup \
  actor::runloop::tests::lagging_consumer_never_applies_stale_summary_after_promote \
  actor::summary::tests::from_state_copies_card_chrome_and_last_completed_turn \
  actor::scheduler::tests::wake_in_summary_emits_summary_not_summary_consumer_gone \
  -- --nocapture
cargo test -p lens-core --lib -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/actor/runloop.rs crates/lens-core/src/actor/scheduler.rs
git commit -m "$(cat <<'EOF'
test(lens-core): ActorFeed gate keystone coverage (§3.1–§3.4)

EOF
)"
```

---

### Task 6: lens-drive Detailed path + final workspace gate

**Files:**
- Modify: `crates/lens-drive/src/main.rs` (confirm Task 2/3 call sites pass `OutputMode::Detailed`; drain ignores Summary)
- No `generated.rs` touches

**Interfaces:**
- Consumes: `FleetScheduler::reconnect(…, feed, OutputMode::Detailed, …)`, `ActorFeed`.
- Produces: headless Detailed-only driver still prints `Rebased` / `TranscriptAdvanced`; workspace clippy+fmt green.

- [ ] **Step 1: Confirm lens-drive wiring (read-only audit, then fix gaps)**

Verify `main.rs` contains:

```rust
let (feed_tx, feed_rx) = async_channel::bounded(64);
// …
scheduler.reconnect(
    &conn.id,
    session_id,
    events,
    feed_tx,
    lens_core::actor::OutputMode::Detailed,
    stores,
    clock,
    api,
)?;
// drain_updates matches ActorFeed::Detailed { Rebased | TranscriptAdvanced }
```

If any `Sender<StreamUpdate>` or missing `initial_mode` remains, fix it now.

- [ ] **Step 2: Run final gate commands — expect PASS / clean**

```bash
cargo test -p lens-core --lib
cargo check -p lens-drive
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green. `generated.rs` git status clean (`git status -- crates/lens-client/src/generated.rs` shows no changes).

- [ ] **Step 3: Commit only if Step 1 produced diffs**

```bash
git add crates/lens-drive/src/main.rs
git commit -m "$(cat <<'EOF'
chore(lens-drive): consume unified ActorFeed in Detailed mode

EOF
)"
```

If no diffs, skip the commit.

- [ ] **Step 4: Gate evidence checklist (paste into PR body)**

- [ ] Summary-mode nonempty catch-up + seed: `Detailed(TranscriptAdvanced)` then `Summary(seed)` FIFO order
- [ ] reconnect / deferred-transcript-commit on unified feed
- [ ] emit-on-Demote → `ActorFeed::Summary`
- [ ] spawn-in-Summary → `Summary` (not `SummaryConsumerGone`)
- [ ] `SummaryUpdate` enrichment incl. `last_completed_turn == state.stream.turn`
- [ ] lagging-consumer order-safety (no stale Summary after Promote)
- [ ] `lens-drive` Detailed path compiles / checks
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] `generated.rs` untouched
- [ ] Cross-family + Opus review requested on the PR

---

## Plan self-review

**1. Spec coverage (§3.1–§3.4 + gate evidence)**

| Requirement | Task |
| --- | --- |
| §3.1 unified `ActorFeed` FIFO | Task 2 |
| §3.1 remove `sum_tx` drop hack | Task 2 |
| §3.1 `bounded(64)` caller construction | Task 2 / 6 (`lens-drive`) |
| §3.2 `wake`/`reconnect` take `feed` + `initial_mode`, `spawn_actor_dual` | Task 3 |
| §3.2 spawn-in-Summary (no wake-then-Demote) | Task 3 |
| §3.3 seed-on-spawn | Task 4 |
| §3.3 emit-on-Demote | Task 4 |
| §3.4 enrich `SummaryUpdate` + `last_completed_turn` | Task 1 |
| §3.4 `SessionState.harness` fold from snapshot | Task 1 (`fold_snapshot`) |
| Gate: interleave catch-up then seed | Task 5 |
| Gate: reconnect / deferred commit | Task 5 |
| Gate: Demote / spawn-in-Summary / enrichment / lagging order | Tasks 3–5 |
| Gate: lens-drive + clippy/fmt | Task 6 |
| Explicitly out of scope: §3.5, lens-ui | — (no tasks) |

**2. Placeholder scan:** none of TBD / "similar to Task N" / "add error handling" remain without concrete code.

**3. Type consistency:** `ActorFeed::{Summary,Detailed}`, `feed: Sender<ActorFeed>`, `initial_mode: OutputMode`, `last_completed_turn: u32`, `harness: Option<String>`, `activity_summary: String` — spelled identically across tasks.

**Deviations from the suggested decomposition:**
- Task 2 includes the minimum scheduler + lens-drive signature churn required to compile after the feed merge; Task 3 then adds `initial_mode` (suggested split kept, but Task 2 cannot leave `Sender<StreamUpdate>` in place).
- Harness is **not** persisted in the control store in this gate (design only mandates snapshot fold); documented above.
- `spawn_actor_dual` keeps its name despite no longer taking two senders (less churn than rename; design blast-radius text still names it).

---

## Reviewer verification (Opus, cross-family review of grok-4.5 author — 2026-07-15)

grok-4.5-xhigh authored this plan; reviewed by the Claude/Opus main loop (diverse
family per the review-diversity rule). Every load-bearing claim was **source-verified**
against `runloop.rs`, `scheduler.rs`, `summary.rs`, `session.rs`, `controls.rs`,
`usage.rs`, `item.rs`, `ids.rs`, and the persist layer. **Verdict: PASS — ready to
execute.** Verified: all 6 emit-site reconstructions match real source exactly
(incl. `coalesce`/`had_snapshot`/`scalars_baseline`/`commit_terminal_prefix`/
`drain_outcome_ring`); seed insertion point (line 1019-1024, before the `Select`
loop) + `ctx`-borrow handling correct; every test helper exists in the module the
test lives in; all domain field names correct; harness RAM-only-no-ALTER is the
right call (persist is column-mapped, snapshot re-folds every bootstrap).

**Minor items for the executor (none block the gate):**
1. **Task 5 test 3e (`lagging_consumer_…`) is near-tautological + its "fill the FIFO
   with 2 events" premise is imprecise.** In Summary mode the run-loop greedy-drains
   events into ONE batch and emits ONE coalesced `SummaryUpdate` per batch
   (`process_main_loop_event` → `apply_reduced_batch` Summary arm), so two
   `status_running_event()`s may collapse to a single Summary frame — the FIFO isn't
   reliably filled to 2. The **assertion still holds by construction** (single FIFO ⇒
   send-order == recv-order ⇒ no stale Summary after Promote), which is exactly the
   §3.1 invariant the gate wants documented — but the test is guarding a structural
   property, not exercising a race. Keep it, but treat a green result as
   *documentation of the invariant*, not proof of a fixed race; watch for mild
   flakiness from the 2s deadline + sleeps.
2. **`ItemKind::FunctionCall` full field list in Task 1's fallback test** was written
   from the variant name, not a verbatim field read. Copy the exact fields from
   `domain/item.rs:56` when implementing (the `activity_summary` impl uses
   `FunctionCall { name, .. }` and is safe regardless; only the test constructor names
   every field). Cheap compile-fix if a name differs.
3. **`activity_summary` "first in-flight tool" iterates a `HashMap`** (`unpaired_calls`)
   — order is nondeterministic for >1 concurrent tool. Fine for the gate (single-tool
   common case; the test uses exactly one). Note for a later deterministic tie-break.
4. **`SummaryConsumerGone` is a slight misnomer post-merge** (a closed *unified* feed
   isn't only a Summary consumer). Kept as-is to avoid churn; the pre-existing
   asymmetry is preserved — Detailed send-failure ⇒ `Break` (actor exits), Summary
   send-failure ⇒ push `SummaryConsumerGone` + continue. Worth a one-line code comment
   at the Summary arm, not a rename.

**Still required at execution (design mandate, separate from this plan review):** the
§3 code diff is a one-way-door actor change → **cross-family + Opus review on the diff**
before merge; author via composer-2.5 per task; `lens-drive` green is necessary but
not sufficient (Task 5 is the real gate evidence).
