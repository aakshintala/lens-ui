# Terminal Slice 5 — Sub-slice A (lens-terminal lifecycle) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Every task is TDD: failing test first, watch it fail, minimal implementation, watch it pass, commit.

**Goal:** Make `lens-terminal` retain a frozen engine across a positively-identified reset and re-attach it to either the same session (agent-switch, fresh) or a new session (cross-session supersede, reuse), plus close the 4404-first gap and fix the scrollback-cap byte bug.

**Architecture:** Slice 4 already enters `ReplacementWaiting` on a reset-delete and adopts an exact-key successor with a *fresh* engine. A converts `ReplacementWaiting` to **retain the frozen engine** (transport-only teardown) and routes all adoption through a single **`adopt(session_id, terminal_id)`** that reuses the retained engine iff the session changed (reconnect-transport shape) or drops it and re-discovers if the session is unchanged. A host-driven **`Transfer { new_session }`** drives the cross-session reuse path; a **4404 on an `OpenOrCreate` target** now enters `ReplacementWaiting` instead of hard-detaching.

**Tech Stack:** Rust, gpui 0.2.2 entities/spawns, libghostty-vt engine, `#[gpui::test]` async tests, the existing reconnect/attach worker (`preflight_reconnect` + `attach` + `on_reconnect_success`).

## Global Constraints

- **Author:** composer-2.5 under **Opus supervision** on the frozen `ReplacementWaiting`/adopt/bridge seam (Slice-4 whole-branch-review territory).
- **TDD mandatory** — no production line without a failing test first. Bug-class from today's review: an unguarded spawn resurrecting a torn-down tab. **Every attach/adopt spawn re-checks `reconnect_epoch` (and lifecycle) at APPLY time**, closing freshly-built parts on mismatch (`close_parts_off_foreground` / `close_attach_off_foreground`).
- **Per-crate gate before each commit:** `cargo test -p lens-terminal --lib` + `cargo clippy -p lens-terminal --all-targets -- -D warnings` + `cargo fmt --all -- --check`. Do NOT run real-window harnesses (they need a real GPUI window; run headless they hang).
- **No independent merge** — A stays on `terminal-slice-5-fleetstore`; the whole slice-5 (A+B+C+D) merges together after D + a final whole-branch Opus review + live riders.
- **Reviews are Opus** (a fresh Opus subagent that builds+tests), not codex — codex quota is exhausted this week.
- Scrollback budget default = **`10_000_000` bytes** (verbatim). `max_scrollback` is a **BYTE** budget, not lines.
- `REPLACEMENT_WAIT = 30s` (unchanged). Adoption is **exact-key** only.

---

### Task 1: Scrollback-cap byte fix (§11)

Isolated, mechanical, no dependency on the lifecycle work. Fixes a latent prod bug: `TerminalOpenOptions::scrollback_lines` feeds `EngineConfig::max_scrollback`, which is a **byte** budget (memory `terminal-max-scrollback-bytes-and-worker-stack`), and defaults to `1000` bytes (~7 rows).

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` (`TerminalOpenOptions` at ~217–246: field, `Default`, builder)
- Modify: `crates/lens-terminal/src/policy.rs:274` (the `.unwrap_or(1000)` consumer)
- Test: `crates/lens-terminal/src/lib.rs` (tests module; there is an existing assertion at ~3622 `assert_eq!(o.scrollback_lines, None)` to update)

**Interfaces:**
- Produces: `TerminalOpenOptions { pub scrollback_bytes: Option<usize> }`, `TerminalOpenOptions::with_scrollback_bytes(self, bytes: Option<usize>) -> Self`, `#[non_exhaustive]` on the struct. Default `scrollback_bytes: None`; the effective default byte budget is `10_000_000` applied at `policy.rs`.

- [ ] **Step 1: Write the failing test** — add to the lib.rs tests module:

