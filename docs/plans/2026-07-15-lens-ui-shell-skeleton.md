# lens-ui shell skeleton — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first rendering consumer of the state model — `lens-ui` + `lens-app` — proving D10 dual-mode Summary↔Detailed feed→card→board isolation, promote/demote, §3.5 Ready, and the slot API, with live-verify at N≥10 as a hard merge gate.

**Architecture:** `FleetStore` (gpui `Entity`) owns a `FleetScheduler` (or `FakeFleet` in tests), creates a **per-session** `async_channel::bounded(64)` `ActorFeed` + clones that session's `Receiver<ActorOutcome>`, and spawns one async-only poller per session that coalesces bursts into one `SessionCard` entity update. Cards observe only themselves, mount as fixed-W×H `.cached` tiles; board recomposes board↔focused with empty slot containers + `ContentTab`/`TabHandle` placeholder. `lens-app` bootstraps gpui + gpui-component and wires the real scheduler (lens-drive shape, generalized to N + Summary).

**Tech Stack:** Rust 2024 / edition workspace, gpui `0.2.2`, gpui-component `0.5.1`, async-channel, crossbeam-channel, lens-core (ActorFeed/FleetScheduler/…), lens-client; product crates `lints.workspace = true`.

## Global Constraints

Copied verbatim from `AGENTS.md` critical rules (every task implicitly includes these):

- **MANDATORY** Performance is the prime objective. Target 120fps (8.3ms/frame); 90fps (11.1ms) is the regression line. Every layer carries a benchmark.
- **MANDATORY** Never block the gpui foreground thread; all I/O off-thread.
- **MANDATORY** The UI never panics the process; errors are modeled values.
- **MANDATORY** Typed end-to-end — no stringly-typed event dispatch.
- **MANDATORY** Benchmark-or-it's-not-done on perf paths; logic cores ship tests.
- **MANDATORY** `clippy` clean + `rustfmt`; `unsafe` needs a `// SAFETY:` note. The clippy gate is **workspace-wide** — `cargo clippy --workspace --all-targets -- -D warnings` — because per-crate green can hide workspace-red (e.g. throwaway `spikes/`). It MUST be clean before any push / end-of-session; **and if it is red when you pick up a task, resolve that first, before starting execution** — never build on a red gate.

## Preamble — scope, seams, locked wrinkles

**Scope:** design `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md` **§4–§7 + §3.5 Ready policy ONLY**. The lens-core §3.1–§3.4 ActorFeed gate is **already merged** — do **not** re-plan or re-implement it.

**Review seams (orchestrator):**
- Cross-family seam review on **Task 2** (SessionCard / §4.4) and **Task 4** (chrome + wave / §3.5 Ready).
- One Opus whole-branch final pass after Task 7.
- Scaffold / lens-app tasks (1, 3, 6): **no** per-task review.

**Stale-name correction:** design §4.5 says `SummaryConsumerGone` — the real variant is `ActorOutcome::FeedConsumerGone` (`outcome.rs:29`). Use the real name everywhere.

**Locked outcomes-receiver mechanism (ONE choice):** After `wake`/`reconnect` (or FakeFleet spawn), clone the public field:

```rust
let outcomes_rx = scheduler
    .handle(&session_id)
    .expect("just registered")
    .outcomes
    .clone(); // async_channel::Receiver is Clone; ActorHandle is NOT
```

Hand `outcomes_rx` to the per-session poller. The handle stays in the scheduler for `commands`. The poller is the **sole** consumer of that session's outcomes — never `recv`/`try_recv` via `handle.outcomes` after the clone is taken. No `FleetScheduler` accessor needed (`ActorHandle.outcomes` is already `pub` at `runloop.rs:60`).

**Per-session feed (not a shared bus):** channels carry no session id (spec §2). At spawn, `FleetStore` builds a fresh `(feed_tx, feed_rx) = async_channel::bounded(64)` per session; the poller owns that session's `feed_rx` and knows which `Entity<SessionCard>` to patch. lens-drive uses one channel only because N=1.

**Poller is async-only:** `cx.spawn` task `await`s `recv()` (or `futures::future::select` over feed+outcomes); never a fg sync-wait — else actor `send_blocking` can deadlock the fg executor. Coalesce: after first recv, `try_recv` until empty → one entity update (lens-store `lib.rs:85-105` precedent).

**Frame-driver for §6.1:** `card.update(cx, |_, cx| cx.notify())` only. **Never** `cx.refresh()` / `refresh_windows()` — sets `window.refreshing` → gpui ignores `.cached()` → false isolation failure.

**Terminal seam (skeleton):** publish only `ContentTab` + `TabHandle { view: AnyView, title: SharedString }` (title updatable) + placeholder tab. Do **not** build `lens-terminal` or call `lens-terminal::open`. `session.superseded` is **BLOCKED** (reducer drops target) — out of skeleton.

**Navigation:** `⌘.` = app-level `Action` with routing priority over any terminal key handler (§6.1 asserts 0 PTY bytes). **Not ESC.** `⌘\` / `⌘D` **DEFERRED**.

**Live-verify:** Task 7 is a **HARD in-plan MERGE GATE** — merge blocked until N≥10 background-Summary sessions + promote/demote runs green vs omnigent 0.5.1.

**Verified ground-truth APIs (do not invent):**

| Type | Location | Notes |
| --- | --- | --- |
| `enum ActorFeed { Summary(Box<SummaryUpdate>), Detailed(StreamUpdate) }` | `actor/feed.rs:8-14` | Summary is **boxed** |
| `struct SummaryUpdate { status, title, last_total_tokens, host_id, needs_attention, subagent_active, llm_model, model_override, agent_name, cumulative_cost, context_window, sandbox_status, git_branch, workspace, reasoning_effort, activity_summary, last_completed_turn, harness }` | `actor/summary.rs:13-32` | |
| `enum StreamUpdate { … TodosChanged, ScratchChanged, Rebased, StatusChanged, … }` | `reduce/update.rs:18-69` | Focused fold **must** consume Todos/Scratch |
| `enum SessionCommand { Stop, Promote, Demote, Send{text,model_override}, Sleep }` | `runloop.rs:32-43` | **No Interrupt** |
| `enum ActorOutcome { … FeedConsumerGone, … }` | `outcome.rs:11-35` | |
| `FleetScheduler::{new,wake,reconnect,handle,take_handle,sleep,mark_parked}` | `scheduler.rs:19-157` | Registry keyed by `SessionId` **only** → skeleton = **one connection** |
| `ActorHandle { commands, outcomes, join }` | `runloop.rs:58-62` | Not Clone; `outcomes` Receiver is Clone |
| Wiring precedent | `crates/lens-drive/src/main.rs` | Single-session Detailed-only |

**Wiring precedent to generalize:** `lens-drive` builds `Client`/`Connection`, `open_stores`, `ClientSessionApi::new(client)`, `Box<dyn Clock+Send>`, `async_channel::bounded(64)` feed, `client.sessions().stream(id)` → crossbeam forwarder, `scheduler.reconnect(..., feed_tx, OutputMode::Detailed, ...)`, drains feed/outcomes, `handle.commands.send(SessionCommand::Send{..})`. Skeleton: N sessions, initial `OutputMode::Summary`, promote→Detailed / demote→Summary, **render**.

---

## Opus verification corrections (2026-07-15) — READ FIRST, apply while executing

This plan was authored by grok-4.5 and Opus-source-verified. **The lens-core/domain
type grounding is CONFIRMED CORRECT** — `Cost.total_cost_usd: Option<f64>`
(`usage.rs:40`), `SessionStatusValue{Idle,Launching,Running,Waiting,Failed,Unknown}`
(`scalars.rs:54`), `Todo{content,status,active_form}` (`controls.rs:19`),
`SessionState.{last_task_error,lifecycle,context_window,todos,pending_elicitations}`
(`session.rs`), the 27-variant `StreamUpdate` match is exhaustive, `ActorHandle.outcomes`
is `pub` and `Clone`. Trust those. **But verification against gpui-0.2.2 source found
FOUR gpui-structural defects the executor MUST fix — the domain code is right; the
gpui plumbing has bugs that green unit tests would hide.**

**C1 (CRITICAL — the poller never runs as written).** gpui `Task` **cancels on drop**
(confirmed: the spikes call `.detach()`, `spikes/transcript-virtual/src/app.rs:645`).
Task 1's `spawn_fake_session` and Task 6's `spawn_live_session` do
`let _task = spawn_session_poller(...)` — the `Task<()>` drops at end of scope →
**the poller is cancelled immediately**; folds never reach the card. FIX: `FleetStore`
owns `pollers: HashMap<SessionId, gpui::Task<()>>`; store the returned Task there
(kept alive with the session, cancelled correctly on removal). Do **NOT** `.detach()`
(that leaks the task past session teardown). Without this, Task 1's coalesce test
either hangs or false-fails — it is load-bearing for the whole crate.

**C2 (CRITICAL — the §6.1 acceptance test is a vacuous false-green as written).**
Task 5's test **never creates a window**, so the view tree is never rendered:
`SessionCardView::render` is never called, `render_count` stays `0`, and *every*
isolation assertion ("B renders, A doesn't", "C doesn't paint") passes trivially on
**broken** code. `TestAppContext::{add_window, add_window_view, draw}` are the real
API (`test_context.rs:215/256/814`). FIX: mount the board via
`cx.add_window_view(|window, cx| BoardView::new(fleet, window, cx)) -> WindowHandle<BoardView>`,
drive frames with `window.update(cx, |_, _, cx| cx.notify())` + `cx.run_until_parked()`
(the effect-flush auto-draws dirty windows at `refreshing=false` — spec §6.1 impl
caveat; still **never** `cx.refresh()`). Consequently **`BoardView` must be an
`Entity`/mounted `Render` view**, not the bare struct grok's test treats it as
(`let board = BoardView::new(...)` calling `&self` helpers is wrong) — its test-helpers
(`card_views_for_test`, `card_bounds_for_test`, `set_pty_probe_for_test`,
`focus_working_tab_for_test`) become `board.read(cx)` / `board.update(cx, …)` on the
`WindowHandle<BoardView>`.

**C3 (CRITICAL false-green risk — the size-invariance sub-assertion).** `.cached(style:
StyleRefinement)` is confirmed real (`view.rs:103`), so the mount signature is fine.
BUT `card_bounds_for_test(&c, cx)` (§6.1 pt 3) must return C's **actual computed layout
bounds after a real draw** — a helper that returns a constant makes the "C bounds
byte-equal" assertion pass vacuously, which is exactly the false-green the spec §6.1
warns against ("a single fixed-geometry injection can't prove bounds-stability"). Capture
genuine post-`draw` bounds; if that proves infeasible headless, the Task-5 reviewer
**flags it — does not weaken the assert**. Confirm the `.cached` receiver at compile:
it is on `AnyView` (`view.rs:103`), so the mount is `view.into_any().cached(style)` —
verify vs grok's `.into_any_element().cached(style)`.

**C4 (minor — compile friction, not logic).** Confirm gpui call forms at compile rather
than trusting the reconstruction: `cx.spawn` takes a closure receiving `AsyncApp`
**by value** (`context.rs:237`, `test_context.rs:347` = `FnOnce(AsyncApp) -> Fut`) —
check the `async move |cx|` vs `|cx| async move {}` form the compiler accepts; the
poller is called with `&mut Context<Self>` where `&mut App` is wanted (needs an explicit
`&mut *cx` reborrow); drop the unused `use gpui::{Stylable, Div};`.

**Struct-field gap:** add `seeded: bool` and `ready_reschedule: bool` to `SessionCard`
in **Task 2's struct definition** — Task 4's code uses them but they are absent from the
field list (init both `false` in `SessionCard::new`).

**Calibration note (not a defect):** §6.1 pt 7's "0 PTY bytes" is a **weak proxy** in the
skeleton — there is no real terminal/PTY competing for the keystroke (placeholder tab
only), so the assert proves `BackToBoard` dispatches + Demotes, **not** true routing
priority over a PTY handler. Acceptable for the skeleton; the real priority test lands
with the terminal slice. State this in the test comment so it is not over-claimed.

The Task 2 (SessionCard/§4.4) and Task 4 (Ready) review seams should specifically
confirm C1–C3 are fixed, not just that tests are green.

---

## File Structure

| Path | Role |
| --- | --- |
| `crates/lens-ui/Cargo.toml` | lib crate; gpui + gpui-component + lens-core; `lints.workspace = true` |
| `crates/lens-ui/src/lib.rs` | Re-exports; module tree |
| `crates/lens-ui/src/clock.rs` | `UiClock` + `ManualUiClock` / `WallUiClock` (Ready decay inject) |
| `crates/lens-ui/src/fleet/mod.rs` | Fleet module |
| `crates/lens-ui/src/fleet/fake.rs` | `FakeFleet` test-support |
| `crates/lens-ui/src/fleet/store.rs` | `FleetStore` entity + promote/demote + membership |
| `crates/lens-ui/src/fleet/poller.rs` | Per-session async poller (feed+outcomes coalesce) |
| `crates/lens-ui/src/card/mod.rs` | Card module |
| `crates/lens-ui/src/card/model.rs` | `SessionCard` state + fold + Ready fields |
| `crates/lens-ui/src/card/wave.rs` | Wave ladder derivation |
| `crates/lens-ui/src/card/view.rs` | `SessionCardView` — observe own entity, fixed tile, `.cached` |
| `crates/lens-ui/src/board/mod.rs` | Board + focused recompose |
| `crates/lens-ui/src/slot/mod.rs` | `ContentTab`, `TabHandle`, placeholder |
| `crates/lens-ui/src/actions.rs` | `BackToBoard` (`⌘.`) |
| `crates/lens-app/Cargo.toml` | bin; depends lens-ui + lens-core + lens-client |
| `crates/lens-app/src/main.rs` | Application bootstrap, Root, live FleetScheduler |
| `crates/lens-drive/src/main.rs` | Optional extend for fleet-spawn harness (Task 7) **or** `lens-app --fleet-verify` |

Workspace `members = ["crates/*", …]` already picks up new crates — no root `Cargo.toml` members edit required.

---

### Task 1: Crate scaffold + FakeFleet + FleetStore poller

**Files:**
- Create: `crates/lens-ui/Cargo.toml`
- Create: `crates/lens-ui/src/lib.rs`
- Create: `crates/lens-ui/src/clock.rs`
- Create: `crates/lens-ui/src/fleet/mod.rs`
- Create: `crates/lens-ui/src/fleet/fake.rs`
- Create: `crates/lens-ui/src/fleet/store.rs`
- Create: `crates/lens-ui/src/fleet/poller.rs`
- Create: `crates/lens-ui/src/card/mod.rs`
- Create: `crates/lens-ui/src/card/model.rs` (minimal stub fields for poller to patch)
- Create: `crates/lens-app/Cargo.toml`
- Create: `crates/lens-app/src/main.rs` (minimal stub window — full bootstrap in Task 6)
- Test: `crates/lens-ui/src/fleet/poller.rs` (`#[cfg(test)]`) + `fake.rs` tests

