> **✅ SUPERSEDED — Slice 2c is DONE (2026-07-21).** All tasks T5–T8 executed, whole-slice codex
> review (9 findings) + re-review (3) folded, `mouse_realwindow` (4 phases) + live P6 PASS, docs
> updated. Full slice `f1922c5..c924fd9` on `terminal-ws` (unpushed — merge→main is the user's call).
> This handoff is retained as the execution record; current state lives in `docs/STATUS.md`.

# Handoff — Terminal Slice 2c (mouse) EXECUTION, resume at Task 5 (2026-07-21)

**Self-contained driver for a fresh session.** The engine side of 2c (Tasks 1–4) is DONE, committed, gate-green. Resume at **Task 5** (foreground capture) → T6 → T7 → **Opus whole-slice review** → **T8** (needs a user heads-up).

## The plan is the driver
`docs/superpowers/plans/2026-07-21-terminal-slice-2c-mouse.md` (Rev 3 — engine-side arbitration; 2 codex plan-reviews + 2 code-review fix-waves already folded). Read it. Execute each remaining task's TDD steps EXACTLY. Ground truth spec: `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` §2c.

## State (branch `terminal-ws`, unpushed; HEAD `4e777ed`)
Commit chain since `e29a501`:
- `ec6a60a` T1 — mouse/wheel/copy DTOs + `LocalClick` presentation variant
- `588f8b9` plan Rev 3 doc
- `9121219` T2 — VtEngine mouse encoder + selection gesture + copy extract; `18d6203` T2 fix-wave (coalesce-safe encoder reconfig + multi-click thresholds + select_all dirty — from codex T2 review)
- `a6092d9` T3 — engine MouseGesture arbitration + epoch/suppression latch + LocalClick + ack-every-branch; `419d736` T3 security fix-wave (**engine-authoritative `SetAccess` closes a report auth-laundering bypass** + lost-Up/suppression-invariant/LocalClick/no-panic — from codex T3 review, Opus-verified); `163f569` plan SetAccess constraint doc
- `4e777ed` T4 — atomic Wheel report/local-scroll + SelectAll + async capacity-1 Copy

**Gate green at HEAD:** `cargo test -p lens-terminal --lib` = **159 passed**, workspace clippy + `-p lens-terminal --features test-util,live-tests` clippy + fmt all clean.

## What's DONE (engine side — do NOT rebuild)
- **DTOs** (`engine/command.rs`): `MouseGesture` (NO `write_allowed` field — access is engine-authoritative), `MouseButtonKind{Left,Right,Middle}`, `MouseEventKind{Down,Move,Up}`, `MouseReportPolicy{Auto,ForceLocal}`, `GestureOwner`, `GestureDisposition`, `MouseAck{encoded,disposition}`, `WheelInput` (NO `write_allowed`), `MouseReportEv`, `CopyResult`/`CopyResponder`, `MouseTracking`/`MouseFormat`.
- **VtEngine caps** (`engine/vt.rs`): `cols`/`rows` fields (updated in `resize`); `read_live_tracking`/`read_live_format`; `encode_mouse_report` (coalesce-safe: only reconfigs encoder on change); `apply_selection_press/drag/release` (→`Result<bool>`, split-borrow, `set_time`+`set_repeat_distance` multi-click); `gesture_dragged`; `select_all`/`clear_selection` (dirty on clear); `extract_selection_text` (unwrap+trim).
- **Worker arbitration** (`engine/worker.rs`): `EngineCommand::{MouseGesture,Wheel,SelectAll,Copy,SetAccess(bool)}` arms. `MouseState{ latch:Option<Latch{owner,button,epoch,suppressed,dragged}>, write_allowed:bool, any_button_pressed }`. Report gate = `mouse_state.write_allowed && cmd_epoch==current_epoch && tracking!=None && !shift && !mouse_local && policy==Auto`. `MouseAck` on every branch. Wheel atomic (never touches latch). `LocalClick` emitted on Select-Up no-drag after successful clear.
- **Handle** (`engine/handle.rs`): `enqueue_mouse_gesture`/`enqueue_wheel`/`enqueue_set_access`/`select_all`/`request_copy` (all forwarder-routed); `enqueue_input` stamps `MouseGesture`/`Wheel` epoch; `is_stale` false for all new variants.
- **Inspect** (`engine/inspect.rs`): `record_mouse_encoded`/`record_mouse_report_coalesced`/`record_mouse_suppressed`/`record_wheel_reported`/`record_copy_started`/`record_copy_completed`/`record_copy_empty`.
- **Presentation** (`engine/presentation.rs`): `EnginePresentationEvent::LocalClick{col,row}`.

## ▶ REMAINING TASKS

### Task 5 — Foreground capture + lowering (`lib.rs` + `render/state.rs`)
Per plan Task 5. **⚠ CRITICAL FOLLOW-THROUGH:** the foreground MUST call `self.engine_handle().enqueue_set_access(self.write_input_allowed())` on **open** AND on **every access/lifecycle change** that alters `write_input_allowed()` (read-only downgrade/upgrade, `input_enabled` toggle). Until this is wired, the engine defaults writable and read-only will NOT suppress reports in production — the T3 security fix is inert without it. Add a test/assertion that a read-only tab causes a `SetAccess(false)` to be enqueued.
- Register `on_mouse_down/up(Left/Middle/Right)`, `on_mouse_move`, `on_mouse_up_out(Left/Middle/Right)` in `render/state.rs`.
- `handle_mouse_{down,move,up,up_out}`: pixel→cell (`hit_test::pixel_to_cell`, None outside), surface px, `time = self.mouse_time_base.elapsed()`, build `MouseGesture{ mouse_local, policy, mods, .. }` (NO write_allowed — removed) → `enqueue_mouse_gesture` IMMEDIATELY (no deferral, no mode logic).
- `handle_wheel`: `gpui_scroll_to_lens` → `enqueue_wheel(WheelInput{lines,..})`.
- `LocalClick` drain in `drain_presentation_events` → `hit_test::uri_for_gesture(&frame,col,row)` → `cx.emit(OpenUrlRequest(url))`.
- Cmd+C → `handle_copy` (two-stage: `bg_executor.spawn(recv_timeout).await` → fg `update` writes clipboard; never block fg, never touch GPUI from bg). Cmd+A → `select_all`. Mirror the Cmd+V intercept at top of `handle_key_down`.
- Add `mouse_local: bool`, `report_policy: MouseReportPolicy`, `mouse_time_base: Instant` fields (defined HERE — first user). `#[cfg(feature="test-util")]` debug hooks: `debug_mouse_{down,move,up,up_out}_for_test`, `debug_wheel_for_test`, `debug_handle_copy_for_test`, `debug_select_all_for_test`.

### Task 6 — mouse-local toggle + policy wiring + arbitration coverage (`lib.rs`)
`toggle_mouse_local` + tests that each arbitration path (ForceLocal/mouse_local/shift/Auto) routes correctly through the engine arbiter.

### Task 7 — per-tracking-mode motion + coalesce reset + full inspect + benches
Per-mode motion (X10 press-only / Normal press+release / Button motion-with-button / Any all-motion) in the worker; explicit `CoalesceKey{owner,tracking,format,mods}` reset via `mouse_encoder.reset()` on change; finalize inspect; `benches/engine.rs` `bench_mouse_encode_throughput` + `bench_mouse_motion_coalesced`.

### (before T8) Opus whole-slice review
**codex stalled repeatedly this session** (transient) — do the whole-slice review with **Opus via the `Agent` tool** (or retry codex if recovered). Security callout: mouse-report-to-PTY + the SetAccess read-only gate + the T5 SetAccess wiring + copy/clipboard. Fold findings.

### Task 8 — real-window proof + live P6 + docs (NEEDS USER)
Per plan Task 8. **READ memory `terminal-realwindow-harness-pitfalls` FIRST.** `mouse_realwindow.rs` (report bytes + drag-select + LocalClick hyperlink + read-only suppression) — **give the user a heads-up (opens a macOS window)**. Live P6 rider needs the omnigent 0.5.1 server (memory `omnigent-terminal-attach-live-run`). Update BOTH specs (`2026-07-16` matrix + `2026-07-17` §2c/DP3/open-question RESOLVED-DEFERRED). Note the 2b bracketed-paste-warn nuance stays deferred.

## Execution mechanics (unchanged)
- **composer-2.5 per task** via `cursor-delegate` (capability write), gate = `["cargo test -p lens-terminal --lib", "cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings", "cargo fmt --check"]`. `cargo test` takes ONE positional filter. Never pipe gate through `tail`.
- **Reviews:** codex `codex exec -s read-only` was the per-task reviewer but STALLED this session (hung at "Reading additional input from stdin"). Fall back to Opus (`Agent` tool) for the whole-slice. Don't run a composer build (edits tree) concurrently with a review (reads tree) — serialize.
- Commit per task (composer does it). Push + `main` merge remain the user's standing call.
- Memories: `terminal-2c-planned`, `terminal-2c-xtshiftescape-not-in-c-abi`, `terminal-realwindow-harness-pitfalls`, `omnigent-terminal-attach-live-run`, `gpui-test-noop-text-system`, `composer-delegation-profile`.

## Deferrals carried (not bugs)
- XTSHIFTESCAPE program-requested shift-capture override (no C-ABI accessor upstream — memory `terminal-2c-xtshiftescape-not-in-c-abi`).
- 2b always-warn-on-multiline-paste nuance (needs an ordered engine-side paste-mode decision; still deferred).

After 2c: **Slice 3** (lifecycle/fleet = WP5), **Slice 4** (perf acceptance = WP6+WP7).