```rust
#[test]
fn scrollback_bytes_default_is_ten_million_not_seven_rows() {
    // With no override, the engine byte budget must be a real default, not 1000 bytes.
    let opts = TerminalOpenOptions::default();
    assert_eq!(opts.scrollback_bytes, None);
    let cfg = super::policy::engine_config_for_test(&opts); // added in Step 3b
    assert_eq!(cfg.max_scrollback, 10_000_000);
}

#[test]
fn with_scrollback_bytes_overrides_the_default() {
    let opts = TerminalOpenOptions::default().with_scrollback_bytes(Some(2_048));
    let cfg = super::policy::engine_config_for_test(&opts);
    assert_eq!(cfg.max_scrollback, 2_048);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-terminal --lib scrollback_bytes`
Expected: FAIL to compile — `scrollback_bytes`, `with_scrollback_bytes`, `engine_config_for_test` do not exist.

- [ ] **Step 3a: Rename the option field + builder + doc.** In `lib.rs`, change the `scrollback_lines` field to `scrollback_bytes`, add `#[non_exhaustive]` to `TerminalOpenOptions`, rename `with_scrollback_lines` → `with_scrollback_bytes`, and update the doc comment to say the value is a **byte** budget (default `10_000_000`).

```rust
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalOpenOptions {
    // ...existing fields...
    /// Scrollback retention as a **byte** budget for the engine (`max_scrollback`),
    /// NOT a line count. `None` → the default `10_000_000` bytes (~12.5k rows @200 cols;
    /// validated safe on the 64 MiB worker stack, memory
    /// `terminal-max-scrollback-bytes-and-worker-stack`).
    pub scrollback_bytes: Option<usize>,
}
```

Update `Default` (`scrollback_bytes: None`) and the builder:

```rust
pub fn with_scrollback_bytes(mut self, bytes: Option<usize>) -> Self {
    self.scrollback_bytes = bytes;
    self
}
```

- [ ] **Step 3b: Apply the default at the consumer.** In `policy.rs:274`, change the map to the new name + default, and add a small `#[cfg(any(test, feature = "test-util"))]` helper so the test can read the resolved config:

```rust
// policy.rs — where EngineConfig is built from options:
max_scrollback: options.scrollback_bytes.unwrap_or(10_000_000),
```

```rust
// policy.rs — test seam near the config builder:
#[cfg(any(test, feature = "test-util"))]
pub(crate) fn engine_config_for_test(options: &TerminalOpenOptions) -> EngineConfig {
    engine_config(options) // call whatever the existing private builder is named
}
```

If a private `engine_config(options)` builder does not already exist, factor the inline construction at `policy.rs:274` into one and call it from both the production site and `engine_config_for_test`.

- [ ] **Step 3c: Update the stale existing assertion** at lib.rs ~3622 from `assert_eq!(o.scrollback_lines, None)` to `assert_eq!(o.scrollback_bytes, None)`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lens-terminal --lib scrollback` then `cargo test -p lens-terminal --lib`
Expected: PASS, 208+ tests green.

- [ ] **Step 5: Gate + commit**

```bash
cargo clippy -p lens-terminal --all-targets -- -D warnings && cargo fmt --all -- --check
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/src/policy.rs
git commit -m "fix(terminal-5-A): scrollback_lines -> scrollback_bytes, default 10MB (§11 byte-budget bug)"
```

---

### Task 2: 4404-first — `OpenOrCreate` 4404 enters `ReplacementWaiting`

Close the Slice-4 gap: a 4404/`TerminalGone` while `Live` currently always hard-detaches. For an `OpenOrCreate` target it should enter `ReplacementWaiting` (there may be a successor to adopt); `Existing` stays hard-detach [Q4].

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — `apply_bridge_event`, the `PolicyAction::StopDetached` arm (~2220–2234)
- Test: `crates/lens-terminal/src/lib.rs` tests module

**Interfaces:**
- Consumes: `self.target: TerminalTarget`, `enter_replacement_waiting(&mut self, cx)`, `on_detach(&mut self, detail, cx)`, `DetachedDetail::TerminalGone`.
- Produces: no new public surface; behavioral change only.

- [ ] **Step 1: Write the failing tests** (two — OpenOrCreate goes to ReplacementWaiting, Existing stays Detached). Model on `open_or_create_delete_enters_replacement_waiting` (~2980) which uses `live_tab_for_test`. Drive the 4404 by delivering a `BridgeEvent::Closed(CloseCause::TerminalNotFound)` via the existing bridge-event test entry (mirror how other `apply_bridge_event` tests inject — e.g. the late-4404 test around 2938 that asserts a ReplacementWaiting tab ignores a stale close).

```rust
#[gpui::test]
async fn openorcreate_4404_while_live_enters_replacement_waiting(cx: &mut gpui::TestAppContext) {
    let (_engine, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        assert_eq!(tab.lifecycle, Lifecycle::Live);
        tab.apply_bridge_event(BridgeEvent::Closed(lens_client::CloseCause::TerminalNotFound), cx);
        assert_eq!(
            tab.lifecycle,
            Lifecycle::ReplacementWaiting,
            "OpenOrCreate 4404 must wait for a successor, not hard-detach"
        );
    });
}

