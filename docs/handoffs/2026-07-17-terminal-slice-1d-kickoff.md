# Handoff — Terminal Slice 1d (convergence): kickoff for a fresh session

Slice **1c is DONE, `xtask gate` is GREEN and no longer flaky**, all on
`terminal-ws` (unpushed). 1d is next and its plan is already written + reviewed.
This is a start-here pointer, not a re-plan.

## Authoritative artifact

**`docs/plans/2026-07-16-terminal-slice-1d-convergence.md`** (842 lines,
task-based, checkbox-tracked). Execute it with
**`superpowers:subagent-driven-development`** (or `executing-plans`). Do NOT
redesign — the C2 wake mechanism, C3 ownership/teardown, and C4 reconnect are
locked in the plan. Ground truth: `docs/specs/2026-07-16-terminal-workstream-design.md`.

**Goal:** converge landed 1a transport + 1b engine + 1c paint into a live
`open()`→`TerminalTab` path: one bridge thread (`AttachHandle.inbound`↔
`EngineHandle`), close-code policy, lifecycle subset + gap marker, hidden-tab
Frame suppression, standalone GPUI demo, live proof vs omnigent 0.5.1.

## Tasks (see plan for detail)

- **T1** bridge thread + `BridgeEvent::OutboundSaturated` + join
- **T2** `TerminalRuntime` ownership + off-foreground teardown
- **T3** `open()` via C2 `background_executor` + foreground apply
- **T4** close-code policy + `RetryWindow` clock + gap marker
- **T5** wake sampler samples `TerminalTab.latest_frame` + resize-before-input + `set_visible`
- **T6** retained-engine reconnect-seed acceptance (C4)
- **T7** basic generation guard (pre-reconnect GET)
- **T8** standalone GPUI demo (handshake before GPUI)
- **T9** Inspect + live vertical rider

**Delegation (per project rules):** T1–T7 are composer-delegable
(`cursor-delegate`, composer-2.5) — static-shaped, fully specced; cross-family
review each ([[composer-delegation-profile]]). **T8 demo + T9 live rider run
inline** (real GPUI window + live omnigent — same reasons 1c's harness ran
inline: `Application::run` + `std::process::exit` don't fit the sandbox).

## Prerequisites before starting

1. **Add `async-channel = "2"`** to `crates/lens-terminal/Cargo.toml` (workspace
   already uses it in lens-ui/lens-core) — T3/T5 wake plumbing needs it.
2. **T9 needs a running omnigent 0.5.1** — use the `installing-omnigent-from-source`
   skill. T1–T8 do not need it; don't block on it.
3. Remove the `#[expect(dead_code)]` on `TerminalTab::{target,client,options}`
   once T3 consumes them.

## 1c seams 1d consumes (all landed, do NOT redefine)

- `render::paint_frame`, `CellMetrics::resolve_menlo`,
  `TerminalTab.latest_frame: Option<Arc<Frame>>`, `set_frame_for_test` (1c).
- Engine (1b): `EngineHandle` (`Send`), `feed`/`da_dsr_rx`/`set_waker`/
  `set_visible`/`build_now`/`stop`, `latest_frame()`. **Note:** the engine waker
  runs on the worker thread and must only `try_send` onto a bounded channel —
  never touch a gpui entity (plan C2 §3).
- Transport (1a): `attach`/`Terminals`/`Backoff`/`CloseCause`; **`AttachHandle::Drop`
  joins synchronously** → never drop it or call `stop()`/`close()` on the gpui
  foreground (plan C3).

## State / gotchas carried in

- **Gate:** `cargo run -p xtask -- gate` (green as of session end). It runs the
  render harness **`--release`** — perf budgets are 120fps *product* targets and
  debug is ~5.4× slower (see `docs/plans/2026-07-16-terminal-slice-1c-perf-resolution.md`
  and memory [[terminal-slice-1c-executed]]). Never pipe the gate through `tail`
  (masks the exit code — memory [[xtask-gate-scope]]).
- **The engine gate flake is FIXED** this session (per-handle build-failure
  injection, was a process-global static consumed cross-test). 120/120 stress
  runs clean; the gate is deterministic again.
- Measure any new real-window perf **foreground** — the GPUI harness throttles
  `request_animation_frame` when unfocused (memory [[gpui-test-noop-text-system]]).
- `terminal-ws` is **unpushed**; solo-project convention is merge-to-main + no
  PRs ([[integration-workflow]]), but the user makes the push/merge call.

## Recent commits (this session, perf-block resolution + flake fix)

`63f490f` (resolution plan) → `b1cc3e2` (gate `--release`) → `ad7e049`
(resolve-once cleanup) → `730fc83` (codex review fix: per-cell decoration order)
→ `53a75be`/`f4f0d15` (release-calibrated budgets) → `f5a660e` (engine flake
fix) → `6eb3717` (STATUS). C-a shape-cache reopen is **retired** (shaping was
never the bottleneck).
