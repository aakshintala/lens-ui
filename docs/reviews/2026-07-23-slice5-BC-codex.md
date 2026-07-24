# Slice 5 sub-slices B + C cross-family review

**Scope:** `60425d2..2c12a91`  
**Reviewer:** Codex / gpt-5.6 family  
**Recommendation:** **DON'T MERGE**

## Ranked findings

1. **HIGH — lifecycle-critical control signals use a lossy diagnostic ring**
   (**CONFIRMED**)

   - **Code:** `crates/lens-core/src/actor/runloop.rs:702-705`,
     `crates/lens-core/src/actor/runloop.rs:747`,
     `crates/lens-core/src/actor/outcome.rs:102-123`
   - `Superseded` and `TerminalResource` are ownership/lifecycle inputs, but
     `apply_reduced_batch` pushes them into `OutcomeRing`. That ring is explicitly a
     non-blocking diagnostic buffer: it drops its oldest entry at capacity and only
     retries a full outcome channel when another batch reaches `drain_outcome_ring`.
   - **Concrete failure:** the foreground outcome channel is full when A emits
     `session.superseded` (or when B emits the matching terminal create). `try_send`
     leaves the signal in the ring. If no later batch arrives, it remains stuck even
     after the foreground drains the channel; if another 64 outcomes accumulate first,
     it is dropped. D then never loads B / moves the member / drives `Transfer`, or
     never forwards the successor create, leaving the live terminal owned by the wrong
     session or stuck in `ReplacementWaiting`.
   - These variants need a reliable, ordered control path with bounded backpressure,
     not the best-effort diagnostic ring.

2. **HIGH — Sleep followed by Wake can apply a stale deferred Sleep after the
   terminal becomes Live** (**CONFIRMED**)

   - **Code:** `crates/lens-ui/src/fleet/terminal.rs:109-123`,
     `crates/lens-ui/src/fleet/terminal.rs:126-135`,
     `crates/lens-ui/src/fleet/terminal.rs:243-265`
   - `cascade_sleep` correctly sets `pending_sleep` for `Starting`, but
     `cascade_wake` neither clears that flag nor otherwise cancels the deferred
     action. `TerminalTab::Wake` is a no-op unless the tab is already `Sleeping`.
   - **Concrete failure:** a visible terminal is `Starting`; its session sleeps, so
     `pending_sleep=true`; the session wakes before discovery/attach finishes, so
     `Wake` is ignored and the flag remains set. When attach completes and emits
     `PresentationChanged(Live)`, FleetStore consumes the stale flag and immediately
     sleeps the visible terminal under an awake session.
   - Add a test for `Starting -> cascade Sleep -> cascade Wake -> Live`; Wake must
     cancel the pending cascade for non-hidden members.

3. **HIGH — close/end/map replacement removes membership without guaranteeing
   terminal teardown** (**CONFIRMED**)

   - **Code:** `crates/lens-ui/src/fleet/terminal.rs:59-70`,
     `crates/lens-ui/src/fleet/terminal.rs:95-106`,
     `crates/lens-ui/src/fleet/terminal.rs:139-140`,
     `crates/lens-ui/src/fleet/terminal.rs:237-240`
   - `open_terminal` returns a strong `Entity<TerminalTab>` while storing another
     strong clone. Removing the map entry therefore does not imply dropping the tab.
     `close_terminal` and `cascade_end` send no terminal teardown event. A duplicate
     `open_terminal` for the same inner key also silently replaces the tracked member
     without terminating or reusing the previous tab.
   - **Concrete failure:** the UI retains the entity returned by `open_terminal`;
     Archive calls `cascade_end`; FleetStore drops only its clone, while the UI-held
     tab keeps its transport and engine alive and is now outside fleet accounting.
     Similarly, a double-open leaves the first returned entity live but unmanaged.
   - The spec requires “tear down + remove member”; membership removal alone does not
     satisfy that requirement in a reference-counted entity model.