#[gpui::test]
async fn existing_4404_while_live_hard_detaches(cx: &mut gpui::TestAppContext) {
    let (_engine, tab) = live_tab_for_test(cx, false, "t1", "main", "k"); // Existing target
    tab.update(cx, |tab, cx| {
        assert_eq!(tab.lifecycle, Lifecycle::Live);
        tab.apply_bridge_event(BridgeEvent::Closed(lens_client::CloseCause::TerminalNotFound), cx);
        assert_eq!(
            tab.lifecycle,
            Lifecycle::Detached,
            "Existing 4404 has no successor semantics — stays hard-detach"
        );
        assert_eq!(tab.presentation.detached_detail, Some(DetachedDetail::TerminalGone));
    });
}
```

(If `apply_bridge_event` is not callable from tests as written, mirror the exact injection the neighbouring `apply_bridge_event` tests use around 2938/3040.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-terminal --lib _4404_`
Expected: `openorcreate_4404...` FAILS (`left: Detached, right: ReplacementWaiting`); `existing_4404...` PASSES (already the behavior).

- [ ] **Step 3: Implement the branch.** In `apply_bridge_event`, replace the `StopDetached` arm body (~2224–2233) so a `TerminalGone` on an `OpenOrCreate` target enters `ReplacementWaiting`:

```rust
PolicyAction::StopDetached { detail, reattach_available } => {
    debug_assert_eq!(
        reattach_available,
        matches!(detail, DetachedDetail::ClientDetached),
        "policy reattach_available must match ClientDetached detail"
    );
    // 4404-first (Slice 5-A): a TerminalGone on a discover-or-create target may
    // have an exact-key successor coming — wait for it (retaining the frozen
    // engine, Task 3) instead of hard-detaching. Existing targets never adopt a
    // different resource, so they stay a hard detach.
    if matches!(detail, DetachedDetail::TerminalGone)
        && matches!(self.target, TerminalTarget::OpenOrCreate { .. })
    {
        self.enter_replacement_waiting(cx);
    } else {
        self.on_detach(detail, cx);
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lens-terminal --lib _4404_` then `cargo test -p lens-terminal --lib`
Expected: both new tests PASS; full suite green.

- [ ] **Step 5: Gate + commit**

```bash
cargo clippy -p lens-terminal --all-targets -- -D warnings && cargo fmt --all -- --check
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-5-A): OpenOrCreate 4404 -> ReplacementWaiting (Existing stays hard-detach)"
```

---

### Task 3: Retain frozen engine + unified `adopt()`