**Interfaces:**
- Consumes: `lens_core::actor::{ActorFeed, ActorOutcome, SessionCommand, SummaryUpdate, StreamUpdate}`; `async_channel`; gpui `App`/`Entity`/`TestAppContext`.
- Produces:
  - `pub struct FakeSession { pub feed_tx: async_channel::Sender<ActorFeed>, pub commands_rx: crossbeam_channel::Receiver<SessionCommand>, pub outcomes_tx: async_channel::Sender<ActorOutcome> }`
  - `pub struct FakeFleet { /* sessions: HashMap<SessionId, FakeSession> */ }`
  - `impl FakeFleet { pub fn new() -> Self; pub fn spawn_session(&mut self, id: SessionId) -> FakeSessionHandles; pub fn feed_tx(&self, id: &SessionId) -> async_channel::Sender<ActorFeed>; pub fn push_feed(&self, id: &SessionId, frame: ActorFeed); pub fn take_commands(&self, id: &SessionId) -> Vec<SessionCommand>; }`
  - `pub struct FakeSessionHandles { pub feed_rx: async_channel::Receiver<ActorFeed>, pub outcomes_rx: async_channel::Receiver<ActorOutcome>, pub commands_tx: crossbeam_channel::Sender<SessionCommand> }`
  - `pub struct SessionCard { pub session_id: SessionId, pub status: SessionStatusValue, pub title: Option<String>, pub activity_summary: String, pub last_completed_turn: u32, pub seen_turn: u32, pub last_completed_at: Option<i64>, /* more fields Task 2 */ pub notify_count: u64 /* test counter */ }`
  - `impl SessionCard { pub fn fold_feed(&mut self, frame: ActorFeed, clock: &dyn UiClock); }` — Task 1: Summary copy-assigns status/title/activity/last_completed_turn; Detailed no-op stubs OK until Task 2
  - `pub struct FleetStore { cards: HashMap<SessionId, Entity<SessionCard>>, focused: Option<SessionId>, fake: Option<FakeFleet>, /* scheduler later */, clock: Arc<dyn UiClock>, store_notify_count: Cell<u64> /* test */ }`
  - `impl FleetStore { pub fn new(clock: Arc<dyn UiClock>, cx: &mut App) -> Entity<Self>; pub fn spawn_fake_session(&mut self, id: SessionId, cx: &mut App) -> Entity<SessionCard>; pub fn card(&self, id: &SessionId) -> Option<Entity<SessionCard>>; pub fn store_notify_count(&self) -> u64; }`
  - `pub fn spawn_session_poller(card: Entity<SessionCard>, feed_rx: async_channel::Receiver<ActorFeed>, outcomes_rx: async_channel::Receiver<ActorOutcome>, clock: Arc<dyn UiClock>, cx: &mut App) -> gpui::Task<()>`
  - `pub trait UiClock: Send + Sync { fn now_millis(&self) -> i64; }` + `ManualUiClock` / `WallUiClock`
  - Constants: `pub const FEED_CAPACITY: usize = 64;`

- [ ] **Step 1: Write failing FakeFleet + coalesce poller tests**

Create `crates/lens-ui/Cargo.toml`:

```toml
[package]
name = "lens-ui"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = "Lens UI shell — board, session cards, FleetStore"

[lints]
workspace = true

[dependencies]
async-channel = "2"
crossbeam-channel = "0.5"
futures = "0.3"
gpui = "0.2.2"
gpui-component = "0.5.1"
lens-core = { path = "../lens-core" }
smallvec = "1"

[dev-dependencies]
gpui = { version = "0.2.2", features = ["test-support"] }
```

Create stub modules so tests compile against intended APIs — then write the tests below in `fleet/fake.rs` and `fleet/poller.rs`. The first `cargo test` must **FAIL** (types/fns missing or asserts fail).

```rust
// crates/lens-ui/src/fleet/fake.rs — #[cfg(test)] or always-on test-support module
#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::actor::{ActorFeed, SummaryUpdate};
    use lens_core::domain::ids::SessionId;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::usage::Cost;

    fn empty_summary(status: SessionStatusValue, turn: u32) -> SummaryUpdate {
        SummaryUpdate {
            status,
            title: Some("t".into()),
            last_total_tokens: None,
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: None,
            model_override: None,
            agent_name: None,
            cumulative_cost: Cost::default(),
            context_window: None,
            sandbox_status: None,
            git_branch: None,
            workspace: None,
            reasoning_effort: None,
            activity_summary: String::new(),
            last_completed_turn: turn,
            harness: None,
        }
    }

    #[test]
    fn fake_fleet_per_session_channels_are_independent() {
        let mut fleet = FakeFleet::new();
        let a = SessionId::new("a");
        let b = SessionId::new("b");
        let _ha = fleet.spawn_session(a.clone());
        let _hb = fleet.spawn_session(b.clone());
        fleet.push_feed(
            &a,
            ActorFeed::Summary(Box::new(empty_summary(SessionStatusValue::Idle, 1))),
        );
        assert!(
            fleet.feed_tx(&b).is_empty() || fleet.try_recv_feed(&b).is_none(),
            "pushing on A must not deliver on B's channel"
        );
        let frame = fleet.try_recv_feed(&a).expect("A has frame");
        assert!(matches!(frame, ActorFeed::Summary(_)));
    }
}
```

