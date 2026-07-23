# Slice 5 sub-slices B+C — fix-delta re-review (Opus)

**Scope:** `2c12a91..HEAD` (`694870b` findings 1-4 + re-scope 5/7; `6e169ae` findings 6+8)
**Reviewer:** Claude Opus (independent)
**Method:** read every change + callers + tests; BUILT and TESTED, incl. an author-of-a-repro test for finding 3.
**Recommendation:** **DON'T MERGE** — one residual HIGH (finding 3 is incomplete; proven by a failing repro). Everything else is correctly resolved.

## Verification run

- `cargo test -p lens-core -p lens-client -p lens-terminal -p lens-ui --lib` — **PASS** (177 lens-terminal, full suite green)
- `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**
- `cargo fmt --all -- --check` — **PASS**
- `cargo test -p lens-terminal -p lens-ui --no-run 2>&1 | grep -i realwindow` — only the two lens-ui `#[ignore]` probe wrappers build; **none** of the lens-terminal `harness=false` real-window bins do.
- New fix tests all pass individually: `control_outcome_survives_outcome_channel_backpressure`, `cascade_wake_cancels_pending_sleep_for_visible_starting`, `close_terminal_tears_down_the_tab`, `cascade_end_tears_down_each_tab`, `double_open_ends_the_previous_tab`, `retained_bytes_estimate_fast_path_matches_snapshot`.

---

## Ranked findings (most severe first)

### A. HIGH — the initial `open()` attach spawn resurrects an `Ended` tab (finding 3 is INCOMPLETE)

- **Code:** `crates/lens-terminal/src/lib.rs:2667-2675` (the `open()` closure) and `on_attached` `lib.rs:1551-1607` / `on_detach` `lib.rs:1705-1718` — **neither the closure nor `on_attached`/`on_detach` guards on `reconnect_epoch` or `lifecycle`.**
- Every *other* attach spawn re-checks the epoch at apply time and bails: wake `1784`, reconnect `1854`, adopt `1977`, ReplacementWaiting `2013`. `on_host_end` bumps `reconnect_epoch`, so those are all correctly revoked. The initial `open()` spawn captured **no** epoch and calls `on_attached`/`on_detach` unconditionally.
- **Concrete failure (proven):** `FleetStore::open_terminal` → `lens_terminal::open` returns a `Starting` tab with `discover_and_attach` in flight. Before it resolves, `close_terminal` / `cascade_end` / a double-open runs `end_member_tab` → `on_host_end` → `Lifecycle::Ended` + `teardown_runtime_full`. The in-flight attach then lands `weak.update(...)` on the caller-held strong clone and calls `on_attached` (→ installs a fresh engine + transport, `Lifecycle::Live`) or `on_detach` (→ `Lifecycle::Detached`). The tab escapes the `Ended` sink — in the success case with a **live engine + transport held only by the caller, outside fleet accounting**: exactly the leak finding 3 set out to prevent.
- **I reproduced it.** A temporary `#[gpui::test]` doing `open(...)` → `on_host_event(End)` (asserts `Ended`) → `run_until_parked` → asserts `Ended` **FAILED**: `resurrected an Ended tab to Detached` (stub discovery fails → `on_detach`; a real server that attaches would land `Live` + leaked runtime). Test reverted after confirming.
- The fix's own docstring at `lib.rs:1686` claims *"Idempotent: safe from any state, including an already-torn-down tab"* and the commit claims safety *"including Starting (attach spawn in flight)"* — **that claim is false.**
- **Blast radius today is low** (findings 5/7 established there is no production caller of the FleetStore terminal API yet), so this is not a live crash — but it is a latent HIGH that (a) ships a false safety claim and (b) silently leaks a live engine+transport the moment Slice 6 wires `open_terminal` + close/cascade.
- **Fix (trivial):** make the terminal sink actually terminal — bail when already `Ended`, e.g. at the top of `on_attached`/`on_detach` `if self.lifecycle == Lifecycle::Ended { /* close/teardown any freshly-built parts */ return; }`, or capture `epoch = 0` in the `open()` closure and re-check `tab.reconnect_epoch != epoch || tab.lifecycle == Ended` (mirroring the other four spawns), closing `parts` on mismatch so the just-built engine isn't leaked.

**Verdict: NOT-RESOLVED for the Starting-with-in-flight-initial-attach case.** (The attached/detached/sleeping/reconnecting/replacement-waiting states ARE correctly torn down and revoked.)

---

### Original findings — resolution verdicts

**1. Control signals off the lossy ring (lens-core) — RESOLVED, no new deadlock.**
`Superseded`/`TerminalResource` are collected into a `Vec` and delivered via `outcomes.send_blocking` after `drain_outcome_ring` (`runloop.rs:697-758`). Deadlock analysis: the poller (`lens-ui/src/fleet/poller.rs`) drains `outcomes` independently of `feed` via `futures::select` in a single async task whose `store.update` bodies are pure store logic — it **never blocks on the actor**, so a full outcomes channel is always eventually drained. The actor is single-threaded, so while blocked on `send_blocking(outcomes)` it produces no further feed items (no feed-flood starvation of the biased select). The foreground dispatches commands with `try_send` (`store.rs:321`), so the executor thread is never synchronously parked waiting on the actor. This is the identical discipline as the pre-existing `Parked` `send_blocking` and the feed `send_blocking` — **no new deadlock class**. Ordering: control outcomes preserved in batch order, emitted after feed + ring-drain and before the `Parked`/disconnect send — correct. Backpressure test (cap-1 channel) passes.