`enter_replacement_waiting` retains the frozen engine (transport-only teardown) [change 2]. All adoption routes through one `adopt(session_id, terminal_id)` [Q1]: **same session → drop the retained engine, fresh `discover_and_attach`** (agent-switch, no leak); **changed session → reuse the retained engine** via the reconnect-transport shape. The >30s timeout still fully tears down (drops the retained engine → no leak) [Q2].

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — `enter_replacement_waiting` (~1932), replace `adopt_successor` (~1952) with `adopt`, update the `on_resource_signal` `AdoptSuccessor` dispatch (~1923-1926), and the existing test assertion at `open_or_create_delete_enters_replacement_waiting` (~2990) that asserts `runtime.is_none()`.
- Test: `crates/lens-terminal/src/lib.rs` tests module

**Interfaces:**
- Consumes: `teardown_transport_off_foreground(&mut self, cx)`, `teardown_runtime_full(&mut self, cx)`, `discover_and_attach(client, target, options) -> Result<AttachedParts, DetachedDetail>`, `on_attached(&mut self, parts, cx)`, `preflight_reconnect(client, &session, &tid) -> Result<TerminalResource, DetachedDetail>`, `attach(client, &session, &tid, AttachOptions) -> Result<AttachHandle, _>`, `on_reconnect_success(&mut self, resource, attach, cx)`, `close_parts_off_foreground`, `close_attach_off_foreground`, `self.{current_session, current_tid, reconnect_epoch, adopt_in_flight}`.
- Produces: `fn adopt(&mut self, session_id: SessionId, terminal_id: TerminalId, cx: &mut Context<Self>)` replacing `adopt_successor`. Same call sites (`on_resource_signal` `AdoptSuccessor` verdict).

- [ ] **Step 1a: Update the flipped existing assertion first (RED for retention).** At `open_or_create_delete_enters_replacement_waiting` (~2990), the current assertion is `assert!(tab.runtime.is_none(), "dead engine must be released on reset")`. Change it to the retention contract:

```rust
assert!(
    tab.runtime.is_some(),
    "reset must RETAIN the frozen engine (transport-only teardown) for possible reuse"
);
assert!(
    tab.runtime.as_ref().and_then(|r| r.engine_ref()).is_some(),
    "frozen engine must survive enter_replacement_waiting"
);
```

- [ ] **Step 1b: Write the failing adopt tests.**

```rust
#[gpui::test]
async fn same_session_adopt_drops_retained_engine_and_reattaches_fresh(cx: &mut gpui::TestAppContext) {
    let (_engine, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    // Reset -> ReplacementWaiting retains the frozen engine.
    tab.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") },
            cx,
        );
        assert_eq!(tab.lifecycle, Lifecycle::ReplacementWaiting);
        assert!(tab.runtime.is_some(), "engine retained in ReplacementWaiting");
    });
    // Same-session successor (agent-switch) -> adopt() takes the FRESH branch.
    tab.update(cx, |tab, cx| {
        let same = tab.current_session.clone().unwrap();
        tab.adopt(same, TerminalId::new("t1"), cx);
    });
    cx.run_until_parked();
    // Mirror the outcome shape of `adopt_outcome_applies_to_live` (~3166) for what the
    // stub attach yields; the invariant under test is: NO leaked second engine, and the
    // adopt path is the fresh one (a brand-new runtime, not the retained instance).
    tab.read_with(cx, |tab, _| {
        assert!(
            !tab.adopt_in_flight,
            "adopt must resolve (fresh path) — not leave adopt_in_flight set"
        );
    });
}

#[gpui::test]
async fn replacement_timeout_drops_retained_engine_no_leak(cx: &mut gpui::TestAppContext) {
    let (_engine, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    tab.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") },
            cx,
        );
        assert!(tab.runtime.is_some(), "engine retained pending successor");
        tab.fire_replacement_timeout_now(cx); // deterministic timeout
    });
    cx.run_until_parked();
    tab.read_with(cx, |tab, _| {
        assert_eq!(tab.lifecycle, Lifecycle::Detached);
        assert_eq!(tab.presentation.detached_detail, Some(DetachedDetail::ReplacementTimedOut));
        assert!(tab.runtime.is_none(), "timeout must fully tear down the retained engine (no leak)");
    });
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-terminal --lib adopt` and `cargo test -p lens-terminal --lib open_or_create_delete_enters_replacement_waiting`
Expected: retention assertion FAILS (`runtime.is_none()` today), and the timeout test FAILS if the retained engine isn't torn down. `same_session_adopt...` compiles only once `adopt` exists (Step 3b) — expect a compile failure now.