```rust
// crates/lens-ui/src/fleet/poller.rs — #[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::model::SessionCard;
    use crate::clock::ManualUiClock;
    use crate::fleet::fake::FakeFleet;
    use crate::fleet::store::FleetStore;
    use lens_core::actor::{ActorFeed, SummaryUpdate};
    use lens_core::domain::ids::SessionId;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::usage::Cost;
    use std::sync::Arc;

    fn summary(status: SessionStatusValue, title: &str, activity: &str, turn: u32) -> SummaryUpdate {
        SummaryUpdate {
            status,
            title: Some(title.into()),
            last_total_tokens: None,
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: Some("opus".into()),
            model_override: None,
            agent_name: None,
            cumulative_cost: Cost::default(),
            context_window: Some(200_000),
            sandbox_status: None,
            git_branch: Some("main".into()),
            workspace: Some("lens".into()),
            reasoning_effort: None,
            activity_summary: activity.into(),
            last_completed_turn: turn,
            harness: Some("claude-native".into()),
        }
    }

    #[gpui::test]
    async fn poller_dispatches_feed_to_card_and_coalesces_burst(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let (fleet_entity, card) = cx.update(|cx| {
            let fleet = FleetStore::new(clock.clone(), cx);
            let card = fleet.update(cx, |f, cx| f.spawn_fake_session(sid.clone(), cx));
            (fleet, card)
        });

        // Burst of 50 Summary frames — last title wins after coalesce.
        cx.update(|cx| {
            let fleet = fleet_entity.read(cx);
            let fake = fleet.fake.as_ref().expect("fake mode");
            for i in 0..50u32 {
                fake.push_feed(
                    &sid,
                    ActorFeed::Summary(Box::new(summary(
                        SessionStatusValue::Running,
                        &format!("t{i}"),
                        "working",
                        i,
                    ))),
                );
            }
        });
        cx.run_until_parked();

        let (title, notifies, store_n) = cx.read(|cx| {
            let c = card.read(cx);
            let f = fleet_entity.read(cx);
            (c.title.clone(), c.notify_count, f.store_notify_count())
        });
        assert_eq!(title.as_deref(), Some("t49"));
        assert!(
            notifies < 50,
            "burst must coalesce: notify_count={notifies}"
        );
        assert_eq!(
            store_n, 1,
            "FleetStore notified only on membership spawn, not on scalar folds"
        );
        // store_n == 1 from spawn_fake_session membership notify; scalar folds must not increment it further.
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL**

Run:

```bash
cargo test -p lens-ui --lib fake_fleet_per_session_channels_are_independent -- --nocapture
cargo test -p lens-ui --lib poller_dispatches_feed_to_card_and_coalesces_burst -- --nocapture
```

Expected: FAIL — `lens-ui` crate / symbols not found, or compile errors on missing types.

- [ ] **Step 3: Minimal implementation**

`crates/lens-ui/src/clock.rs`:

```rust
use std::sync::atomic::{AtomicI64, Ordering};

pub trait UiClock: Send + Sync {
    fn now_millis(&self) -> i64;
}

pub struct WallUiClock;

impl UiClock for WallUiClock {
    fn now_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

pub struct ManualUiClock {
    now: AtomicI64,
}

impl ManualUiClock {
    pub fn new(now_millis: i64) -> Self {
        Self {
            now: AtomicI64::new(now_millis),
        }
    }
    pub fn set(&self, now_millis: i64) {
        self.now.store(now_millis, Ordering::SeqCst);
    }
}

impl UiClock for ManualUiClock {
    fn now_millis(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}
```

`crates/lens-ui/src/card/model.rs` (Task-1 minimal):

```rust
use crate::clock::UiClock;
use lens_core::actor::ActorFeed;
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::SessionStatusValue;

#[derive(Clone, Debug)]
pub struct SessionCard {
    pub session_id: SessionId,
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub activity_summary: String,
    pub last_completed_turn: u32,
    pub seen_turn: u32,
    pub last_completed_at: Option<i64>,
    pub connection_overlay: ConnectionOverlay,
    /// Test/instrumentation: increments on each `cx.notify` from poller folds.
    pub notify_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionOverlay {
    #[default]
    Connected,
    Reconnecting,
    Disconnected,
}

impl SessionCard {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            status: SessionStatusValue::Idle,
            title: None,
            activity_summary: String::new(),
            last_completed_turn: 0,
            seen_turn: 0,
            last_completed_at: None,
            connection_overlay: ConnectionOverlay::Connected,
            notify_count: 0,
        }
    }

    pub fn fold_feed(&mut self, frame: ActorFeed, _clock: &dyn UiClock) {
        match frame {
            ActorFeed::Summary(u) => {
                self.status = u.status;
                self.title = u.title.clone();
                self.activity_summary = u.activity_summary.clone();
                self.last_completed_turn = u.last_completed_turn;
                // Ready stamp lands in Task 4; Task 1 only copies scalars.
            }
            ActorFeed::Detailed(_) => {
                // Task 2 dual-mode fold.
            }
        }
    }
}
```

`crates/lens-ui/src/fleet/fake.rs`:

```rust
use crossbeam_channel::{Receiver, Sender};
use lens_core::actor::{ActorFeed, ActorOutcome, SessionCommand};
use lens_core::domain::ids::SessionId;
use std::collections::HashMap;

pub const FEED_CAPACITY: usize = 64;

pub struct FakeSessionHandles {
    pub feed_rx: async_channel::Receiver<ActorFeed>,
    pub outcomes_rx: async_channel::Receiver<ActorOutcome>,
    pub commands_tx: Sender<SessionCommand>,
}

struct FakeSession {
    feed_tx: async_channel::Sender<ActorFeed>,
    feed_rx: async_channel::Receiver<ActorFeed>,
    outcomes_tx: async_channel::Sender<ActorOutcome>,
    outcomes_rx: async_channel::Receiver<ActorOutcome>,
    commands_tx: Sender<SessionCommand>,
    commands_rx: Receiver<SessionCommand>,
}

pub struct FakeFleet {
    sessions: HashMap<SessionId, FakeSession>,
}

impl FakeFleet {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn spawn_session(&mut self, id: SessionId) -> FakeSessionHandles {
        let (feed_tx, feed_rx) = async_channel::bounded(FEED_CAPACITY);
        let (outcomes_tx, outcomes_rx) = async_channel::bounded(FEED_CAPACITY);
        let (commands_tx, commands_rx) = crossbeam_channel::bounded(FEED_CAPACITY);
        let handles = FakeSessionHandles {
            feed_rx: feed_rx.clone(),
            outcomes_rx: outcomes_rx.clone(),
            commands_tx: commands_tx.clone(),
        };
        self.sessions.insert(
            id,
            FakeSession {
                feed_tx,
                feed_rx,
                outcomes_tx,
                outcomes_rx,
                commands_tx,
                commands_rx,
            },
        );
        handles
    }

    pub fn feed_tx(&self, id: &SessionId) -> async_channel::Sender<ActorFeed> {
        self.sessions[id].feed_tx.clone()
    }

    pub fn push_feed(&self, id: &SessionId, frame: ActorFeed) {
        self.sessions[id]
            .feed_tx
            .try_send(frame)
            .expect("fake feed push");
    }

    pub fn try_recv_feed(&self, id: &SessionId) -> Option<ActorFeed> {
        self.sessions[id].feed_rx.try_recv().ok()
    }

