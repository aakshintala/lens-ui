# Terminal Slice 5 — Sub-slice D (fleet-integration) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route the session-level control outcomes B landed (`Superseded`, `TerminalResource`) into `FleetStore` so resource signals reach owned terminals and a `/clear` supersede re-parents the terminal into the new session B with its scrollback intact.

**Architecture:** The poller already holds `WeakEntity<FleetStore>` and already routes `TransportChanged` to the store; D adds two explicit match arms that convert the two control outcomes into a typed `SessionControl` and call one new store entry point, `FleetStore::on_session_control`. Resource signals fan out to every owned terminal of that session as `TerminalHostEvent::ResourceCreated/Deleted` (the tab filters to its own identity — Slice-4 contract). Supersede is handled behind a new injected **`SessionLoader`** seam: the store asks the loader to make session B reachable (the real impl does a background `GET /v1/sessions/{id}` → `seed_disk` → `spawn_live_session`; tests inject a fake), then moves A's terminal members to B — **re-subscribing each member to B** — and drives `TerminalHostEvent::Transfer { new_session: B }`.

**Tech Stack:** Rust, gpui (`Entity`/`Context`/`Task`/`cx.subscribe`), `lens-ui` (`FleetStore`), `lens-core` (`ActorOutcome`), `lens-terminal` (`TerminalTab`, `TerminalHostEvent`), `lens-client` (`Client::sessions().get`).

## Global Constraints

- **Branch:** `terminal-slice-5-fleetstore`. D does **not** merge independently — whole slice 5 (A+B+C+D) merges to `main` together after D + final whole-branch review + live riders.
- **Per-task gate before every commit** (per memory `per-task-gate-must-run-clippy`):
  `cargo test -p lens-ui --lib -- --test-threads=4` +
  `cargo clippy -p lens-ui --all-targets -- -D warnings` + `cargo fmt --all -- --check`.
  Tasks touching `lens-terminal` also run `cargo test -p lens-terminal --lib -- --test-threads=4`.
  Do **not** run real-window harnesses. If only `engine::handle::tests::wheel_no_tracking_local_scrolls_without_egress` fails with a ~15s runtime, it is the known oversubscription flake (memory `worker-stall-gate-busy-spin-flake`) — re-run isolated to confirm.
- **Design SSOT:** `docs/specs/2026-07-22-terminal-slice-5-fleet-membership-design.md` §4.1, §9, §10, §13.
- **Review discipline:** every task gets a fresh cross-family review; codex quota is exhausted this week (memory `codex-quota-exhausted-week-2026-07-23`) → use an **Opus** subagent or `grok-4.5` via cursor-delegate. Author is `composer-2.5`, so composer cannot review its own task.
- **Do not weaken A's frozen-engine seam.** Nothing in D may bypass `TerminalTab::on_host_event` to reach engine internals.

## Resolved planning questions

The design deferred four items to "D planning". All four are resolved here — record these in the task-1 commit message so reviewers do not re-litigate them:

1. **Headless load-B is reachable, but not free (§10 step 1, §14).** `spawn_live_session` (`crates/lens-ui/src/fleet/store.rs:353`) is **not** UI-entangled — no `Window`, focus, or user gesture; `crates/lens-app/src/fleet_verify.rs:73` already calls it headlessly. But two gaps make it uncallable from inside a store handler: (a) `FleetStore` retains **no** `Connection`/`Client`/`data_dir` (`store.rs:59-77`), and (b) a brand-new B must be seeded to the control store first — `scheduler.reconnect` does `load_session(...).ok_or(SessionNotFound)` (`crates/lens-core/src/actor/scheduler.rs:103-105`), and seeding needs a `SessionSnapshot` from `GET /v1/sessions/{id}` (`crates/lens-app/src/main.rs:454-459`, `632-644`). **Decision (user, 2026-07-23): full supersede in D behind a `SessionLoader` seam** — store-layer logic headless-tested with a fake loader; the real GET→seed→spawn impl lives in `lens-app` and is proven by the live rider.
2. **The `GET` must not block the foreground.** `Sessions::get` is blocking (`crates/lens-client/src/sessions.rs:1281`). The loader therefore returns a `Task<Result<(), String>>` and does its blocking IO on `cx.background_executor()`. If `Client` is not `Send`, construct one inside the background task via `Client::new(conn.clone())` — the pattern `spawn_live_session` already uses at `store.rs:383-385`.
3. **The persisted-item `map_item` path (§4.2) is NOT needed.** It would only matter if D discovered B's transferred terminal via B's snapshot. D drives `Transfer` directly off the `Superseded` outcome (Q8), so the item path stays unbuilt. Do not add it.
4. **D does not re-prove "both race orders → adoption" (§9/§13).** `lens-ui` cannot synthesize a `4404` — it arrives on the transport/bridge path, which is `lens-terminal`-internal with no public seam. A already binds the real chain end-to-end (`fourohfour_first_then_delete_create_adopts`), and the real interleaving is a live rider. **D's testable obligation is forwarding fidelity:** the right host event reaches exactly the owned terminals of the signalling session, and no others.

## Known trap (do not miss)

`register_terminal_member` captures `session_for_sub` **into the subscription closure** at insert time (`crates/lens-ui/src/fleet/terminal.rs:241-247`), and `on_terminal_presentation_changed` early-returns when `terminals[session][key]` is absent (`terminal.rs:274-280`). A naive `terminals[A].remove(k)` → `terminals[B].insert(k, member)` therefore leaves the member's subscription calling back with **session A**, where the key no longer exists — silently killing `pending_sleep` deferred-sleep for the transferred terminal. **The move must build a fresh subscription bound to B** while preserving `last_viewed`, `hidden`, and `pending_sleep`. Task 4 exists specifically to close this, and its test asserts the post-move deferred sleep still fires.

## File structure

| File | Responsibility | Task |
| --- | --- | --- |
| `crates/lens-terminal/src/lib.rs` | add `test-util`-gated host-event recorder on `TerminalTab` | 1 |
| `crates/lens-ui/Cargo.toml` | dev-dep on `lens-terminal` with `test-util` feature | 1 |
| `crates/lens-ui/src/fleet/poller.rs` | two new match arms → `on_session_control` | 2 |
| `crates/lens-ui/src/fleet/terminal.rs` | `SessionControl`, `on_session_control`, resource forwarding, `move_terminal_members`, supersede handler | 2,4,5 |
| `crates/lens-ui/src/fleet/loader.rs` (new) | `SessionLoader` trait + test fake | 3 |
| `crates/lens-ui/src/fleet/store.rs` | `session_loader` field + setter | 3 |
| `crates/lens-app/src/loader.rs` (new) | real `AppSessionLoader` (GET → seed → spawn) | 6 |
| `crates/lens-app/src/main.rs` | wire the real loader into the store | 6 |