- [ ] **Step 3a: Retain the engine.** In `enter_replacement_waiting` (~1935) change `self.teardown_runtime_full(cx);` to `self.teardown_transport_off_foreground(cx);` and update the surrounding comment to say the frozen engine is retained for possible same/cross-session reuse.

- [ ] **Step 3b: Replace `adopt_successor` with unified `adopt`.** Keep the `adopt_in_flight` guard and the apply-time epoch re-check. Branch on session identity:

```rust
fn adopt(&mut self, session_id: SessionId, terminal_id: TerminalId, cx: &mut Context<Self>) {
    if self.adopt_in_flight {
        return;
    }
    let same_session = self.current_session.as_ref() == Some(&session_id);
    self.adopt_in_flight = true;
    // Cancel the replacement timeout; a successor is in hand.
    self.reconnect_epoch = self.reconnect_epoch.wrapping_add(1);
    let epoch = self.reconnect_epoch;
    let read_only = matches!(self.presentation.access, AccessMode::ReadOnly);

    if same_session {
        // Agent-switch: the retained engine belongs to the OLD generation; drop it and
        // discover+attach a FRESH engine against the exact key (today's behavior — no reuse).
        self.teardown_runtime_full(cx); // explicit drop closes the leak change 3a introduces
        let client = Arc::clone(&self.client);
        let options = self.options.clone();
        let target = TerminalTarget::Existing { session_id, terminal_id };
        cx.spawn(async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { discover_and_attach(client, target, options) })
                .await;
            let _ = weak.update(cx, |tab, cx| {
                if tab.reconnect_epoch != epoch {
                    tab.adopt_in_flight = false;
                    if let Ok(parts) = outcome { tab.close_parts_off_foreground(parts, cx); }
                    return;
                }
                tab.adopt_in_flight = false;
                match outcome {
                    Ok(parts) => tab.on_attached(parts, cx),
                    Err(detail) => tab.on_detach(detail, cx),
                }
            });
        })
        .detach();
    } else {
        // Cross-session transfer: REUSE the retained frozen engine — transport-only
        // re-attach against the new session (reconnect shape), retargeting current_session.
        let client = Arc::clone(&self.client);
        cx.spawn(async move |weak, cx| {
            let attempt = cx
                .background_executor()
                .spawn({
                    let client = Arc::clone(&client);
                    let session = session_id.clone();
                    let tid = terminal_id.clone();
                    async move {
                        let resource = preflight_reconnect(client.as_ref(), &session, &tid)?;
                        let attach = attach(client.as_ref(), &session, &tid, AttachOptions { read_only })
                            .map_err(|_| DetachedDetail::DiscoveryFailed)?;
                        Ok::<_, DetachedDetail>((resource, attach))
                    }
                })
                .await;
            let _ = weak.update(cx, |tab, cx| {
                if tab.reconnect_epoch != epoch {
                    tab.adopt_in_flight = false;
                    if let Ok((_r, attach)) = attempt { tab.close_attach_off_foreground(attach, cx); }
                    return;
                }
                tab.adopt_in_flight = false;
                match attempt {
                    // on_reconnect_success installs transport on the RETAINED engine and
                    // retargets current_session = resource.session_id (= the new session B).
                    Ok((resource, attach)) => tab.on_reconnect_success(resource, attach, cx),
                    Err(detail) => tab.on_detach(detail, cx),
                }
            });
        })
        .detach();
    }
}
```