    pub fn take_commands(&self, id: &SessionId) -> Vec<SessionCommand> {
        let rx = &self.sessions[id].commands_rx;
        let mut out = Vec::new();
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    pub fn push_outcome(&self, id: &SessionId, outcome: ActorOutcome) {
        let _ = self.sessions[id].outcomes_tx.try_send(outcome);
    }
}

impl Default for FakeFleet {
    fn default() -> Self {
        Self::new()
    }
}
```

`crates/lens-ui/src/fleet/poller.rs`:

```rust
use crate::card::model::{ConnectionOverlay, SessionCard};
use crate::clock::UiClock;
use futures::future::{Either, select};
use futures::pin_mut;
use gpui::{App, Entity, Task};
use lens_core::actor::{ActorFeed, ActorOutcome, ParkReason};
use std::sync::Arc;

pub fn spawn_session_poller(
    card: Entity<SessionCard>,
    feed_rx: async_channel::Receiver<ActorFeed>,
    outcomes_rx: async_channel::Receiver<ActorOutcome>,
    clock: Arc<dyn UiClock>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        loop {
            let feed_wait = feed_rx.recv();
            let out_wait = outcomes_rx.recv();
            pin_mut!(feed_wait);
            pin_mut!(out_wait);
            match select(feed_wait, out_wait).await {
                Either::Left((Ok(first), _)) => {
                    let mut batch = smallvec::SmallVec::<[ActorFeed; 8]>::new();
                    batch.push(first);
                    while let Ok(more) = feed_rx.try_recv() {
                        batch.push(more);
                    }
                    let clock = Arc::clone(&clock);
                    if card
                        .update(cx, |card, cx| {
                            for frame in batch.drain(..) {
                                card.fold_feed(frame, clock.as_ref());
                            }
                            card.notify_count = card.notify_count.saturating_add(1);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Either::Right((Ok(first), _)) => {
                    let mut batch = smallvec::SmallVec::<[ActorOutcome; 4]>::new();
                    batch.push(first);
                    while let Ok(more) = outcomes_rx.try_recv() {
                        batch.push(more);
                    }
                    if card
                        .update(cx, |card, cx| {
                            for o in batch.drain(..) {
                                apply_outcome(card, o);
                            }
                            card.notify_count = card.notify_count.saturating_add(1);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Either::Left((Err(_), _)) | Either::Right((Err(_), _)) => break,
            }
        }
    })
}

fn apply_outcome(card: &mut SessionCard, outcome: ActorOutcome) {
    match outcome {
        ActorOutcome::Parked { reason } => {
            let _ = reason; // ParkReason retained for overlay mapping
            card.connection_overlay = ConnectionOverlay::Disconnected;
        }
        ActorOutcome::TransportChanged { transport, .. } => {
            use lens_core::actor::ActorTransport;
            card.connection_overlay = match transport {
                ActorTransport::Connected => ConnectionOverlay::Connected,
                ActorTransport::Reconnecting => ConnectionOverlay::Reconnecting,
            };
        }
        ActorOutcome::FeedConsumerGone
        | ActorOutcome::PersistError { .. }
        | ActorOutcome::SendLost { .. }
        | ActorOutcome::Slept
        | ActorOutcome::SleepDeclined
        | ActorOutcome::Command(_) => {
            // Log-only in skeleton — no panic.
        }
    }
}
```

`crates/lens-ui/src/fleet/store.rs` (fake-mode spawn):

```rust
use crate::card::model::SessionCard;
use crate::clock::UiClock;
use crate::fleet::fake::FakeFleet;
use crate::fleet::poller::spawn_session_poller;
use gpui::{App, Context, Entity};
use lens_core::domain::ids::SessionId;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

pub struct FleetStore {
    pub cards: HashMap<SessionId, Entity<SessionCard>>,
    pub focused: Option<SessionId>,
    pub fake: Option<FakeFleet>,
    clock: Arc<dyn UiClock>,
    store_notify_count: Cell<u64>,
    // command senders for fake mode (promote/demote later)
    command_txs: HashMap<SessionId, crossbeam_channel::Sender<lens_core::actor::SessionCommand>>,
}

impl FleetStore {
    pub fn new(clock: Arc<dyn UiClock>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            cards: HashMap::new(),
            focused: None,
            fake: Some(FakeFleet::new()),
            clock,
            store_notify_count: Cell::new(0),
            command_txs: HashMap::new(),
        })
    }

    pub fn store_notify_count(&self) -> u64 {
        self.store_notify_count.get()
    }

    pub fn card(&self, id: &SessionId) -> Option<Entity<SessionCard>> {
        self.cards.get(id).cloned()
    }

    pub fn spawn_fake_session(
        &mut self,
        id: SessionId,
        cx: &mut Context<Self>,
    ) -> Entity<SessionCard> {
        let fake = self.fake.as_mut().expect("fake mode");
        let handles = fake.spawn_session(id.clone());
        self.command_txs.insert(id.clone(), handles.commands_tx);
        let card = cx.new(|_| SessionCard::new(id.clone()));
        let _task = spawn_session_poller(
            card.clone(),
            handles.feed_rx,
            handles.outcomes_rx,
            Arc::clone(&self.clock),
            cx,
        );
        self.cards.insert(id, card.clone());
        self.store_notify_count
            .set(self.store_notify_count.get().saturating_add(1));
        cx.notify(); // membership change ONLY
        card
    }
}
```

Wire `lib.rs` / `fleet/mod.rs` / `card/mod.rs`. Stub `crates/lens-app`:

```toml
# crates/lens-app/Cargo.toml
[package]
name = "lens-app"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = "Lens native macOS app"

[lints]
workspace = true

[[bin]]
name = "lens-app"
path = "src/main.rs"

[dependencies]
gpui = "0.2.2"
gpui-component = "0.5.1"
lens-ui = { path = "../lens-ui" }
```

```rust
// crates/lens-app/src/main.rs — stub until Task 6
fn main() {
    eprintln!("lens-app: bootstrap lands in Task 6");
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p lens-ui --lib
```

Expected: PASS for FakeFleet independence + poller coalesce; `store_notify_count == 1` after burst.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui crates/lens-app
git commit -m "$(cat <<'EOF'
feat(lens-ui): scaffold crate, FakeFleet, and coalescing session poller

EOF
)"
```

---

### Task 2: SessionCard dual-mode fold + §4.4 isolation invariant

> **Review seam:** cross-family review required after this task.

**Files:**
- Modify: `crates/lens-ui/src/card/model.rs`
- Create: `crates/lens-ui/src/card/view.rs`
- Modify: `crates/lens-ui/src/card/mod.rs`
- Test: `crates/lens-ui/src/card/model.rs` (`#[cfg(test)]`) + `view.rs` isolation audit test

**Interfaces:**
- Consumes: `ActorFeed`, `SummaryUpdate`, `StreamUpdate::{StatusChanged,UsageChanged,ModelChanged,TodosChanged,ScratchChanged,Rebased,LastTokensChanged,ContextWindowChanged,LastTaskErrorChanged,TitleChanged,AgentChanged,ReasoningEffortChanged,SandboxChanged,ElicitationsChanged,Disconnected,Reconnecting,Reconnected,…}`; `SessionState` on `Rebased`.
- Produces:
  - Extended `SessionCard` fields: `llm_model`, `model_override`, `agent_name`, `harness`, `cumulative_cost: Cost`, `context_window`, `last_total_tokens`, `sandbox_status`, `git_branch`, `workspace`, `reasoning_effort`, `needs_attention`, `subagent_active`, `last_task_error`, `lifecycle: SessionLifecycle`, `repos: Vec<RepoRef>`, `todos: Vec<Todo>`, `scratch_activity: String` (from ScratchChanged / todos for activity while focused)
  - `pub struct RepoRef { pub name: String, pub branch: Option<String> }`
  - `pub const CARD_WIDTH_PX: f32 = 280.0;`
  - `pub const CARD_HEIGHT_PX: f32 = 148.0;`
  - `impl SessionCard { pub fn fold_feed(&mut self, frame: ActorFeed, clock: &dyn UiClock); pub fn fold_summary(&mut self, u: &SummaryUpdate, clock: &dyn UiClock); pub fn fold_detailed(&mut self, u: StreamUpdate); pub fn set_repos_for_test(&mut self, repos: Vec<RepoRef>); }`
  - `pub struct SessionCardView { card: Entity<SessionCard>, render_count: Rc<Cell<usize>>, paint_count: Rc<Cell<usize>> }`
  - `impl SessionCardView { pub fn new(card: Entity<SessionCard>, cx: &mut Context<Self>) -> Self; }` + `Render` with **fixed** `w(px(CARD_WIDTH_PX)).h(px(CARD_HEIGHT_PX))`, observes **only** `self.card`
  - Board mount helper: `pub fn mount_cached_card(view: Entity<SessionCardView>) -> impl IntoElement` wrapping `AnyView` in `.cached(style)` with stable id

**§4.4 audit step (required):** any gpui-component widget used inside the card must be checked against cache-key/bounds rules (reuse keys on `cache_key.bounds == bounds`; `.cached()` ignored when `window.refreshing`). Prefer plain `div`/text for skeleton chrome; if a gpui-component widget is used, document why it does not break sibling cache (or keep it out of the card).

- [ ] **Step 1: Write failing dual-mode fold + isolation tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::actor::{ActorFeed, SummaryUpdate};
    use lens_core::domain::controls::{Todo, TodoStatus};
    use lens_core::domain::ids::{AgentId, ConnectionId, SessionId};
    use lens_core::domain::item::StreamScratch;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::session::SessionState;
    use lens_core::domain::usage::Cost;
    use lens_core::reduce::StreamUpdate;
    use std::sync::Arc;

    fn base_summary() -> SummaryUpdate {
        SummaryUpdate {
            status: SessionStatusValue::Idle,
            title: Some("hello".into()),
            last_total_tokens: Some(1000),
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: Some("opus".into()),
            model_override: None,
            agent_name: Some("coder".into()),
            cumulative_cost: Cost {
                total_cost_usd: Some(0.5),
                ..Cost::default()
            },
            context_window: Some(200_000),
            sandbox_status: None,
            git_branch: Some("main".into()),
            workspace: Some("lens".into()),
            reasoning_effort: Some("high".into()),
            activity_summary: String::new(),
            last_completed_turn: 3,
            harness: Some("claude-native".into()),
        }
    }

    #[test]
    fn summary_fold_copies_enriched_scalars_and_seeds_one_repo() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);
        assert_eq!(card.title.as_deref(), Some("hello"));
        assert_eq!(card.llm_model.as_deref(), Some("opus"));
        assert_eq!(card.harness.as_deref(), Some("claude-native"));
        assert_eq!(card.repos.len(), 1);
        assert_eq!(card.repos[0].name, "lens");
        assert_eq!(card.repos[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn detailed_fold_consumes_todos_and_scratch_for_activity() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);

        let mut baseline = SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("s"),
            AgentId::new("ag"),
        );
        baseline.title = Some("rebased".into());
        baseline.llm_model = Some("sonnet".into());
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::Rebased(Box::new(baseline))),
            &clock,
        );
        assert_eq!(card.title.as_deref(), Some("rebased"));
        assert_eq!(card.llm_model.as_deref(), Some("sonnet"));

        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::TodosChanged(vec![Todo {
                content: "x".into(),
                status: TodoStatus::InProgress,
                active_form: "wiring cards".into(),
            }])),
            &clock,
        );
        assert_eq!(card.activity_summary, "wiring cards");

        let scratch = Arc::new(StreamScratch::default());
        card.fold_feed(
            ActorFeed::Detailed(StreamUpdate::ScratchChanged(scratch)),
            &clock,
        );
        // ScratchChanged must be matched (not dropped); activity may stay from todos.
        assert_eq!(card.activity_summary, "wiring cards");
    }

    #[test]
    fn resources_changed_does_not_clear_branch() {
        let mut card = SessionCard::new(SessionId::new("s"));
        let clock = crate::clock::ManualUiClock::new(0);
        card.fold_feed(ActorFeed::Summary(Box::new(base_summary())), &clock);
        card.fold_feed(ActorFeed::Detailed(StreamUpdate::ResourcesChanged), &clock);
        assert_eq!(card.git_branch.as_deref(), Some("main"));
    }
}
```

Isolation smoke (view observes own entity only) — add in `view.rs` tests once view exists; Step 1 can land the fold tests first.

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-ui --lib summary_fold_copies_enriched_scalars_and_seeds_one_repo -- --nocapture
cargo test -p lens-ui --lib detailed_fold_consumes_todos_and_scratch_for_activity -- --nocapture
```

Expected: FAIL (missing fields / match arms).

- [ ] **Step 3: Implement fold + SessionCardView**

Extend `SessionCard` with chrome fields. Implement:

```rust
impl SessionCard {
    pub fn fold_feed(&mut self, frame: ActorFeed, clock: &dyn UiClock) {
        match frame {
            ActorFeed::Summary(u) => self.fold_summary(&u, clock),
            ActorFeed::Detailed(u) => self.fold_detailed(u),
        }
    }

    pub fn fold_summary(&mut self, u: &SummaryUpdate, _clock: &dyn UiClock) {
        self.status = u.status;
        self.title = u.title.clone();
        self.last_total_tokens = u.last_total_tokens;
        self.needs_attention = u.needs_attention;
        self.subagent_active = u.subagent_active;
        self.llm_model = u.llm_model.clone();
        self.model_override = u.model_override.clone();
        self.agent_name = u.agent_name.clone();
        self.cumulative_cost = u.cumulative_cost.clone();
        self.context_window = u.context_window;
        self.sandbox_status = u.sandbox_status.clone();
        self.git_branch = u.git_branch.clone();
        self.workspace = u.workspace.clone();
        self.reasoning_effort = u.reasoning_effort.clone();
        self.activity_summary = u.activity_summary.clone();
        self.last_completed_turn = u.last_completed_turn;
        self.harness = u.harness.clone();
        self.repos = match (&u.workspace, &u.git_branch) {
            (None, None) => Vec::new(),
            (name, branch) => vec![RepoRef {
                name: name.clone().unwrap_or_else(|| "—".into()),
                branch: branch.clone(),
            }],
        };
        // Ready stamp: Task 4
    }

    pub fn fold_detailed(&mut self, u: StreamUpdate) {
        match u {
            StreamUpdate::Rebased(state) => {
                self.status = state.status;
                self.title = state.title.clone();
                self.last_task_error = state.last_task_error.clone();
                self.llm_model = state.llm_model.clone();
                self.model_override = state.model_override.clone();
                self.agent_name = state.agent_name.clone();
                self.cumulative_cost = state.cumulative_cost.clone();
                self.context_window = state.context_window;
                self.last_total_tokens = state.last_total_tokens;
                self.sandbox_status = state.sandbox_status.clone();
                self.git_branch = state.git_branch.clone();
                self.workspace = state.workspace.clone();
                self.reasoning_effort = state.reasoning_effort.clone();
                self.harness = state.harness.clone();
                self.lifecycle = state.lifecycle;
                self.needs_attention = !state.pending_elicitations.is_empty()
                    || state.status == SessionStatusValue::Failed;
                self.todos = state.todos.clone();
                self.refresh_activity_from_todos();
                self.repos = match (&state.workspace, &state.git_branch) {
                    (None, None) => Vec::new(),
                    (name, branch) => vec![RepoRef {
                        name: name.clone().unwrap_or_else(|| "—".into()),
                        branch: branch.clone(),
                    }],
                };
            }
            StreamUpdate::StatusChanged(s) => self.status = s,
            StreamUpdate::LastTaskErrorChanged(e) => self.last_task_error = e,
            StreamUpdate::UsageChanged(c) => self.cumulative_cost = c,
            StreamUpdate::ModelChanged {
                llm_model,
                model_override,
            } => {
                self.llm_model = llm_model;
                self.model_override = model_override;
            }
            StreamUpdate::ReasoningEffortChanged(e) => self.reasoning_effort = e,
            StreamUpdate::TodosChanged(todos) => {
                self.todos = todos;
                self.refresh_activity_from_todos();
            }
            StreamUpdate::ScratchChanged(_scratch) => {
                // Focused activity: prefer in-progress todo; scratch is consumed so
                // the match arm exists (activity must not stall while focused).
                self.refresh_activity_from_todos();
            }
            StreamUpdate::SandboxChanged(s) => self.sandbox_status = s,
            StreamUpdate::TitleChanged(t) => self.title = t,
            StreamUpdate::LastTokensChanged(t) => self.last_total_tokens = t,
            StreamUpdate::ContextWindowChanged(w) => self.context_window = w,
            StreamUpdate::AgentChanged { agent_name, .. } => self.agent_name = agent_name,
            StreamUpdate::ElicitationsChanged(e) => {
                self.needs_attention = !e.is_empty() || self.status == SessionStatusValue::Failed;
            }
            StreamUpdate::Reconnecting { .. } => {
                self.connection_overlay = ConnectionOverlay::Reconnecting;
            }
            StreamUpdate::Reconnected => {
                self.connection_overlay = ConnectionOverlay::Connected;
            }
            StreamUpdate::Disconnected(_) => {
                self.connection_overlay = ConnectionOverlay::Disconnected;
            }
            // SnapshotRestored: pending inputs only — does NOT seed card scalars.
            // ResourcesChanged: valueless marker — no branch delta.
            // TranscriptAdvanced: deferred with transcript slice.
            StreamUpdate::SnapshotRestored(_)
            | StreamUpdate::ResourcesChanged
            | StreamUpdate::TranscriptAdvanced { .. }
            | StreamUpdate::SkillsChanged(_)
            | StreamUpdate::TerminalPendingChanged(_)
            | StreamUpdate::PendingUserChanged(_)
            | StreamUpdate::ChildSessionChanged
            | StreamUpdate::PresenceChanged(_)
            | StreamUpdate::CollaborationModeChanged(_)
            | StreamUpdate::ModelOptionsChanged(_) => {}
        }
    }

    fn refresh_activity_from_todos(&mut self) {
        if let Some(t) = self
            .todos
            .iter()
            .find(|t| t.status == TodoStatus::InProgress)
        {
            self.activity_summary = t.active_form.clone();
        }
    }

    pub fn set_repos_for_test(&mut self, repos: Vec<RepoRef>) {
        self.repos = repos;
    }
}
```

`SessionCardView` — observe own card only; fixed outer size; no FleetStore observe:

```rust
use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px,
    prelude::*,
};
use std::cell::Cell;
use std::rc::Rc;

use super::model::{CARD_HEIGHT_PX, CARD_WIDTH_PX, SessionCard};

pub struct SessionCardView {
    card: Entity<SessionCard>,
    pub render_count: Rc<Cell<usize>>,
}

impl SessionCardView {
    pub fn new(card: Entity<SessionCard>, cx: &mut Context<Self>) -> Self {
        cx.observe(&card, |_, _, cx| cx.notify()).detach();
        Self {
            card,
            render_count: Rc::new(Cell::new(0)),
        }
    }
}

impl Render for SessionCardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        let card = self.card.read(cx);
        // Chrome details land in Task 4 — fixed outer bounds are the §4.4 load-bearing bit.
        div()
            .id(card.session_id.as_str())
            .w(px(CARD_WIDTH_PX))
            .h(px(CARD_HEIGHT_PX))
            .child(card.title.clone().unwrap_or_default())
    }
}

/// Mount as AnyView inside `.cached(...)` with stable bounds style.
pub fn cached_card_element(view: Entity<SessionCardView>) -> gpui::AnyElement {
    use gpui::{Stylable, Div};
    let style = gpui::StyleRefinement::default(); // pinned W×H also on wrapper in board
    view.into_any_element().cached(style)
}
```

**Audit note (commit message / code comment):** skeleton card chrome uses gpui `div`/`text` only — no gpui-component widget inside the tile — so §4.4 cache-key/bounds risk from component internals is N/A. If a later step adds e.g. `Tooltip`, verify it is a floating overlay (dirties hovered card only, no sibling reflow) and does not change outer tile bounds.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-ui --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/card
git commit -m "$(cat <<'EOF'
feat(lens-ui): dual-mode SessionCard fold and fixed-tile card view

EOF
)"
```

---

### Task 3: Board + focused recompose + slot API + ⌘. Action

**Files:**
- Create: `crates/lens-ui/src/slot/mod.rs`
- Create: `crates/lens-ui/src/board/mod.rs`
- Create: `crates/lens-ui/src/actions.rs`
- Modify: `crates/lens-ui/src/fleet/store.rs` (focus / promote / demote)
- Modify: `crates/lens-ui/src/lib.rs`
- Test: `crates/lens-ui/src/board/mod.rs`, `slot/mod.rs`, `fleet/store.rs` focus tests

**Interfaces:**
- Consumes: `SessionCommand::{Promote,Demote}`, FakeFleet `commands_tx`, `SessionCardView`, CARD_* constants.
- Produces:
  - `pub trait ContentTab { /* object-safe marker — empty for skeleton */ }`
  - `pub struct TabHandle { pub view: gpui::AnyView, pub title: SharedString }`
  - `impl TabHandle { pub fn set_title(&mut self, title: SharedString); }`
  - `pub struct PlaceholderTab;` implementing `Render` + `ContentTab`
  - `pub fn placeholder_tab(cx: &mut App) -> TabHandle`
  - `pub enum ShellMode { Board, Focused { session_id: SessionId } }`
  - `impl FleetStore { pub fn focus_session(&mut self, id: SessionId, cx: &mut Context<Self>); pub fn blur_to_board(&mut self, cx: &mut Context<Self>); pub fn focused(&self) -> Option<&SessionId>; }` — sends Promote to new focus, Demote to previous; membership notify only on mode/layout change
  - `pub struct BoardView { fleet: Entity<FleetStore>, card_views: HashMap<SessionId, Entity<SessionCardView>>, working_tab: TabHandle }`
  - `actions!(lens_ui, [BackToBoard]);` bound to `Command + .` (macOS)
  - Board click: click card → `focus_session`; click focused card again → `blur_to_board`

- [ ] **Step 1: Write failing focus / slot / action tests**

```rust
#[gpui::test]
async fn click_focus_sends_promote_and_demote_previous(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(0));
    let a = SessionId::new("a");
    let b = SessionId::new("b");
    let fleet = cx.update(|cx| {
        let f = FleetStore::new(clock, cx);
        f.update(cx, |f, cx| {
            f.spawn_fake_session(a.clone(), cx);
            f.spawn_fake_session(b.clone(), cx);
        });
        f
    });
    cx.update(|cx| {
        fleet.update(cx, |f, cx| f.focus_session(a.clone(), cx));
        fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx));
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let f = fleet.read(cx);
        let cmds_a = f.fake.as_ref().unwrap().take_commands(&a);
        let cmds_b = f.fake.as_ref().unwrap().take_commands(&b);
        assert!(
            cmds_a.iter().any(|c| matches!(c, SessionCommand::Promote)),
            "A promoted first"
        );
        assert!(
            cmds_a.iter().any(|c| matches!(c, SessionCommand::Demote)),
            "A demoted when B focused"
        );
        assert!(
            cmds_b.iter().any(|c| matches!(c, SessionCommand::Promote)),
            "B promoted"
        );
        assert_eq!(f.focused.as_ref(), Some(&b));
    });
}

#[test]
fn tab_handle_title_is_updatable() {
    // Construct PlaceholderTab / TabHandle in a unit test without window if possible;
    // otherwise #[gpui::test].
    let mut title = SharedString::from("Placeholder");
    title = SharedString::from("Updated");
    assert_eq!(&*title, "Updated");
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-ui --lib click_focus_sends_promote_and_demote_previous -- --nocapture
```

Expected: FAIL (`focus_session` missing).

- [ ] **Step 3: Implement slot API, board, actions, focus policy**

```rust
// crates/lens-ui/src/slot/mod.rs
use gpui::{AnyView, App, SharedString, IntoElement, Render, Window, div, prelude::*, Context};

pub trait ContentTab {}

pub struct TabHandle {
    pub view: AnyView,
    pub title: SharedString,
}

impl TabHandle {
    pub fn set_title(&mut self, title: SharedString) {
        self.title = title;
    }
}

pub struct PlaceholderTab;

impl ContentTab for PlaceholderTab {}

impl Render for PlaceholderTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Working area (placeholder)")
    }
}