---

### Task 1: Host-event recorder seam on `TerminalTab`

D asserts *"the store forwarded the right host event to the right tabs"*. A tab built by `open_with_engine_for_test` has no `current_tid`, so it correctly **ignores** resource signals — meaning there is no observable side effect to assert on. This task adds a minimal, feature-gated recorder so forwarding is provable. It records only; it changes no behavior.

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` (the `TerminalTab` struct, its constructors, and `on_host_event` at ~`:663`)
- Modify: `crates/lens-ui/Cargo.toml` (dev-dependencies)

**Interfaces:**
- Produces: `TerminalTab::host_events_for_test(&self) -> &[TerminalHostEvent]` — available under `#[cfg(any(test, feature = "test-util"))]`. Tasks 2 and 5 assert against it.

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `crates/lens-terminal/src/lib.rs` (find the module that already exercises `open_with_engine_for_test`; match its imports and helper style):

```rust
#[gpui::test]
async fn host_event_recorder_records_in_order(cx: &mut gpui::TestAppContext) {
    let engine = std::sync::Arc::new(
        EngineHandle::spawn(engine_config_for_test()).expect("engine"),
    );
    let tab = cx.update(|cx| TerminalTab::open_with_engine_for_test(engine, cx));
    cx.update(|cx| {
        tab.update(cx, |tab, cx| {
            tab.on_host_event(
                TerminalHostEvent::ResourceDeleted {
                    terminal_id: TerminalId::new("term_1"),
                },
                cx,
            );
            tab.on_host_event(TerminalHostEvent::Sleep, cx);
        });
    });
    cx.update(|cx| {
        let recorded = tab.read(cx).host_events_for_test().to_vec();
        assert_eq!(recorded.len(), 2, "both host events recorded");
        assert!(
            matches!(recorded[0], TerminalHostEvent::ResourceDeleted { .. }),
            "first recorded event is ResourceDeleted, got {:?}",
            recorded[0]
        );
        assert!(
            matches!(recorded[1], TerminalHostEvent::Sleep),
            "second recorded event is Sleep, got {:?}",
            recorded[1]
        );
    });
}
```

If `engine_config_for_test()` is not in scope in that module, use the local helper the neighbouring tests use to build an `EngineConfig`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-terminal --lib host_event_recorder_records_in_order -- --test-threads=4`
Expected: FAIL to compile — `no method named host_events_for_test found for struct TerminalTab`.

- [ ] **Step 3: Write minimal implementation**

Add the field to the `TerminalTab` struct definition:

```rust
    #[cfg(any(test, feature = "test-util"))]
    host_events_seen: Vec<TerminalHostEvent>,