Update the `on_resource_signal` `AdoptSuccessor` arm (~1926) to call `self.adopt(session_id, terminal_id, cx)`. (Autonomous adoption is always same-session, so it takes the fresh branch — but the check makes that a runtime fact, not an assumed invariant.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p lens-terminal --lib` (adopt + retention + timeout + the existing adoption/replacement tests)
Expected: all green, 210+ tests. If any pre-existing `adopt_successor`/replacement test references the old name or the old "engine released on reset" contract, update it to the retention contract (note each such change in the commit body).

- [ ] **Step 5: Gate + commit**

```bash
cargo clippy -p lens-terminal --all-targets -- -D warnings && cargo fmt --all -- --check
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-5-A): retain frozen engine in ReplacementWaiting + unified adopt() (same->fresh, changed->reuse)"
```

---

### Task 4: `TerminalHostEvent::Transfer` + cross-session no-double-feed test

Add the host-driven cross-session supersede entry point [Q3]: `Transfer { new_session }` calls `adopt(new_session, current_tid)` → the reuse branch. Lock the cross-session no-double-feed guarantee with a dedicated engine-seed test (the existing `reconnect_seed` tests only cover same-session reconnect).

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` — `TerminalHostEvent` enum (~379), `on_host_event` match (~658)
- Modify: `crates/lens-terminal/src/engine/reconnect_seed.rs` — add the cross-session seed acceptance test
- Test: both files above

**Interfaces:**
- Consumes: `adopt(&mut self, session_id, terminal_id, cx)` (Task 3), `self.current_tid: Option<TerminalId>`.
- Produces: `TerminalHostEvent::Transfer { new_session: SessionId }`.

- [ ] **Step 1: Write the failing host-event test.**

```rust
#[gpui::test]
async fn transfer_reuses_retained_engine_and_retargets_session(cx: &mut gpui::TestAppContext) {
    let (_engine, tab) = live_tab_for_test(cx, true, "t1", "main", "k");
    // Reset -> ReplacementWaiting retains the frozen engine.
    tab.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::ResourceDeleted { terminal_id: TerminalId::new("t1") },
            cx,
        );
        assert!(tab.runtime.is_some(), "engine retained pending transfer");
    });
    let engine_ptr_before = tab.read_with(cx, |tab, _| {
        tab.runtime.as_ref().and_then(|r| r.engine_ref()).map(|e| e as *const _ as usize)
    });
    // Host drives a cross-session transfer to session B.
    tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::Transfer { new_session: SessionId::new("session_B") }, cx);
    });
    cx.run_until_parked();
    tab.read_with(cx, |tab, _| {
        // Reuse branch: the SAME retained engine instance is still installed (not replaced).
        let engine_ptr_after = tab.runtime.as_ref().and_then(|r| r.engine_ref()).map(|e| e as *const _ as usize);
        assert_eq!(engine_ptr_before, engine_ptr_after, "Transfer must REUSE the retained engine, not build a fresh one");
        assert_eq!(tab.current_session, Some(SessionId::new("session_B")), "current_session retargeted to B");
        assert_eq!(tab.current_tid, Some(TerminalId::new("t1")), "terminal_id retained across transfer");
    });
}
```