pub fn placeholder_tab(cx: &mut App) -> TabHandle {
    let view = cx.new(|_| PlaceholderTab);
    TabHandle {
        view: view.into(),
        title: SharedString::from("Placeholder"),
    }
}
```

```rust
// crates/lens-ui/src/actions.rs
use gpui::{Action, actions};

actions!(lens_ui, [BackToBoard]);
```

```rust
// FleetStore::focus_session / blur_to_board
pub fn focus_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
    if self.focused.as_ref() == Some(&id) {
        self.blur_to_board(cx);
        return;
    }
    if let Some(prev) = self.focused.clone() {
        self.send_command(&prev, SessionCommand::Demote);
    }
    self.send_command(&id, SessionCommand::Promote);
    self.focused = Some(id);
    self.store_notify_count
        .set(self.store_notify_count.get().saturating_add(1));
    cx.notify(); // layout / mode change
}

pub fn blur_to_board(&mut self, cx: &mut Context<Self>) {
    if let Some(prev) = self.focused.take() {
        self.send_command(&prev, SessionCommand::Demote);
        self.store_notify_count
            .set(self.store_notify_count.get().saturating_add(1));
        cx.notify();
    }
}

fn send_command(&self, id: &SessionId, cmd: SessionCommand) {
    if let Some(tx) = self.command_txs.get(id) {
        let _ = tx.try_send(cmd);
    }
}
```

`BoardView::render`: if `focused.is_none()` → nav rail + ordinal grid of `.cached` cards; else → nav │ shrunk boards │ empty labeled chat │ empty navigator │ working-area with `TabHandle` placeholder. Register `BackToBoard` at app level (Task 6 binds keystroke); handler calls `fleet.blur_to_board`.

Wire a stub PTY counter for Task 5:

```rust
pub struct PtyProbe {
    pub bytes_sent: Rc<Cell<usize>>,
}
```

Board/key path must not increment it when `BackToBoard` fires.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-ui --lib
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src
git commit -m "$(cat <<'EOF'
feat(lens-ui): board/focused recompose, ContentTab slot, BackToBoard action

EOF
)"
```

---

### Task 4: Card chrome + wave ladder + §3.5 Ready policy

> **Review seam:** cross-family review required after this task.

**Files:**
- Create: `crates/lens-ui/src/card/wave.rs`
- Create: `crates/lens-ui/src/card/chrome.rs`
- Modify: `crates/lens-ui/src/card/model.rs` (Ready stamp on Summary fold)
- Modify: `crates/lens-ui/src/card/view.rs` (render chrome)
- Modify: `crates/lens-ui/src/fleet/store.rs` (schedule/cancel per-card decay timer; timer notifies **card only**)
- Test: `wave.rs`, Ready tests in `model.rs` / store

**Interfaces:**
- Consumes: `SessionCard` fields; `UiClock`; `READY_DECAY_MS: i64 = 5 * 60 * 1000`.
- Produces:
  - `pub enum Wave { NeedsInput, Ready, Working, Failed, Slept, Neutral }`
  - `pub fn derive_wave(card: &SessionCard, now_ms: i64, is_focused: bool) -> Wave`
    - NeedsInput if `needs_attention`
    - else Failed if `status == Failed` or `last_task_error.is_some()`
    - else Ready if `status == Idle && last_completed_at.is_some_and(|t| now_ms - t < READY_DECAY_MS)` **and not** `is_focused` (glow suppressed — return Neutral/Idle treatment when focused but Ready-true)
    - else Working if `status ∈ {Running, Launching, Waiting}`
    - else Slept if `lifecycle == Slept`
    - else Neutral
  - On **Summary** fold only: if `u.last_completed_turn > card.seen_turn` → `last_completed_at = clock.now_millis()`, `seen_turn = u.last_completed_turn`, ask FleetStore/card owner to `(re)schedule` one-shot timer at `last_completed_at + READY_DECAY_MS` that `card.update(|_, cx| cx.notify())` **only** (FleetStore notify must stay 0)
  - Seed/reconnect: same compare — if seed turn `<= seen_turn`, no stamp; if advanced, stamp
  - Initial spawn: `seen_turn = last_completed_turn` from first Summary **without** stamping (pre-attach completions show no Ready — spec §3.5 limitation). Use a `seeded: bool` flag: first Summary initializes `seen_turn` only; subsequent advances stamp.
  - Chrome: status tile, `<STATUS>`/`title`, `<harness> · <model>`, **reserved** activity line (blank if empty — never collapse), repos **one row** + `·+N` + hover tooltip for full list, footer host/`~$spend`/`ctx %`, in-bounds connection overlay + Failed Retry in footer/activity slot
  - Kebab: Sleep → `SessionCommand::Sleep`, Send → `Send` — **not** Interrupt

- [ ] **Step 1: Write failing Ready + wave + chrome layout tests**

```rust
#[test]
fn wave_ladder_priority_needs_input_over_ready() {
    let mut card = SessionCard::new(SessionId::new("s"));
    card.status = SessionStatusValue::Idle;
    card.needs_attention = true;
    card.last_completed_at = Some(1_000);
    assert_eq!(
        derive_wave(&card, 1_100, false),
        Wave::NeedsInput
    );
}

#[test]
fn ready_requires_idle_and_recent_completion_suppressed_when_focused() {
    let mut card = SessionCard::new(SessionId::new("s"));
    card.status = SessionStatusValue::Idle;
    card.last_completed_at = Some(1_000);
    assert_eq!(derive_wave(&card, 1_000 + 60_000, false), Wave::Ready);
    assert_ne!(derive_wave(&card, 1_000 + 60_000, true), Wave::Ready);
    assert_ne!(
        derive_wave(&card, 1_000 + READY_DECAY_MS + 1, false),
        Wave::Ready
    );
}

#[test]
fn summary_fold_stamps_ready_on_turn_advance_not_status_edge() {
    let clock = ManualUiClock::new(5_000);
    let mut card = SessionCard::new(SessionId::new("s"));
    let mut u = base_summary(); // from Task 2 helper
    u.status = SessionStatusValue::Idle;
    u.last_completed_turn = 2;
    card.fold_summary(&u, &clock); // seed: seen_turn=2, no stamp
    assert!(card.last_completed_at.is_none());
    assert_eq!(card.seen_turn, 2);

    u.last_completed_turn = 5; // coalesced idle→running→idle appears as turn jump
    u.status = SessionStatusValue::Idle;
    card.fold_summary(&u, &clock);
    assert_eq!(card.last_completed_at, Some(5_000));
    assert_eq!(card.seen_turn, 5);
}

#[test]
fn repos_render_one_row_with_overflow_badge() {
    let row = format_repos_row(&[
        RepoRef { name: "a".into(), branch: Some("main".into()) },
        RepoRef { name: "b".into(), branch: None },
        RepoRef { name: "c".into(), branch: None },
    ]);
    assert!(row.contains("·+2"), "overflow badge: {row}");
    assert!(!row.contains('\n'));
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-ui --lib summary_fold_stamps_ready_on_turn_advance_not_status_edge -- --nocapture
cargo test -p lens-ui --lib ready_requires_idle_and_recent_completion_suppressed_when_focused -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement wave, Ready stamp, chrome, decay timer**

Ready stamp in `fold_summary`:

```rust
pub const READY_DECAY_MS: i64 = 5 * 60 * 1000;

// inside fold_summary, after copying scalars:
if !self.seeded {
    self.seen_turn = u.last_completed_turn;
    self.seeded = true;
} else if u.last_completed_turn > self.seen_turn {
    self.last_completed_at = Some(clock.now_millis());
    self.seen_turn = u.last_completed_turn;
    self.ready_reschedule = true; // poller/store clears + schedules timer
}
```

FleetStore / poller after fold: if `card.ready_reschedule`, schedule:

```rust
let fire_at = last_completed_at + READY_DECAY_MS;
let delay = (fire_at - clock.now_millis()).max(0) as u64;
// gpui timer: cx.spawn async { Timer::after(Duration::from_millis(delay)).await; card.update(... notify only) }
```

Cancel previous timer handle when re-stamping. **Never** `FleetStore::notify` from the decay timer.

Chrome: reserved activity `div().h(px(16)).child(activity_or_empty)`; repos via `format_repos_row`; overlay `div().absolute().inset_0()` when disconnected — inside fixed tile.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-ui --lib
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/card crates/lens-ui/src/fleet
git commit -m "$(cat <<'EOF'
feat(lens-ui): card chrome, wave ladder, and Ready stamp/decay timers

EOF
)"
```

---

### Task 5: §6.1 acceptance test in TestAppContext

**Files:**
- Create: `crates/lens-ui/tests/acceptance_shell.rs` (or `src/board/acceptance.rs` with `#[gpui::test]`)
- Modify: instrumentation hooks on `SessionCardView` / `FleetStore` / `PtyProbe` as needed
- Test: the single acceptance module covering **all** sub-assertions

**Interfaces:**
- Consumes: everything from Tasks 1–4; `FakeFleet`; `ManualUiClock`.
- Produces: hermetic proof that merge-gates the mechanism (live-verify still required in Task 7).

**ALL sub-assertions (must appear as asserts in the test):**

1. Mount real board + N cards with `.cached(...)`; settle first frame via targeted `card.update(|_,cx| cx.notify())` — **not** `cx.refresh()`.
2. Inject Summary on session B → B re-renders; A does **no** render work (`render_count` unchanged).
3. Downstream sibling C: after B grows activity idle→present **and** repos `1→3` (via `set_repos_for_test` + activity fold — SummaryUpdate cannot carry N repos yet; document), **C bounds byte-equal** and C paint/render unchanged.
4. `FleetStore` notify count **unchanged** across scalar folds (== 0 delta).
5. Mode-switch order-safety with lagging poller: enqueue Summary frames, then Promote, then Detailed on unified feed without draining mid-way; after drain, card ends on Detailed projection; Demote emits Summary restoring coarse projection.
6. Ready: (a) single Summary with jumped `last_completed_turn` → Ready; (b) send→running clears Ready, later idle+bump re-lights; (c) after `READY_DECAY` via injected clock + one-shot, Ready clears and **only** card notified; (d) glow suppressed when focused; (e) reconnect seed with no advance keeps within-window Ready; advanced seed re-stamps.
7. `⌘.` / `BackToBoard`: with terminal-focused placeholder tab + `PtyProbe`, fire Action → Demote + **`PtyProbe.bytes_sent == 0`**.