4. **HIGH — the new lens-ui dev dependency activates every terminal real-window
   harness in the ordinary test phase** (**CONFIRMED**)

   - **Code:** `crates/lens-ui/Cargo.toml:44-46`,
     `crates/lens-terminal/Cargo.toml:24-28`,
     `crates/lens-terminal/Cargo.toml:49-63`,
     `crates/xtask/src/main.rs:450-467`
   - Cargo feature unification makes `lens-terminal/test-util` active when the gate
     tests `lens-ui` and `lens-terminal` together. That makes all
     `required-features = ["test-util"]` real-window binaries eligible in the
     ordinary debug test command, contradicting the manifest comment and the gate's
     separate, selected release-harness phase.
   - **Concrete failure observed in this review:**  
     `cargo test -p lens-client -p lens-core -p lens-terminal -p lens-ui` unexpectedly
     ran `input_realwindow`, `mouse_realwindow`, and `presentation_realwindow`, then
     failed with `presentation_realwindow FAIL: link-cell click did not emit
     OpenUrlRequest`. Even if that individual miss is load-flaky, the new activation
     of all debug real-window harnesses is deterministic and makes the default gate
     broader/flakier than designed.
   - Wiring `lens-terminal/test-util -> lens-client/test-util` does not leak into a
     normal production build by itself, but using that broad feature from
     `lens-ui`'s dev dependency is not a safe test seam. Use a narrower feature/API
     for the engine-backed FleetStore helper or otherwise keep the real-window
     feature out of the ordinary workspace test resolution.

5. **MEDIUM — idle “auto-sleep” has no periodic driver** (**CONFIRMED**)

   - **Code:** `crates/lens-ui/src/fleet/terminal.rs:190-212`
   - The implementation exposes one synchronous `idle_tick` method, but there is no
     production call site or approximately-30-second task; repository references are
     only the method and its unit test.
   - **Concrete failure:** after Slice 6 marks a live terminal hidden, no memory
     warning occurs and the app remains open for hours. Because no tick fires, the
     terminal never reaches the ten-minute auto-sleep policy and retains its engine
     indefinitely.
   - The design treats the OS memory-warning source as a deferred seam, but describes
     the idle cadence itself as built in C. Either install the bounded periodic task
     here or explicitly re-scope the driver before calling auto-sleep built.

6. **MEDIUM — the promised two-atomic retained-size accessor performs a full
   inspect snapshot on the foreground thread** (**CONFIRMED**)

   - **Code:** `crates/lens-terminal/src/lib.rs:641-646`,
     `crates/lens-terminal/src/engine/inspect.rs:419-440`,
     `crates/lens-ui/src/fleet/terminal.rs:143-161`
   - The accessor contains a TODO for the required fast path and calls
     `engine.inspect()`. That snapshots every inspect counter and, when inspection is
     enabled, locks and clones the recent-event ring. `on_memory_pressure` invokes it
     serially for every eligible tab on the GPUI foreground thread.
   - **Concrete failure:** with inspection enabled and many hidden terminals, a memory
     warning makes the foreground thread take every engine inspect lock and clone up
     to 32 events per tab before policy selection. The implementation violates C's
     explicit “2 atomics” deliverable and adds avoidable UI-thread work precisely
     during memory pressure.

7. **MEDIUM — `Existing` membership smuggles a different identity through an empty
   `TerminalKey` sentinel** (**PLAUSIBLE**)

   - **Code:** `crates/lens-ui/src/fleet/terminal.rs:73-107`,
     `crates/lens-ui/src/fleet/terminal.rs:348-363`
   - A caller opens `Existing { terminal_id }`, but the public visibility/close APIs
     accept `TerminalKey`. The store privately indexes that member as
     `{ terminal_name: terminal_id.to_string(), session_key: "" }`; no public helper
     returns this synthetic key, and it is not the terminal's real logical key.
   - **Concrete failure:** Slice 6 opens an existing terminal and later uses metadata
     `{ terminal_name: "tui", session_key: "main" }` to hide or close it. The lookup
     misses the synthetic `{"terminal_tui_main", ""}` entry, so it stays visible/live
     in policy state. Reproducing the private empty-string convention in every caller
     would be stringly identity coupling.
   - Empty `session_key` is rejected by the server today, so collision with a valid
     `OpenOrCreate` key is unlikely; the problem is downstream addressability and
     identity fidelity. Prefer an inner identity enum such as
     `Existing(TerminalId) | Logical(TerminalKey)`.