(If the stub client's `preflight_reconnect`/`attach` for `session_B` does not succeed out of the box, follow the same stub setup the reconnect-success test uses — mirror `adopt_outcome_applies_to_live` (~3166) / the reconnect tests. The invariant under test is engine-instance identity + session retarget, not the network.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-terminal --lib transfer_reuses`
Expected: FAIL to compile — `TerminalHostEvent::Transfer` does not exist.

- [ ] **Step 3a: Add the enum variant** (after `Reattach`/`End`, ~387):

```rust
/// Host-driven cross-session supersede: the server has moved this live terminal
/// (same `terminal_id`) into `new_session`. Reuses the retained frozen engine via a
/// transport-only re-attach against B, retargeting `current_session` (design §10, Q3).
Transfer { new_session: SessionId },
```

- [ ] **Step 3b: Wire the handler** in `on_host_event` (~662):

```rust
TerminalHostEvent::Transfer { new_session } => {
    if let Some(tid) = self.current_tid.clone() {
        self.adopt(new_session, tid, cx);
    }
}
```

- [ ] **Step 4a: Run the host-event test to verify pass**

Run: `cargo test -p lens-terminal --lib transfer_reuses`
Expected: PASS.

- [ ] **Step 4b: Write + run the cross-session no-double-feed test** in `reconnect_seed.rs`, mirroring `retained_reconnect_seed_does_not_duplicate_scrollback` (~144) but framed as a cross-session transfer seed (the seed bytes B replays on attach must be clear+redraw only, not full history):

```rust
#[test]
fn transfer_seed_across_sessions_does_not_duplicate_scrollback() {
    // Identical mechanics to retained_reconnect_seed_does_not_duplicate_scrollback:
    // a retained engine already holds leg1 history; the cross-session (B) attach seed is
    // leg2 (clear+redraw). Feeding leg2 must NOT grow scrollback beyond one viewport.
    let legs = split_legs(); // reuse the existing helper
    let engine = Arc::new(EngineHandle::spawn(seed_test_cfg()).expect("engine"));
    engine.feed(legs.leg1_seed).expect("feed leg1");
    engine.build_now().expect("build");
    let sb0 = engine.inspect().total_rows;
    // Cross-session transfer replays the same clear+redraw seed contract as reconnect.
    engine.feed(legs.leg2_seed).expect("feed leg2 (transfer seed)");
    engine.build_now().expect("build");
    let sb1 = engine.inspect().total_rows;
    let delta = sb1.saturating_sub(sb0);
    assert!(
        delta <= seed_test_cfg().rows as u64,
        "transfer seed duplicated history across sessions (delta={delta}, sb0={sb0}, sb1={sb1})"
    );
}
```

Run: `cargo test -p lens-terminal --lib transfer_seed_across_sessions` (or the reconnect_seed test path if it is a separate `#[cfg(test)]` module)
Expected: PASS (the no-byte-replay contract holds identically for a different session).

- [ ] **Step 5: Gate + commit**

```bash
cargo test -p lens-terminal --lib
cargo clippy -p lens-terminal --all-targets -- -D warnings && cargo fmt --all -- --check
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/src/engine/reconnect_seed.rs
git commit -m "feat(terminal-5-A): TerminalHostEvent::Transfer -> cross-session engine reuse + no-double-feed test"
```

---

## After all tasks

- [ ] **Whole-slice-A Opus review** — dispatch a fresh Opus subagent (builds+tests) over `git diff <A-start>..HEAD`, with the frozen-seam checklist: every adopt/attach spawn epoch-guarded at apply; no retained-engine leak on any of {same-session adopt, timeout, Existing 4404, Transfer}; `Transfer` reuses the exact retained engine; `Ended`/frozen-state gate still holds. (codex is out of quota this week — Opus, not codex.)
- [ ] **Demo host events** — extend the terminal demo's chord map with a Transfer trigger (mirror the Slice-4 `ctrl-alt-{s,w,r,x,d}` chords) for a manual cross-session smoke, if cheap.
- [ ] Then D (fleet-integration) builds on A+B+C; live riders (supersede scrollback, 4404-first ordering) run against a live omnigent before the whole slice-5 merges to main together [Q6].

## Self-review notes (spec coverage)

- Change 1 (4404 branch) → Task 2. Change 2 (retain engine) → Task 3 Step 3a. Change 3 (unified adopt) → Task 3 Step 3b (both branches). Change 4 (Transfer) → Task 4. Change 5 (scrollback) → Task 1.
- Q1 unified adopt ✓ (Task 3). Q2 "don't regress + don't leak" + timeout drops engine ✓ (Task 3 timeout test). Q3 Transfer{new_session} through reuse branch + no-double-feed test ✓ (Task 4). Q4 Existing 4404 hard-detach ✓ (Task 2). Q5 scrollback_bytes/10MB ✓ (Task 1). Q6 land-together — no merge task in A (correct; D + final review gate the merge).
- Guard discipline (finding-3 lesson): every spawn in Task 3/4 re-checks `reconnect_epoch` at apply and closes freshly-built parts on mismatch. The `open()` guard is already landed (`b12676e`).