- [ ] **Step 1: Write the failing acceptance test (full)**

```rust
// crates/lens-ui/tests/acceptance_shell.rs
use gpui::{prelude::*, px, Bounds, Pixels, Size};
use lens_core::actor::{ActorFeed, SessionCommand, SummaryUpdate};
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::usage::Cost;
use lens_core::reduce::StreamUpdate;
use lens_ui::actions::BackToBoard;
use lens_ui::board::BoardView;
use lens_ui::card::model::{RepoRef, SessionCard, READY_DECAY_MS};
use lens_ui::card::wave::{derive_wave, Wave};
use lens_ui::clock::ManualUiClock;
use lens_ui::fleet::store::FleetStore;
use lens_ui::PtyProbe;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

fn summary(
    status: SessionStatusValue,
    title: &str,
    activity: &str,
    turn: u32,
    workspace: Option<&str>,
) -> SummaryUpdate {
    SummaryUpdate {
        status,
        title: Some(title.into()),
        last_total_tokens: Some(1_000),
        host_id: None,
        needs_attention: false,
        subagent_active: false,
        llm_model: Some("opus".into()),
        model_override: None,
        agent_name: None,
        cumulative_cost: Cost::default(),
        context_window: Some(200_000),
        sandbox_status: None,
        git_branch: Some("main".into()),
        workspace: workspace.map(str::to_string),
        reasoning_effort: None,
        activity_summary: activity.into(),
        last_completed_turn: turn,
        harness: Some("claude-native".into()),
    }
}

#[gpui::test]
async fn shell_skeleton_acceptance(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(10_000));
    let a = SessionId::new("a");
    let b = SessionId::new("b");
    let c = SessionId::new("c");

    let (fleet, board, views, pty) = cx.update(|cx| {
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn lens_ui::clock::UiClock>, cx);
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(a.clone(), cx);
            f.spawn_fake_session(b.clone(), cx);
            f.spawn_fake_session(c.clone(), cx);
        });
        let board = BoardView::new(fleet.clone(), cx); // builds SessionCardViews + .cached mount
        let views = board.card_views_for_test(); // HashMap<SessionId, Entity<SessionCardView>>
        let pty = PtyProbe {
            bytes_sent: Rc::new(Cell::new(0)),
        };
        board.set_pty_probe_for_test(pty.clone());
        (fleet, board, views, pty)
    });

    // (1) Settle first frame — targeted notify only (never cx.refresh()).
    for id in [&a, &b, &c] {
        let card = cx.read(|cx| fleet.read(cx).card(id).unwrap());
        cx.update(|cx| {
            card.update(cx, |_, cx| cx.notify());
        });
    }
    cx.run_until_parked();

    let renders0 = |id: &SessionId| {
        cx.read(|cx| views[id].read(cx).render_count.get())
    };
    let a0 = renders0(&a);
    let b0 = renders0(&b);
    let c0 = renders0(&c);
    let store0 = cx.read(|cx| fleet.read(cx).store_notify_count());

    let bounds_c_before: Bounds<Pixels> = cx.update(|cx| {
        board.card_bounds_for_test(&c, cx) // must return layout bounds of C's cached tile
    });

    // (2) Inject Summary on B only.
    cx.update(|cx| {
        let f = fleet.read(cx);
        f.fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Running,
                "B-live",
                "",
                1,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();

    assert!(renders0(&b) > b0, "B must re-render on its Summary fold");
    assert_eq!(renders0(&a), a0, "A sibling must not re-render");
    let store1 = cx.read(|cx| fleet.read(cx).store_notify_count());
    assert_eq!(store1, store0, "(4) FleetStore notify==0 on scalar fold");

    // (3) Size-invariance: activity present + repos 1→3 on B; C bounds byte-equal.
    cx.update(|cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card, cx| {
            card.activity_summary = "wiring isolation".into();
            card.set_repos_for_test(vec![
                RepoRef {
                    name: "lens".into(),
                    branch: Some("main".into()),
                },
                RepoRef {
                    name: "omnigent".into(),
                    branch: Some("dev".into()),
                },
                RepoRef {
                    name: "other".into(),
                    branch: None,
                },
            ]);
            cx.notify();
        });
    });
    cx.run_until_parked();
    let bounds_c_after = cx.update(|cx| board.card_bounds_for_test(&c, cx));
    assert_eq!(
        bounds_c_after, bounds_c_before,
        "C bounds must be byte-equal after B content growth"
    );
    assert_eq!(renders0(&c), c0, "downstream C must not paint/render");

    // (5) Lagging mode-switch order-safety on B's unified feed.
    cx.update(|cx| {
        let fake = fleet.read(cx).fake.as_ref().unwrap();
        // Queue Summary then Promote command then Detailed without intermediate drain.
        fake.push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "stale-summary",
                "",
                2,
                Some("lens"),
            ))),
        );
        fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx)); // enqueues Promote
        fake.push_feed(
            &b,
            ActorFeed::Detailed(StreamUpdate::StatusChanged(SessionStatusValue::Running)),
        );
        fake.push_feed(
            &b,
            ActorFeed::Detailed(StreamUpdate::TitleChanged(Some("detailed-title".into()))),
        );
    });
    cx.run_until_parked();
    let title = cx.read(|cx| fleet.read(cx).card(&b).unwrap().read(cx).title.clone());
    assert_eq!(
        title.as_deref(),
        Some("detailed-title"),
        "must end on Detailed projection, not stale Summary"
    );
    cx.update(|cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    cx.run_until_parked();
    // FakeFleet should accept Demote; push a Summary seed-after-demote frame:
    cx.update(|cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "post-demote",
                "",
                2,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    let title = cx.read(|cx| fleet.read(cx).card(&b).unwrap().read(cx).title.clone());
    assert_eq!(title.as_deref(), Some("post-demote"));

    // (6a) Coalesce-safe Ready: single Summary with jumped turn.
    clock.set(20_000);
    cx.update(|cx| {
        // Fresh card D or reset seen_turn via respawn semantics on B:
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card, _| {
            card.seeded = true;
            card.seen_turn = 2;
            card.last_completed_at = None;
            card.status = SessionStatusValue::Idle;
        });
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "done",
                "",
                9, // jumped past seen_turn=2
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(20_000));
        assert_eq!(
            derive_wave(card, 20_000 + 1_000, false),
            Wave::Ready,
            "(6a) monotonic turn jump lights Ready"
        );
        assert_ne!(
            derive_wave(card, 20_000 + 1_000, true),
            Wave::Ready,
            "(6d) glow suppressed when focused"
        );
    });

    // (6b) running clears Ready; later idle+bump re-lights.
    cx.update(|cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Running,
                "busy",
                "working",
                9,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_ne!(derive_wave(card, 21_000, false), Wave::Ready);
    });
    clock.set(22_000);
    cx.update(|cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "done2",
                "",
                10,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(derive_wave(card, 22_000 + 1_000, false), Wave::Ready);
    });

    // (6c) Decay via injected clock + one-shot notifies only the card.
    let store_before_decay = cx.read(|cx| fleet.read(cx).store_notify_count());
    let card_notifies_before = cx.read(|cx| {
        fleet.read(cx).card(&b).unwrap().read(cx).notify_count
    });
    clock.set(22_000 + READY_DECAY_MS + 1);
    cx.run_until_parked(); // fire scheduled decay timer
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_ne!(
            derive_wave(card, clock.now_millis(), false),
            Wave::Ready,
            "Ready clears after READY_DECAY"
        );
        assert!(card.notify_count > card_notifies_before);
        assert_eq!(
            fleet.read(cx).store_notify_count(),
            store_before_decay,
            "decay timer must not notify FleetStore"
        );
    });

    // (6e) Reconnect seed: no advance keeps within-window Ready; advanced seed re-stamps.
    clock.set(30_000);
    cx.update(|cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card, _| {
            card.last_completed_at = Some(29_000);
            card.seen_turn = 10;
            card.seeded = true;
            card.status = SessionStatusValue::Idle;
        });
        // Seed with same turn — must NOT clear last_completed_at.
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "reconn",
                "",
                10,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(29_000));
        assert_eq!(derive_wave(card, 30_000, false), Wave::Ready);
    });
    cx.update(|cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "reconn2",
                "",
                11,
                Some("lens"),
            ))),
        );
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(30_000));
        assert_eq!(card.seen_turn, 11);
    });

    // (7) BackToBoard — Demote + zero PTY bytes.
    cx.update(|cx| {
        fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx));
        board.focus_working_tab_for_test(cx); // terminal-focused placeholder
        pty.bytes_sent.set(0);
        cx.dispatch_action(&BackToBoard); // app-level Action
    });
    cx.run_until_parked();
    assert_eq!(pty.bytes_sent.get(), 0, "⌘. must not send PTY bytes");
    cx.read(|cx| {
        assert!(fleet.read(cx).focused.is_none());
        let cmds = fleet.read(cx).fake.as_ref().unwrap().take_commands(&b);
        assert!(
            cmds.iter().any(|c| matches!(c, SessionCommand::Demote)),
            "BackToBoard must Demote"
        );
    });
}
```

Helpers this test requires (add in Task 5 Step 3 if missing): `BoardView::card_views_for_test`, `card_bounds_for_test`, `set_pty_probe_for_test`, `focus_working_tab_for_test`, `PtyProbe`, and `SessionCard.seeded` (from Task 4).

- [ ] **Step 2: Run — expect FAIL on asserts**

```bash
cargo test -p lens-ui --test acceptance_shell -- --nocapture
```

Expected: FAIL on isolation / Ready / pty asserts (not compile errors).

- [ ] **Step 3: Fix any product gaps revealed by the test (minimal)**

Only change product code needed for green acceptance — e.g. bounds capture helper, PtyProbe wiring on BackToBoard, seed flag edge cases. Do **not** weaken asserts. Do **not** use `cx.refresh()`.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-ui --test acceptance_shell -- --nocapture
cargo clippy -p lens-ui --all-targets -- -D warnings
cargo fmt --check
```

Expected: PASS / clean.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui
git commit -m "$(cat <<'EOF'
test(lens-ui): §6.1 shell skeleton acceptance in TestAppContext

EOF
)"
```