8. **LOW — lens-drive serializes the typed terminal signal as Rust Debug text**
   (**CONFIRMED**)

   - **Code:** `crates/lens-drive/src/main.rs:751-754`
   - **Concrete failure:** a JSON consumer must parse a string such as
     `Created { terminal_id: ... }` to distinguish create/delete and recover fields.
     A harmless Rust rename or Debug-format change silently breaks that consumer.
   - This compile-unblock arm should emit a structured JSON object, as the adjacent
     `Superseded` arm does, to preserve the repository's typed-end-to-end rule.

## Judgment-call verdicts

### Sub-slice B

1. **`TerminalResourceCreated.session_id = state.id`: SOUND.** The event is reduced
   by the actor for the stream on which it arrived. On the supersede path the created
   event is published on successor B, so B's reducer state is the ownership session
   D must route through. The pinned created-event schema exposes a loose resource
   object rather than a top-level session id; using the actor's session identity also
   keeps routing tied to the subscribed stream. A debug assertion against
   `resource.session_id`, if lens-client later models it, would be useful drift
   detection but is not required for correctness.

2. **Missing terminal metadata -> `ResourcesChanged`: SOUND fail-safe.** Without both
   logical-key fields Lens cannot safely correlate/adopt a successor. Falling back to
   the generic marker avoids a false identity transition. The cost is a bounded,
   visible degradation (replacement may time out/detach) rather than attaching the
   wrong terminal.

3. **Leaving `reduce/mod.rs::map_item` untouched: SOUND for B/C.** The accepted D path
   is driven from the live `Superseded` control event and explicit FleetStore
   `Transfer`, not discovery from B's persisted resource item. Durable cold-start
   recovery may need a later design, but B does not need to invent it now.

4. **Interim match arms:** the card/focused no-ops are sound because the actor strips
   these control-only updates from `ActorFeed`; the poller no-op is a deliberate
   B-before-D boundary and drops no previously implemented behavior. It is only safe
   as an interim build because D must replace it before terminal control integration
   is considered complete. The lens-drive Debug-string arm is not sound; see finding
   8.

### Sub-slice C

1. **`TerminalKeyId` as the inner-map key: SOUND.** It contains exactly the two fields
   participating in `TerminalKey` equality and derives both `Hash` and `Eq`; ordinary
   hash collisions are resolved by equality, so this introduces no identity
   collision. It is slightly duplicative but correct for the current frozen shape.

2. **`Existing` -> empty `session_key`: NOT SOUND as a public downstream identity.**
   It is collision-resistant under today's non-empty server contract, but it makes
   visibility/close callers depend on a private sentinel and loses the distinction
   between exact and logical identity. See finding 7.

3. **`test-util -> lens-client/test-util`: conditionally sound, current integration
   not merge-safe.** The feature does not enter normal production builds unless
   explicitly enabled, and it is a reasonable way to expose `Client::stub_for_test`.
   However, the `lens-ui` dev-dependency activates the same broad feature that gates
   real-window tests, changing and currently breaking the ordinary gate phase. See
   finding 4.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**
- `cargo fmt --all -- --check` — **PASS**
- `git diff --check 60425d2..2c12a91` — **PASS**
- `cargo test -p lens-ui --lib` — **PASS, 173/173**
- `cargo test -p lens-client -p lens-core -p lens-terminal -p lens-ui` —
  **FAIL** after the newly activated debug real-window harnesses reached
  `presentation_realwindow`

## Overall recommendation

**DON'T MERGE.** Findings 1-4 are merge blockers: lifecycle ownership signals are
not delivered reliably, the cascade state machine has a stale-Sleep race,
end/close does not guarantee teardown, and the ordinary test gate is broadened into
a failing real-window run. Re-review after those are fixed; findings 5-8 should be
resolved or explicitly re-scoped before C is called complete.