```

Initialize it to `Vec::new()` in **every** `TerminalTab` constructor / struct literal (search `lib.rs` for `TerminalTab {` — the real `open()` path and `open_with_engine_for_test` both build one; the compiler will name any you miss):

```rust
            #[cfg(any(test, feature = "test-util"))]
            host_events_seen: Vec::new(),
```

Record at the top of `on_host_event`, before any existing logic:

```rust
    pub fn on_host_event(&mut self, event: TerminalHostEvent, cx: &mut Context<Self>) {
        #[cfg(any(test, feature = "test-util"))]
        self.host_events_seen.push(event.clone());
        // ... existing body unchanged ...
```

Add the accessor in the same `impl TerminalTab` block:

```rust
    /// Every [`TerminalHostEvent`] delivered to this tab, in arrival order.
    /// Test-only observability: sub-slice D asserts fleet-level forwarding
    /// fidelity, which has no other observable effect on a tab that has not
    /// yet bound a `terminal_id`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn host_events_for_test(&self) -> &[TerminalHostEvent] {
        &self.host_events_seen
    }
```

`TerminalHostEvent` already derives `Clone` + `Debug` (`lib.rs:378-380`), so no derive changes are needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-terminal --lib -- --test-threads=4`
Expected: PASS — 217/217 (216 existing + 1 new).

- [ ] **Step 5: Enable the seam for `lens-ui` tests**

In `crates/lens-ui/Cargo.toml`, ensure the `[dev-dependencies]` entry for `lens-terminal` enables the feature. If `lens-terminal` is currently only a normal dependency, add a dev-dependency line alongside it:

```toml
[dev-dependencies]
lens-terminal = { path = "../lens-terminal", features = ["test-util"] }
```

Confirm `test-util` exists in `crates/lens-terminal/Cargo.toml` under `[features]`; if it does not, add `test-util = []`.

⚠️ Per memory `feature-unification-gate-trap`, a dev-dep feature can unify into the ordinary build. After this step run the **multi-crate** gate, not just `-p lens-ui`.

- [ ] **Step 6: Run the multi-crate gate**

Run: `cargo test -p lens-terminal -p lens-ui --lib -- --test-threads=4`
Then: `cargo clippy -p lens-terminal -p lens-ui --all-targets -- -D warnings`
Then: `cargo fmt --all -- --check`
Expected: all green, no `required-features` binaries dragged in.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/Cargo.toml crates/lens-ui/Cargo.toml
git commit -m "test(terminal-5-D): host-event recorder seam on TerminalTab (test-util)

Fleet-level forwarding has no observable effect on a tab with no bound
terminal_id, so D needs a recorder to assert forwarding fidelity.
Record-only; no behavior change."
```

---

### Task 2: `SessionControl` routing + resource-signal forwarding

Closes design §4.1 (routing) and §9 (forwarding). Today both control outcomes fall into the poller's `other =>` arm and are no-oped by `apply_outcome` (`poller.rs:145-148`).

**Files:**
- Modify: `crates/lens-ui/src/fleet/poller.rs:90-108` (match arms) and `:145-148` (comment)
- Modify: `crates/lens-ui/src/fleet/terminal.rs` (new `SessionControl` enum + `on_session_control` + forwarding)

**Interfaces:**
- Consumes: `TerminalTab::host_events_for_test` (Task 1).
- Produces:
  - `pub(crate) enum SessionControl { Superseded { target: SessionId, reason: String }, TerminalResource(TerminalResourceSignal) }`
  - `pub(crate) fn FleetStore::on_session_control(&mut self, session_id: &SessionId, signal: SessionControl, cx: &mut Context<Self>)`
  - Task 5 replaces this method's `Superseded` arm body.

**Design note for the reviewer:** design §4.1 names a single store method `on_session_control(id, signal)`. We keep that name and introduce the typed `SessionControl` enum so the store never has to match `ActorOutcome` variants it does not own, and so `target_conversation_id: String → SessionId` conversion happens once, at the boundary.

- [ ] **Step 1: Write the failing tests**

Add to `#[cfg(test)] mod tests` in `crates/lens-ui/src/fleet/terminal.rs`. Reuse the existing helpers in that module (`spawn_tab_with_rows`, `test_key`, `test_target`, `ManualUiClock`) and follow how existing tests construct the store:

```rust
#[gpui::test]
async fn resource_signal_forwards_to_owned_terminals_only(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(1_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));

    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");
    let key_a = test_key("main", "sk_a");
    let key_b = test_key("main", "sk_b");

    let (_e1, tab_a) = spawn_tab_with_rows(cx, 0);
    let (_e2, tab_b) = spawn_tab_with_rows(cx, 0);
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.insert_terminal_for_test(sess_a.clone(), key_a.clone(), tab_a.clone(), cx);
            store.insert_terminal_for_test(sess_b.clone(), key_b.clone(), tab_b.clone(), cx);
        });
    });

    let signal = TerminalResourceSignal::Deleted {
        terminal_id: TerminalId::new("term_1"),
    };
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.on_session_control(&sess_a, SessionControl::TerminalResource(signal), cx);
        });
    });

    cx.update(|cx| {
        let a_events = tab_a.read(cx).host_events_for_test().to_vec();
        assert_eq!(a_events.len(), 1, "owned terminal got exactly one host event");
        assert!(
            matches!(a_events[0], TerminalHostEvent::ResourceDeleted { .. }),
            "owned terminal got ResourceDeleted, got {:?}",
            a_events[0]
        );
        assert!(
            tab_b.read(cx).host_events_for_test().is_empty(),
            "a terminal owned by a DIFFERENT session must not be forwarded to"
        );
    });
}

#[gpui::test]
async fn resource_created_forwards_full_identity(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(1_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let sess = SessionId::new("conv_a");
    let key = test_key("main", "sk_a");
    let (_e, tab) = spawn_tab_with_rows(cx, 0);
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.insert_terminal_for_test(sess.clone(), key.clone(), tab.clone(), cx);
        });
    });

    let signal = TerminalResourceSignal::Created {
        terminal_id: TerminalId::new("term_9"),
        terminal_name: "main".into(),
        session_key: "sk_a".into(),
        session_id: sess.clone(),
    };
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.on_session_control(&sess, SessionControl::TerminalResource(signal), cx);
        });
    });

    cx.update(|cx| {
        let events = tab.read(cx).host_events_for_test().to_vec();
        assert_eq!(events.len(), 1);
        match &events[0] {
            TerminalHostEvent::ResourceCreated {
                session_id,
                terminal_id,
                terminal_name,
                session_key,
            } => {
                assert_eq!(session_id, &sess);
                assert_eq!(terminal_id.as_str(), "term_9");
                assert_eq!(terminal_name, "main");
                assert_eq!(session_key, "sk_a");
            }
            other => panic!("expected ResourceCreated, got {other:?}"),
        }
    });
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib resource_signal_forwards -- --test-threads=4`
Expected: FAIL to compile — `SessionControl` not found, `on_session_control` not found.

- [ ] **Step 3: Add `SessionControl` + `on_session_control` + forwarding**

In `crates/lens-ui/src/fleet/terminal.rs`, add near the `MemoryPressure` enum (`:16-21`):

```rust
/// Session-level control signals `FleetStore` owns (design §4.1). The poller
/// converts the two `ActorOutcome` control variants into this typed form so the
/// store never matches outcomes it does not own, and so the
/// `target_conversation_id: String -> SessionId` conversion happens once.
pub(crate) enum SessionControl {
    Superseded { target: SessionId, reason: String },
    TerminalResource(TerminalResourceSignal),
}
```

Add to the `impl FleetStore` block in the same file:

```rust
    /// Entry point for session-level control outcomes routed by the poller.
    pub(crate) fn on_session_control(
        &mut self,
        session_id: &SessionId,
        signal: SessionControl,
        cx: &mut Context<Self>,
    ) {
        match signal {
            SessionControl::TerminalResource(signal) => {
                self.forward_terminal_resource(session_id, signal, cx);
            }
            SessionControl::Superseded { target, reason: _ } => {
                // Task 5 replaces this body with load-B + move + Transfer.
                let _ = target;
            }
        }
    }

    /// Fan a resource signal out to every terminal owned by `session_id`. The
    /// tab filters to its own identity (Slice-4 contract), so a broadcast to
    /// the session's terminals is correct and keeps the store free of
    /// terminal-identity logic.
    fn forward_terminal_resource(
        &mut self,
        session_id: &SessionId,
        signal: TerminalResourceSignal,
        cx: &mut Context<Self>,
    ) {
        let Some(inner) = self.terminals.get(session_id) else {
            return;
        };
        let event = match signal {
            TerminalResourceSignal::Created {
                terminal_id,
                terminal_name,
                session_key,
                session_id,
            } => TerminalHostEvent::ResourceCreated {
                session_id,
                terminal_id,
                terminal_name,
                session_key,
            },
            TerminalResourceSignal::Deleted { terminal_id } => {
                TerminalHostEvent::ResourceDeleted { terminal_id }
            }
        };
        let tabs: Vec<_> = inner.values().map(|m| m.tab.clone()).collect();
        for tab in tabs {
            tab.update(cx, |tab, cx| {
                tab.on_host_event(event.clone(), cx);
            });
        }
    }
```

Add `use lens_core::actor::TerminalResourceSignal;` to the file's imports (match the existing import grouping).

The `tabs: Vec<_>` collect is deliberate: it ends the borrow of `self.terminals` before the `tab.update` loop, which needs `cx`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib resource_ -- --test-threads=4`
Expected: PASS — both new tests green.

- [ ] **Step 5: Wire the poller routing arm**

In `crates/lens-ui/src/fleet/poller.rs`, inside the `for o in batch.drain(..)` match (currently `:90-108`), add two arms **before** the `other =>` arm:

```rust
                                    ActorOutcome::Superseded {
                                        target_conversation_id,
                                        reason,
                                    } => store.on_session_control(
                                        &session_id,
                                        crate::fleet::terminal::SessionControl::Superseded {
                                            target: SessionId::new(target_conversation_id),
                                            reason,
                                        },
                                        cx,
                                    ),
                                    ActorOutcome::TerminalResource(signal) => store
                                        .on_session_control(
                                            &session_id,
                                            crate::fleet::terminal::SessionControl::TerminalResource(
                                                signal,
                                            ),
                                            cx,
                                        ),
```

Update the now-dead arm in `apply_outcome` (`:145-148`) to record why it is unreachable rather than implying work is pending:

```rust
        // Routed by the poller to FleetStore::on_session_control (sub-slice D);
        // these never reach the card-bound outcome path. Arm kept for exhaustiveness.
        ActorOutcome::Superseded { .. } | ActorOutcome::TerminalResource(_) => {}
```

- [ ] **Step 6: Run the gate**

Run: `cargo test -p lens-ui --lib -- --test-threads=4`
Then: `cargo clippy -p lens-ui --all-targets -- -D warnings`
Then: `cargo fmt --all -- --check`
Expected: all green; existing poller/coalescing/decay and reconcile-epoch tests still pass (design §14 requires the card-bound path stay unperturbed).

- [ ] **Step 7: Commit**

```bash
git add crates/lens-ui/src/fleet/poller.rs crates/lens-ui/src/fleet/terminal.rs
git commit -m "feat(terminal-5-D): route control outcomes to on_session_control + forward resource signals

Poller converts ActorOutcome::{Superseded,TerminalResource} into a typed
SessionControl and calls the new FleetStore::on_session_control. Resource
signals fan out to every terminal owned by the signalling session; the tab
filters to its own identity (Slice-4 contract).

Superseded arm is a stub until Task 5. Resolved in planning: D does not
re-prove both 4404 race orders (lens-ui cannot synthesize a 4404; A's
fourohfour_first_then_delete_create_adopts binds the chain, live rider
proves real ordering), and the persisted-item map_item path (design 4.2)
is NOT needed since Transfer is driven off the Superseded outcome."
```

---

### Task 3: `SessionLoader` seam

Gives `FleetStore` a way to make a not-yet-tracked session reachable without retaining `Connection`/`Client`/`data_dir` itself.

**Files:**
- Create: `crates/lens-ui/src/fleet/loader.rs`
- Modify: `crates/lens-ui/src/fleet/store.rs` (field + setter + `mod` declaration site)
- Modify: `crates/lens-ui/src/fleet/mod.rs` (add `pub mod loader;` — match the existing module style)

**Interfaces:**
- Produces:
  - `pub trait SessionLoader { fn load(&self, session_id: SessionId, store: WeakEntity<FleetStore>, cx: &mut App) -> Task<Result<(), String>>; }`
  - `FleetStore::set_session_loader(&mut self, loader: Rc<dyn SessionLoader>)`
  - `pub(crate) struct FakeSessionLoader` (test fake) with `loaded(&self) -> Vec<SessionId>`
- Task 5 consumes all of these; Task 6 implements the trait in `lens-app`.

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `crates/lens-ui/src/fleet/loader.rs` (created in step 3):

```rust
#[gpui::test]
async fn fake_loader_records_and_spawns(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(crate::clock::ManualUiClock::new(1_000));
    let store = cx.update(|cx| FleetStore::new(clock, cx));
    let loader = Rc::new(FakeSessionLoader::new());
    cx.update(|cx| {
        store.update(cx, |store, _cx| {
            store.set_session_loader(loader.clone());
        });
    });

    let target = SessionId::new("conv_b");
    let task = cx.update(|cx| loader.load(target.clone(), store.downgrade(), cx));
    task.await.expect("fake loader succeeds");
    cx.run_until_parked();

    assert_eq!(loader.loaded(), vec![target.clone()], "loader recorded the request");
    cx.update(|cx| {
        assert!(
            store.read(cx).cards.contains_key(&target),
            "fake loader made session B reachable (card present)"
        );
    });
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib fake_loader_records_and_spawns -- --test-threads=4`
Expected: FAIL to compile — `crates/lens-ui/src/fleet/loader.rs` does not exist.

- [ ] **Step 3: Create the loader module**

Create `crates/lens-ui/src/fleet/loader.rs`:

```rust
//! The session-load seam (design §10 step 1).
//!
//! `FleetStore` must be able to make a brand-new session reachable when the
//! server supersedes A -> B, but it deliberately retains no `Connection`,
//! `Client`, or data dir. This trait is that seam: `lens-app` supplies the real
//! implementation (background `GET /v1/sessions/{id}` -> seed control store ->
//! `spawn_live_session`); tests supply a fake.

use crate::fleet::store::FleetStore;
use gpui::{App, Task, WeakEntity};
use lens_core::domain::ids::SessionId;

/// Makes a not-yet-tracked session reachable in a [`FleetStore`].
pub trait SessionLoader {
    /// Load `session_id` into `store`.
    ///
    /// Implementations **must not block the foreground**: the real path does a
    /// blocking HTTP GET and must run it on `cx.background_executor()` before
    /// returning to the foreground to spawn. The returned task resolves once
    /// the session is reachable (its card + poller exist) or with an error.
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>>;
}

#[cfg(test)]
pub(crate) struct FakeSessionLoader {
    loaded: std::cell::RefCell<Vec<SessionId>>,
    fail: bool,
}

#[cfg(test)]
impl FakeSessionLoader {
    pub(crate) fn new() -> Self {
        Self {
            loaded: std::cell::RefCell::new(Vec::new()),
            fail: false,
        }
    }

    /// A loader that always fails — proves the store does not re-parent
    /// terminals into a session it could not load.
    pub(crate) fn failing() -> Self {
        Self {
            loaded: std::cell::RefCell::new(Vec::new()),
            fail: true,
        }
    }

    pub(crate) fn loaded(&self) -> Vec<SessionId> {
        self.loaded.borrow().clone()
    }
}

#[cfg(test)]
impl SessionLoader for FakeSessionLoader {
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        self.loaded.borrow_mut().push(session_id.clone());
        if self.fail {
            return Task::ready(Err("fake loader: forced failure".into()));
        }
        // Make B reachable the same way the fake fleet does elsewhere.
        let result = store.update(cx, |store, cx| {
            store.spawn_fake_session(session_id, cx);
        });
        Task::ready(result.map(|_| ()).map_err(|e| format!("{e:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::store::FleetStore;
    use std::rc::Rc;
    use std::sync::Arc;

    // (test from Step 1 goes here)
}
```

Register the module — in `crates/lens-ui/src/fleet/mod.rs` add `pub mod loader;` next to the existing `pub mod` lines.

- [ ] **Step 4: Add the store field + setter**

In `crates/lens-ui/src/fleet/store.rs`, add to the `FleetStore` struct (`:59-77`):

```rust
    session_loader: Option<std::rc::Rc<dyn crate::fleet::loader::SessionLoader>>,
```

Initialize it to `None` in **every** `FleetStore` struct literal (`FleetStore::new` at `:80`, and any other constructor the compiler flags):

```rust
            session_loader: None,
```

Add the setter to `impl FleetStore`:

```rust
    /// Inject the session-load seam (design §10 step 1). `lens-app` wires the
    /// real loader at startup; tests inject a fake. Without a loader the
    /// supersede handler no-ops rather than stranding terminals.
    pub fn set_session_loader(
        &mut self,
        loader: std::rc::Rc<dyn crate::fleet::loader::SessionLoader>,
    ) {
        self.session_loader = Some(loader);
    }
```

`Rc` (not `Arc`) because `FleetStore` is a gpui entity living on one thread; the loader's own background work clones what it needs internally.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib fake_loader_records_and_spawns -- --test-threads=4`
Expected: PASS.

If `spawn_fake_session` panics with `expect("fake mode")`, the test store was not created in fake mode — check how `FleetStore::new` initializes `fake` (`store.rs:81-84` sets `fake: Some(FakeFleet::new())`, so the default is fake mode and this should work).

- [ ] **Step 6: Run the gate**

Run: `cargo test -p lens-ui --lib -- --test-threads=4`
Then: `cargo clippy -p lens-ui --all-targets -- -D warnings`
Then: `cargo fmt --all -- --check`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-ui/src/fleet/loader.rs crates/lens-ui/src/fleet/mod.rs crates/lens-ui/src/fleet/store.rs
git commit -m "feat(terminal-5-D): SessionLoader seam for headless load-B

FleetStore deliberately retains no Connection/Client/data_dir, and a
brand-new B must be GET+seeded before spawn_live_session can run
(scheduler.reconnect -> SessionNotFound). This trait is that seam: the real
impl lands in lens-app (Task 6); tests inject FakeSessionLoader. The trait
returns a Task so the real blocking GET runs off the foreground."
```

---

### Task 4: `move_terminal_members` — re-parent A→B with a rebound subscription

Design §10 step 2. This is the task that closes the **known trap** documented above.

**Files:**
- Modify: `crates/lens-ui/src/fleet/terminal.rs` (new method + tests)

**Interfaces:**
- Produces: `fn FleetStore::move_terminal_members(&mut self, from: &SessionId, to: &SessionId, cx: &mut Context<Self>) -> Vec<TerminalKeyId>` — returns the keys moved, in arbitrary order. Task 5 consumes it.

- [ ] **Step 1: Write the failing tests**

Add to `#[cfg(test)] mod tests` in `crates/lens-ui/src/fleet/terminal.rs`:

```rust
#[gpui::test]
async fn move_terminal_members_reparents_preserving_state(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(5_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");
    let key = test_key("main", "sk_a");
    let (_e, tab) = spawn_tab_with_rows(cx, 0);

    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.insert_terminal_for_test(sess_a.clone(), key.clone(), tab.clone(), cx);
            store.set_terminal_visible(&sess_a, &key, false, cx); // hidden = true
            store.set_member_pending_sleep_for_test(&sess_a, &key, true);
        });
    });

    let before = cx.update(|cx| {
        let s = store.read(cx);
        let m = s
            .terminal_member_for_test(&sess_a, &key, cx)
            .expect("member under A");
        (m.last_viewed, m.hidden, m.pending_sleep)
    });

    let moved = cx.update(|cx| {
        store.update(cx, |store, cx| store.move_terminal_members(&sess_a, &sess_b, cx))
    });
    assert_eq!(moved.len(), 1, "exactly one member moved");

    cx.update(|cx| {
        let s = store.read(cx);
        assert!(
            s.terminal_member_for_test(&sess_a, &key, cx).is_none(),
            "member no longer under A"
        );
        let m = s
            .terminal_member_for_test(&sess_b, &key, cx)
            .expect("member now under B");
        assert_eq!(
            (m.last_viewed, m.hidden, m.pending_sleep),
            before,
            "last_viewed/hidden/pending_sleep preserved across the move"
        );
    });
}

// THE TRAP: the subscription closure captures the owning SessionId. If the move
// does not rebuild it, PresentationChanged still calls back with session A,
// where the key no longer exists -> deferred pending_sleep silently dies.
#[gpui::test]
async fn moved_member_subscription_is_rebound_to_new_session(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(5_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");
    let key = test_key("main", "sk_a");
    let (_e, tab) = spawn_tab_with_rows(cx, 0);

    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.insert_terminal_for_test(sess_a.clone(), key.clone(), tab.clone(), cx);
            store.set_member_pending_sleep_for_test(&sess_a, &key, true);
            store.move_terminal_members(&sess_a, &sess_b, cx);
        });
    });

    // Fire the subscription. If it is still bound to A, on_terminal_presentation_changed
    // early-returns and pending_sleep stays set forever.
    cx.update(|cx| {
        tab.update(cx, |_tab, cx| {
            cx.emit(TerminalEvent::PresentationChanged);
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let s = store.read(cx);
        let m = s
            .terminal_member_for_test(&sess_b, &key, cx)
            .expect("member under B");
        assert!(
            !m.pending_sleep,
            "subscription must be rebound to B: deferred sleep should have been \
             applied and pending_sleep cleared"
        );
    });
}
```

If the tab's lifecycle from `open_with_engine_for_test` is not sleepable, `on_terminal_presentation_changed` will not clear `pending_sleep` and this test would false-pass its own premise. Verify by checking `is_sleepable` against the lifecycle `open_with_engine_for_test` produces; if it is **not** sleepable, assert the rebinding differently — assert that the callback reached B at all by checking `terminals[A]` is absent and adding a counter, rather than relying on the sleep side effect. **Confirm this before trusting the test** (memory `false-green-probe-drives-production-path`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib move_terminal_members -- --test-threads=4`
Expected: FAIL to compile — `no method named move_terminal_members`.

- [ ] **Step 3: Implement the move**

Add to `impl FleetStore` in `crates/lens-ui/src/fleet/terminal.rs`:

```rust
    /// Re-parent every terminal owned by `from` to `to` (design §10 step 2).
    ///
    /// The inner `TerminalKeyId` is deliberately **not** rekeyed: a retained
    /// `TerminalId` implies a retained `(terminal_name, session_key)`, and keys
    /// are session-scoped so they cannot collide with B's own terminals.
    ///
    /// Each member's `TerminalEvent` subscription captures its owning
    /// `SessionId`, so the move must build a **fresh** subscription bound to
    /// `to`; otherwise the callback keeps looking the member up under `from`,
    /// where it no longer exists, and deferred `pending_sleep` dies silently.
    fn move_terminal_members(
        &mut self,
        from: &SessionId,
        to: &SessionId,
        cx: &mut Context<Self>,
    ) -> Vec<TerminalKeyId> {
        let Some(inner) = self.terminals.remove(from) else {
            return Vec::new();
        };
        let mut moved = Vec::with_capacity(inner.len());
        for (key_id, member) in inner {
            let key = terminal_key_from_id(&key_id);
            let session_for_sub = to.clone();
            let key_for_sub = key.clone();
            let tab = member.tab.clone();
            let sub = cx.subscribe(&tab, move |store, _tab, event, cx| {
                if matches!(event, TerminalEvent::PresentationChanged) {
                    store.on_terminal_presentation_changed(&session_for_sub, &key_for_sub, cx);
                }
            });
            let rebound = TerminalMember {
                tab,
                last_viewed: member.last_viewed,
                hidden: member.hidden,
                pending_sleep: member.pending_sleep,
                _sub: sub,
            };
            // Dropping `member` here drops its old subscription (bound to `from`).
            drop(member);
            let previous = self
                .terminals
                .entry(to.clone())
                .or_default()
                .insert(key_id.clone(), rebound);
            // Defensive: keys are session-scoped so a collision should be
            // impossible, but never leave a tab with a live engine outside
            // fleet accounting.
            if let Some(previous) = previous {
                end_member_tab(&previous, cx);
            }
            moved.push(key_id);
        }
        moved
    }
```

Note `let tab = member.tab.clone();` before building the subscription, then `drop(member)` — this makes the old subscription's death explicit rather than incidental.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib move_terminal_members -- --test-threads=4`
Then: `cargo test -p lens-ui --lib moved_member_subscription -- --test-threads=4`
Expected: PASS both.

- [ ] **Step 5: Run the gate**

Run: `cargo test -p lens-ui --lib -- --test-threads=4`
Then: `cargo clippy -p lens-ui --all-targets -- -D warnings`
Then: `cargo fmt --all -- --check`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/fleet/terminal.rs
git commit -m "feat(terminal-5-D): move_terminal_members re-parents A->B with a rebound subscription

The TerminalEvent subscription captures its owning SessionId at insert time
and on_terminal_presentation_changed early-returns when the key is absent, so
a naive map move would silently kill deferred pending_sleep for the
transferred terminal. The move rebuilds the subscription against B and
preserves last_viewed/hidden/pending_sleep. Inner key is NOT rekeyed
(design 10 step 2)."
```

---

### Task 5: `Superseded` → load B → move → `Transfer`

Completes design §10 at the store layer.

**Files:**
- Modify: `crates/lens-ui/src/fleet/terminal.rs` (`on_session_control` Superseded arm + `on_supersede`/`complete_supersede` + tests)

**Interfaces:**
- Consumes: `SessionLoader` + `FakeSessionLoader` (Task 3), `move_terminal_members` (Task 4), `TerminalHostEvent::Transfer { new_session }` (sub-slice A, `lens-terminal/src/lib.rs:670`).

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test]
async fn supersede_loads_b_moves_member_and_drives_transfer(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(5_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let loader = Rc::new(FakeSessionLoader::new());
    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");
    let key = test_key("main", "sk_a");
    let (_e, tab) = spawn_tab_with_rows(cx, 0);

    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.set_session_loader(loader.clone());
            store.insert_terminal_for_test(sess_a.clone(), key.clone(), tab.clone(), cx);
            store.on_session_control(
                &sess_a,
                SessionControl::Superseded {
                    target: sess_b.clone(),
                    reason: "clear".into(),
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    assert_eq!(loader.loaded(), vec![sess_b.clone()], "B was loaded");
    cx.update(|cx| {
        let s = store.read(cx);
        assert!(
            s.terminal_member_for_test(&sess_a, &key, cx).is_none(),
            "member left A"
        );
        assert!(
            s.terminal_member_for_test(&sess_b, &key, cx).is_some(),
            "member re-parented to B under the SAME key (no rekey)"
        );
    });
    cx.update(|cx| {
        let events = tab.read(cx).host_events_for_test().to_vec();
        let transfer = events
            .iter()
            .find(|e| matches!(e, TerminalHostEvent::Transfer { .. }))
            .expect("Transfer was driven");
        match transfer {
            TerminalHostEvent::Transfer { new_session } => {
                assert_eq!(new_session, &sess_b, "Transfer retargets to B");
            }
            other => panic!("expected Transfer, got {other:?}"),
        }
    });
}

#[gpui::test]
async fn supersede_does_not_reparent_when_load_fails(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(5_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let loader = Rc::new(FakeSessionLoader::failing());
    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");
    let key = test_key("main", "sk_a");
    let (_e, tab) = spawn_tab_with_rows(cx, 0);

    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.set_session_loader(loader.clone());
            store.insert_terminal_for_test(sess_a.clone(), key.clone(), tab.clone(), cx);
            store.on_session_control(
                &sess_a,
                SessionControl::Superseded {
                    target: sess_b.clone(),
                    reason: "clear".into(),
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let s = store.read(cx);
        assert!(
            s.terminal_member_for_test(&sess_a, &key, cx).is_some(),
            "load failed -> member stays under A, never orphaned into an unloaded B"
        );
        assert!(s.terminal_member_for_test(&sess_b, &key, cx).is_none());
    });
    cx.update(|cx| {
        assert!(
            !tab.read(cx)
                .host_events_for_test()
                .iter()
                .any(|e| matches!(e, TerminalHostEvent::Transfer { .. })),
            "no Transfer when B could not be loaded"
        );
    });
}

#[gpui::test]
async fn supersede_without_owned_terminals_does_not_load(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(5_000));
    let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
    let loader = Rc::new(FakeSessionLoader::new());
    let sess_a = SessionId::new("conv_a");
    let sess_b = SessionId::new("conv_b");

    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.set_session_loader(loader.clone());
            store.on_session_control(
                &sess_a,
                SessionControl::Superseded {
                    target: sess_b.clone(),
                    reason: "clear".into(),
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    assert!(
        loader.loaded().is_empty(),
        "no owned terminals -> nothing to follow -> do not load B \
         (Slice 5 makes B reachable only for the terminal follow; view \
          auto-follow is Slice 6)"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui --lib supersede_ -- --test-threads=4`
Expected: FAIL — `supersede_loads_b_...` fails (the Task-2 stub no-ops, so nothing moves and no Transfer is recorded).

- [ ] **Step 3: Implement the supersede handler**

Replace the `SessionControl::Superseded` arm in `on_session_control`:

```rust
            SessionControl::Superseded { target, reason: _ } => {
                self.on_supersede(session_id, target, cx);
            }
```

Add to `impl FleetStore`:

```rust
    /// `/clear` rotated session A to a brand-new session B and the server
    /// transferred the terminal live (same `TerminalId`). To keep the terminal
    /// we must load B, re-parent the member, and drive a retain-engine
    /// `Transfer` so scrollback survives (design §10).
    fn on_supersede(&mut self, from: &SessionId, to: SessionId, cx: &mut Context<Self>) {
        // Nothing to follow. Slice 5 makes B reachable only for the terminal
        // follow; the view auto-follow to B is Slice 6.
        if self.terminals.get(from).is_none_or(|m| m.is_empty()) {
            return;
        }
        // Already tracked (duplicate/replayed signal) — skip straight to the move.
        if self.cards.contains_key(&to) {
            self.complete_supersede(from, &to, cx);
            return;
        }
        let Some(loader) = self.session_loader.clone() else {
            return;
        };
        let task = loader.load(to.clone(), cx.entity().downgrade(), &mut *cx);
        let from = from.clone();
        cx.spawn(async move |store, cx| {
            if task.await.is_err() {
                // Leave the member under A rather than orphaning it into a
                // session that does not exist. A's replacement timeout still
                // bounds the tab's wait.
                return;
            }
            let _ = store.update(cx, |store, cx| {
                store.complete_supersede(&from, &to, cx);
            });
        })
        .detach();
    }

    /// Re-parent A's terminals to B and retarget each tab to B.
    fn complete_supersede(&mut self, from: &SessionId, to: &SessionId, cx: &mut Context<Self>) {
        let moved = self.move_terminal_members(from, to, cx);
        if moved.is_empty() {
            return;
        }
        let tabs: Vec<_> = self
            .terminals
            .get(to)
            .into_iter()
            .flat_map(|inner| moved.iter().filter_map(|k| inner.get(k)))
            .map(|m| m.tab.clone())
            .collect();
        for tab in tabs {
            tab.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::Transfer {
                        new_session: to.clone(),
                    },
                    cx,
                );
            });
        }
        cx.notify();
    }
```

Add `use std::rc::Rc;` and the `FakeSessionLoader`/`SessionLoader` imports to the test module as needed.

If clippy rejects `is_none_or` on the installed toolchain, use `self.terminals.get(from).map_or(true, |m| m.is_empty())`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui --lib supersede_ -- --test-threads=4`
Expected: PASS — all three.

- [ ] **Step 5: Run the gate**

Run: `cargo test -p lens-ui --lib -- --test-threads=4`
Then: `cargo clippy -p lens-ui --all-targets -- -D warnings`
Then: `cargo fmt --all -- --check`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/fleet/terminal.rs
git commit -m "feat(terminal-5-D): session.superseded -> load B, re-parent, drive Transfer

On ActorOutcome::Superseded the store loads B through the SessionLoader
seam, moves A's terminal members to B under the same key, and drives
TerminalHostEvent::Transfer { new_session: B } so A's retained frozen engine
is reused and scrollback survives /clear.

Guards: no owned terminals -> no load; B already tracked -> skip load;
load failure -> member stays under A (never orphaned into an unloaded
session), A's replacement timeout still bounds the wait."
```

---

### Task 6: Real `AppSessionLoader` in `lens-app`

**Files:**
- Create: `crates/lens-app/src/loader.rs`
- Modify: `crates/lens-app/src/main.rs` (declare `mod loader;`, make `seed_disk` reachable, wire the loader after the store is built at ~`:104`)

**Interfaces:**
- Consumes: `SessionLoader` (Task 3), `FleetStore::spawn_live_session` (`store.rs:353`), `seed_disk` (`main.rs:632`), `Sessions::get` (`lens-client/src/sessions.rs:1281`).

- [ ] **Step 1: Make `seed_disk` reachable from the loader module**

In `crates/lens-app/src/main.rs`, change `fn seed_disk(` (`:632`) to `pub(crate) fn seed_disk(`. Do the same for `seed_connection`/`seed_session` only if the compiler requires it.

- [ ] **Step 2: Write the loader**

Create `crates/lens-app/src/loader.rs`:

```rust
//! The real [`SessionLoader`]: makes a brand-new session reachable mid-stream.
//!
//! `session.superseded` hands us a conversation id we have never seen. Before
//! `FleetStore::spawn_live_session` can run, that session must exist in the
//! control store — `scheduler.reconnect` does `load_session(..).ok_or(
//! SessionNotFound)`. So we GET the snapshot and seed it first. The GET is
//! blocking, so it runs on the background executor; only the spawn returns to
//! the foreground.

use gpui::{App, AsyncApp, Task, WeakEntity};
use lens_client::{Client, Connection, GetOpts};
use lens_core::domain::ids::SessionId;
use lens_ui::fleet::loader::SessionLoader;
use lens_ui::fleet::store::FleetStore;
use std::path::PathBuf;

pub(crate) struct AppSessionLoader {
    conn: Connection,
    data_dir: PathBuf,
}

impl AppSessionLoader {
    pub(crate) fn new(conn: Connection, data_dir: PathBuf) -> Self {
        Self { conn, data_dir }
    }
}

impl SessionLoader for AppSessionLoader {
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let conn = self.conn.clone();
        let data_dir = self.data_dir.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            // Blocking GET + control-store seed, off the foreground.
            let seeded = {
                let conn = conn.clone();
                let data_dir = data_dir.clone();
                let session_id = session_id.clone();
                cx.background_executor()
                    .spawn(async move {
                        let client = Client::new(conn.clone())
                            .map_err(|e| format!("client handshake: {e}"))?;
                        let snap = client
                            .sessions()
                            .get(&session_id, GetOpts::default())
                            .map_err(|e| format!("get session {session_id}: {e}"))?;
                        crate::seed_disk(&conn, &session_id, &data_dir, &snap)
                    })
                    .await
            };
            seeded?;

            // Foreground: build the live session (poller + card + bridge).
            let client =
                Client::new(conn.clone()).map_err(|e| format!("client handshake: {e}"))?;
            store
                .update(cx, |store, cx| {
                    store
                        .spawn_live_session(&conn, &client, session_id.clone(), &data_dir, cx)
                        .map(|_card| ())
                })
                .map_err(|e| format!("store gone: {e:?}"))?
        })
    }
}
```

`Client` is rebuilt inside the background task rather than captured, mirroring `spawn_live_session`'s own `Client::new(conn.clone())` (`store.rs:383-385`) — this avoids requiring `Client: Send`.

Adjust imports to the crate's actual paths: check how `main.rs` imports `Connection`, `Client`, `GetOpts`, and whether `lens_ui::fleet::loader` / `lens_ui::fleet::store` are publicly reachable. If `fleet::loader` is not `pub`, re-export it from `crates/lens-ui/src/lib.rs` (the file already re-exports fleet items at `:17`).

- [ ] **Step 3: Wire it in `main.rs`**

Add `mod loader;` near the other module declarations. After the fleet store is created and before/around the `spawn_live_session` loop at `:104`, inject the loader:

```rust
                            fleet.update(cx, |fleet, _cx| {
                                fleet.set_session_loader(std::rc::Rc::new(
                                    crate::loader::AppSessionLoader::new(
                                        conn.clone(),
                                        config.data_dir.clone(),
                                    ),
                                ));
                            });
```

Place it so it runs once per store, before any session is spawned. Match the surrounding closure's variable names (`fleet`, `conn`, `config`) — read `:95-115` and adapt.

- [ ] **Step 4: Verify it compiles and the workspace gate passes**

Run: `cargo build -p lens-app`
Expected: compiles clean.

Run: `cargo run -p xtask -- gate`
Expected: workspace clippy `-D warnings` + fmt + tests all green. Do **not** pipe through `tail` (memory `xtask-gate-scope`). Expect a cold-cache run of several minutes (memory `gate-cost-is-cold-cache`).

There is no headless unit test for `AppSessionLoader` — it needs a live server. Its proof is the Task 7 live rider. Do not fabricate a passing test for it.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-app/src/loader.rs crates/lens-app/src/main.rs crates/lens-ui/src/lib.rs
git commit -m "feat(terminal-5-D): real AppSessionLoader (GET snapshot -> seed -> spawn_live_session)

Blocking GET /v1/sessions/{id} + control-store seed run on the background
executor (scheduler.reconnect needs B on disk or it returns SessionNotFound);
only spawn_live_session returns to the foreground. Wired into main.rs.

No headless test — needs a live server; proven by the supersede live rider."
```

---

### Task 7: Live riders + whole-branch review

Design §13 gates D on live riders and a whole-branch review of the actor-outcome × terminal-event cross seam.

**Files:**
- Create/extend the opt-in live-rider harness (follow the existing P7/P8 rider pattern; see memory `omnigent-terminal-attach-live-run` for the ephemeral rider-shell bundle and `installing-omnigent-from-source` for a matching 0.5.1 server)

- [ ] **Step 1: Start a live omnigent 0.5.1**

Use the `installing-omnigent-from-source` skill. Confirm `omnigent --version` reports 0.5.1.

- [ ] **Step 2: Rider — supersede scrollback survives `/clear`**

Open a terminal in a session, write recognizable output, `/clear` the conversation, then assert the terminal is still live under B **and** the pre-`/clear` output is still scrollable. This is the only real proof of the retain-engine `Transfer` (design §10; A's cross-session success path was explicitly deferred to this rider).

- [ ] **Step 3: Rider — `4404`-first real ordering**

Force the `4404` ↔ `resource.deleted` race and confirm the tab adopts regardless of order. A converges both orders on `ReplacementWaiting`; this rider proves the real interleaving.

- [ ] **Step 4: Rider — transfer `output_gap` visual**

A's whole-slice review flagged that `on_reconnect_success` sets `presentation.output_gap = true` on the Transfer reuse path, which may be spurious when server B replays clear+redraw. Confirm visually and either keep or suppress it.

- [ ] **Step 5: Record rider results**

Write findings to `docs/handoffs/` and update `.superpowers/sdd/progress.md`. If a rider fails, fix on-branch and re-run — do not merge on a red rider.

- [ ] **Step 6: Whole-branch review of D**

Dispatch a fresh cross-family reviewer (Opus subagent, or `grok-4.5` via cursor-delegate — codex quota is exhausted this week). **The reviewer must build and run the gate** (memory `whole-branch-review-needs-a-builder`). Review range: D's first commit through HEAD. Reviewer checklist:
  1. The card-bound outcome path and focused-replica routing are unperturbed (design §14) — coalescing/decay + reconcile-epoch tests still green.
  2. No re-parent into an unloaded session on any loader failure/cancel path.
  3. The moved member's subscription is bound to B on **every** exit, and `pending_sleep`/`hidden`/`last_viewed` survive.
  4. No path bypasses `on_host_event` to reach engine internals (A's frozen seam intact).
  5. The recorder seam is `test-util`-gated and cannot inflate a production build.
  6. No unbounded growth in `host_events_seen` in any long-lived non-test build.

- [ ] **Step 7: Final commit**

```bash
git add docs/ .superpowers/sdd/progress.md
git commit -m "docs(terminal-5-D): live-rider results + whole-branch review outcome"
```

---

## Self-review

**Spec coverage (design §3 sub-slice D row):**
- "resource-signal forwarding to owned terminals" → Task 2.
- "`4404`-first reconciliation driving" → Task 2 (forwarding is the driving); the adoption proof is A's e2e + Task 7 rider. Documented as resolved planning question 4.
- "supersede: load B headlessly + move member A→B + drive `Transfer`" → Tasks 3+4+5 (store layer), Task 6 (real loader), Task 7 (rider).
- "injected-outcome tests + live riders; whole-branch review" → Tasks 2/5 tests, Task 7.
- §4.1 `on_session_control` → Task 2. §4.2 `map_item` → resolved as NOT needed.

**Type consistency:** `SessionControl` (Task 2) is consumed unchanged in Task 5. `move_terminal_members -> Vec<TerminalKeyId>` (Task 4) is consumed by `complete_supersede` (Task 5). `SessionLoader::load` (Task 3) is implemented in Task 6 with the identical signature. `host_events_for_test` (Task 1) is used in Tasks 2 and 5.

**Known soft spots the executor must confirm, not assume:**
- Task 4's rebinding test depends on the lifecycle `open_with_engine_for_test` yields being sleepable. Verify before trusting it (flagged inline).
- Task 6's import paths and the `main.rs` injection site are described from a read of the surrounding code; adapt to what is actually there.
- `Rc<dyn SessionLoader>` assumes `FleetStore` stays single-threaded. It does today (gpui entity).