---

### Task 6: lens-app live bootstrap (single-session smoke)

**Files:**
- Modify: `crates/lens-app/Cargo.toml` (add lens-core, lens-client, async-channel, crossbeam-channel, url, serde_json)
- Modify: `crates/lens-app/src/main.rs`
- Modify: `crates/lens-ui/src/fleet/store.rs` (real scheduler path: `FleetStore::attach_live_session(...)`)
- Test: compile + optional ignored live smoke `#[ignore]` test documenting flags

**Interfaces:**
- Consumes: lens-drive wiring shape — `Client`, `Connection`, `open_stores`, `ClientSessionApi::new`, `Box<dyn Clock+Send>`, `async_channel::bounded(64)`, EventStream→crossbeam forwarder, `FleetScheduler::reconnect` / `wake` with **`OutputMode::Summary`** for background, `outcomes.clone()`, `SessionCommand`.
- Produces:
  - `lens-app` binary: `gpui_component::init(cx)`; window wrapped in `gpui_component::Root`; builds `FleetStore` with real scheduler; env `LENS_OMNIGENT_URL` / `LENS_OMNIGENT_TOKEN` / `--session`
  - `FleetStore::spawn_live_session(&mut self, conn, client, session_id, data_dir, cx) -> Result<Entity<SessionCard>, LiveSpawnError>`
  - Registers `BackToBoard` keybinding `KeyBinding::new("cmd-.", BackToBoard, None)` with app-level priority

- [ ] **Step 1: Write failing compile/smoke harness stub**

Add `crates/lens-app/tests/smoke_compile.rs` or a `#[test] fn live_spawn_api_exists()` in lens-ui that references `FleetStore::spawn_live_session` — expect FAIL until implemented.

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-ui --lib live_spawn -- --nocapture
cargo build -p lens-app
```

Expected: FAIL missing `spawn_live_session` / incomplete main.

- [ ] **Step 3: Implement live spawn + main**

Core of `spawn_live_session` (mirror `lens-drive` `attach_actor`, but Summary + poller):

```rust
pub fn spawn_live_session(
    &mut self,
    conn: &lens_client::Connection,
    client: &lens_client::Client,
    session_id: SessionId,
    data_dir: &std::path::Path,
    cx: &mut Context<Self>,
) -> Result<Entity<SessionCard>, String> {
    let (feed_tx, feed_rx) = async_channel::bounded(FEED_CAPACITY);
    let stream = client
        .sessions()
        .stream(&session_id)
        .map_err(|e| format!("stream: {e}"))?;
    let (events_tx, events_rx) = crossbeam_channel::bounded(1024);
    // spawn forwarder thread: while let Some(ev) = stream.recv() { events_tx.send(ev) }
    let stores = open_stores(data_dir, &conn.id, &session_id)?;
    let api = Box::new(lens_core::actor::ClientSessionApi::new(
        lens_client::Client::new(conn.clone()).map_err(|e| e.to_string())?,
    ));
    let clock = Box::new(WallClock);
    let scheduler = self.scheduler.get_or_insert_with(FleetScheduler::new);
    scheduler
        .reconnect(
            &conn.id,
            &session_id,
            events_rx,
            feed_tx,
            lens_core::actor::OutputMode::Summary,
            stores,
            clock,
            api,
        )
        .map_err(|e| format!("{e:?}"))?;
    let outcomes_rx = scheduler
        .handle(&session_id)
        .ok_or_else(|| "handle missing".to_string())?
        .outcomes
        .clone();
    let commands = scheduler
        .handle(&session_id)
        .unwrap()
        .commands
        .clone();
    self.command_txs.insert(session_id.clone(), commands);
    let card = cx.new(|_| SessionCard::new(session_id.clone()));
    let _task = spawn_session_poller(
        card.clone(),
        feed_rx,
        outcomes_rx,
        Arc::clone(&self.clock),
        cx,
    );
    self.cards.insert(session_id, card.clone());
    self.store_notify_count
        .set(self.store_notify_count.get().saturating_add(1));
    cx.notify();
    Ok(card)
}
```

`main.rs` pattern from spikes:

```rust
Application::new().run(|cx: &mut App| {
    gpui_component::init(cx);
    // register BackToBoard keybinding
    cx.open_window(WindowOptions::default(), |window, cx| {
        let root_view = cx.new(|cx| AppShell::new(/* fleet + board */, window, cx));
        let any: gpui::AnyView = root_view.into();
        cx.new(|cx| gpui_component::Root::new(any, window, cx))
    })
    .ok();
    cx.activate(true);
});
```

- [ ] **Step 4: Build + unit tests PASS**

```bash
cargo test -p lens-ui --lib
cargo build -p lens-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS / clean. Live window smoke is manual / Task 7.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-app crates/lens-ui
git commit -m "$(cat <<'EOF'
feat(lens-app): bootstrap Root shell on real FleetScheduler (Summary spawn)

EOF
)"
```

---

### Task 7: Fleet-spawn harness + N≥10 live-verify HARD GATE

**Files:**
- Create: `crates/lens-app/src/fleet_verify.rs` **or** extend `crates/lens-drive` with `--fleet-verify`
- Create: `scripts/fleet-verify.sh` (optional) documenting omnigent install
- Modify: `crates/lens-app/src/main.rs` (`--fleet-verify` mode)
- Test: live harness (not hermetic) — exit 0 only when green

**Interfaces:**
- Consumes: running omnigent **0.5.1** (`installing-omnigent-from-source` skill); `lens-capture` / REST create to spawn ≥10 sessions; `FleetStore` / scheduler Summary wake; promote/demote cycles.
- Produces:
  - Harness that: creates ≥10 sessions, attaches each with **per-session** feed + `OutputMode::Summary`, renders or headlessly folds cards, promotes one, demotes, promotes another; asserts no panic, feeds progressing, mode-switch order OK
  - **Merge gate statement:** PR/merge is **blocked** until this harness exits 0 against omnigent 0.5.1

- [ ] **Step 1: Write harness that fails closed without server**

```rust
// fleet_verify entry: if omnigent unreachable → exit 2 with clear message
// if N < 10 sessions attached → exit 1
// scripted promote/demote; assert command outcomes / summary frames received
```

- [ ] **Step 2: Run without server — expect non-zero**

```bash
cargo run -p lens-app -- --fleet-verify --base-url http://127.0.0.1:9
```

Expected: exit ≠ 0 (connection refused / gate failed).

- [ ] **Step 3: Implement full harness**

Procedure (document in harness `--help`):

1. `omnigent --version` must report `0.5.1` and pinned commit per `vendor/omnigent-0.5.1/README.md` (use `installing-omnigent-from-source` skill).
2. Start server if needed.
3. Create ≥10 sessions (REST via lens-client, or drive `lens_capture` / harness spawn loop).
4. For each session: seed disk like lens-drive, `wake`/`reconnect` with **Summary**, per-session `bounded(64)` feed, clone outcomes, spawn poller (headless OK — no window required for gate).
5. Drive: wait for Summary seed on each; `Promote` session 0; assert Detailed/`Rebased`; `Demote`; assert Summary; `Promote` session 1; repeat a few cycles.
6. Exit 0 only if all assertions hold.

- [ ] **Step 4: Live-verify green (HARD GATE)**

```bash
# After omnigent 0.5.1 is up:
cargo run -p lens-app -- --fleet-verify --base-url "$LENS_OMNIGENT_URL" --count 10
```

Expected: exit 0. **Merge is blocked until this is green.**

Also:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test -p lens-ui
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-app crates/lens-drive scripts
git commit -m "$(cat <<'EOF'
test(lens-app): N≥10 fleet-verify live gate vs omnigent 0.5.1

EOF
)"
```

---

## Self-Review

### 1. Spec coverage

| Spec section | Task |
| --- | --- |
| §3.1–§3.4 lens-core | **Out of scope** (already merged) — noted in preamble |
| §3.5 Ready stamp/decay/focus suppress/reconnect | Task 4 (+ asserted in Task 5) |
| §4.1 poller coalesce async-only | Task 1 |
| §4.2 dual-mode fold incl. Todos/Scratch | Task 2 |
| §4.3 FleetStore ownership / one connection | Tasks 1, 6 |
| §4.4 isolation fixed tile / observe own / store notify | Tasks 2, 4, 5 |
| §4.5 commands + ActorOutcome (`FeedConsumerGone`) / no Interrupt | Tasks 1, 3, 4 |
| §5.1 click toggle + ⌘. Action / defer ⌘\ ⌘D | Task 3 (+ Task 5 pty) |
| §5.2 ContentTab/TabHandle/placeholder only | Task 3 |
| §6 chrome + wave ladder | Task 4 |
| §6.1 acceptance all sub-asserts | Task 5 |
| §7 FakeFleet + live-verify N≥10 | Tasks 1, 7 |
| Per-session feed channels | Task 1 preamble + FakeFleet |
| Outcomes clone mechanism | Preamble + Tasks 1, 6 |
| gpui 0.2.2 + gpui-component 0.5.1 + Root init | Tasks 1, 6 |
| Live-verify hard merge gate | Task 7 |

### 2. Placeholder scan

No TBD / “similar to Task N” / bare `todo!` in executable steps. Task 5 Step 1 is a full assert body covering §6.1 (1)–(7). Multi-repo on SummaryUpdate noted explicitly (`set_repos_for_test`) because the core type has a single `workspace` field — a typed gap, not a placeholder.

### 3. Type consistency

- `ActorFeed::Summary(Box<SummaryUpdate>)` used throughout (matches `feed.rs`).
- `FeedConsumerGone` not `SummaryConsumerGone`.
- `SessionCommand` has no `Interrupt`.
- Outcomes: `handle.outcomes.clone()` after spawn — sole poller consumer.
- `TabHandle { view: AnyView, title: SharedString }` with `set_title`.
- `READY_DECAY_MS = 5 * 60 * 1000`; Ready = idle ∧ recent `last_completed_at`; `seen_turn` detector only.
- Frame driver: `cx.notify` on card entity only — never `refresh()`.

Plan saved; do not git-commit the plan file unless the orchestrator asks.

---

**Plan complete and saved to `docs/plans/2026-07-15-lens-ui-shell-skeleton.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review at seams (Tasks 2 & 4) + Opus final pass after Task 7  
2. **Inline Execution** — executing-plans in-session with checkpoints  

Which approach?