**2. cascade Sleep→Wake stale flag (lens-ui) — RESOLVED, no regression.**
`cascade_wake` (`terminal.rs:130-145`) clears `pending_sleep` only for `!hidden` members before sending `Wake`. Closes the race: `Starting` → `cascade_sleep` (defers) → `cascade_wake` (clears) → `Live` no longer re-sleeps under an awake session (`on_terminal_presentation_changed` sees `pending_sleep=false`). Clearing is *always* correct for a visible member on wake, because a woken session makes any deferred sleep stale by definition. Hidden members are deliberately left to policy (`is_policy_eligible = hidden && Live`) — unchanged from before, so no regression. Test passes.

**3. close/end/double-open teardown (lens-terminal + lens-ui) — PARTIALLY RESOLVED.** See finding A above. The `End` event + `teardown_runtime_full` + `Ended` sink are correct and idempotent for attached/detached/sleeping/reconnecting/replacement-waiting; `teardown_runtime_full` safely no-ops on `None` runtime; the epoch bump revokes the reconnect/adopt/wake/RW in-flight spawns. **Gap: the initial `open()` spawn is unguarded → resurrection.** `Ended` was previously defined-but-never-produced; no other code assumed it unreachable (verified: no `unreachable!`/exhaustive-panic on it).

**4. real-window feature split (lens-terminal Cargo + xtask) — RESOLVED.**
New `real-window = ["test-util"]` gates all six `harness=false` bins; `test-util` no longer does. Proven: `cargo test -p lens-terminal -p lens-ui --no-run` builds **none** of `render/stream_perf/input/presentation/mouse/terminal_live_realwindow`. The two hits (`focused_finalize_realwindow`, `focused_scroll_realwindow`) are **lens-ui** `#[ignore]`-guarded probe wrappers that always compiled and never run headless — unrelated, pre-existing, safe. xtask runs `render_realwindow` + `stream_perf_realwindow` with `--features real-window` (`xtask/src/main.rs:487,499`). Feature unification via lens-ui's dev-dep can no longer drag real-window bins into the ordinary gate.

**5. idle_tick has no driver — RE-SCOPE JUSTIFIED.**
Grep confirms the only `idle_tick` caller is the unit test (`terminal.rs:981`). No production FleetStore terminal driver exists. `DRIVER DEFERRED` comment (`terminal.rs:202-207`) is adequate; Slice 6 owns the periodic tick + OS memory source.

**6. 2-atomic retained-bytes fast path (lens-terminal) — RESOLVED.**
`InspectShared::retained_bytes_estimate()` (`inspect.rs:218-226`) reads only `cols`+`total_rows` (no ring lock); `snapshot()` delegates to it (`inspect.rs:438-441`) so they cannot diverge; `EngineHandle`/`TerminalTab` accessors delegate. `record_resize` (`inspect.rs:203-204`) and `record_retained_rows` (`inspect.rs:215`) store the atomics **unconditionally**, ahead of the `enabled`-gated `record_event`. Callers are inspection-agnostic: `worker.rs:358` samples on the build cadence (gated only by `dirty`+throttle), `worker.rs:823` on resize. Value is identical to the old `inspect().retained_bytes_estimate` (same atomics, same `total_rows × cols × PER_CELL_BYTES`); two tests assert fast == snapshot. Correct with inspection off.

**7. `Existing` synthetic empty-`session_key` key — RE-SCOPE JUSTIFIED.**
No production `FleetStore::open_terminal(TerminalTarget::Existing)` caller: all `Existing` constructions outside tests are `discover_and_attach`'s match arm, the demo's direct `lens_terminal::open`, and the key-synthesis site itself. The public `TerminalKey`-addressed APIs cannot round-trip the sentinel, but nothing reaches it in production. `PROVISIONAL` comment (`terminal.rs:390-399`) is clear and names the Slice-6 fix (identity enum). Adequate.

**8. lens-drive Debug-string JSON — RESOLVED.**
`main.rs:751-773` now matches `TerminalResourceSignal::{Created,Deleted}` and emits structured typed JSON (`variant`/`signal` discriminators + typed fields). Exhaustive match against the real enum (`outcome.rs:11-21`); compiles + clippy-clean (so `TerminalId`/`SessionId` serialize). No Debug string.

---

## Overall

**DON'T MERGE** until finding A (the initial-`open()` resurrection) is fixed or explicitly re-scoped alongside 5/7 **with the false docstring/commit safety claim corrected**. It is a ~3-line fix. Findings 1, 2, 4, 6, 8 are correctly and completely resolved; 5 and 7 are legitimately re-scoped to Slice 6. No other new bug was introduced by the delta.
