# Handoff — Terminal Slice 2: EXECUTION (2026-07-17, SERIAL re-cut)

**Self-contained driver for a fresh session** whose job is to **execute** Slice 2. Planning is
DONE and gpt-5.6-reviewed; do NOT re-plan or re-review the plan bodies — execute them.

**2026-07-17 update — Task 0 dissolved; execution is now SERIAL.** The earlier parallel-worktree
structure (Task 0 foundation + 2a∥2d in isolated worktrees + merge) was **replaced** with a straight
serial sequence on `terminal-ws`. Task 0 existed *only* to make parallel worktrees merge-safe; in
serial there is no merge, so each slice declares and fills its own surface against the prior slice's
committed code. The Task 0 plan file was deleted and its declarations folded into 2a and 2d.

## State

- **Two plans, execution-ready**, in `docs/superpowers/plans/`:
  - `2026-07-17-terminal-slice-2a-input.md` — input/IME/focus/read-only. **Self-contained, lands FIRST.**
  - `2026-07-17-terminal-slice-2d-presentation.md` — titles/hyperlinks/presentation egress. **Lands
    AFTER 2a**, on 2a's committed code.
  - (`…-task0-foundation.md` was **deleted** — dissolved into 2a/2d.)
- **Design spec (ground truth):** `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`;
  parent `docs/specs/2026-07-16-terminal-workstream-design.md` (matrix + Open-contract-gaps
  amended for the progress/notification defer).
- **Branch:** `terminal-ws` (pushed to `origin/terminal-ws`; **not** merged to `main` — user holds
  the whole terminal workstream on-branch). Slice-2 **planning** (the design + both plan docs) is
  **already committed** (673d8bb). The serial re-cut (this handoff + plan edits + Task-0 deletion)
  is uncommitted at handoff time — **commit it first**, then execute.
- **Durable context:** memories [[terminal-slice-2-design-ghostty-precedent]] (design + spike
  resolution), [[terminal-parallel-worktree-task0-foundation]] (**superseded** — records why Task 0
  was needed for the PARALLEL structure; serial dropped it), [[terminal-slice-1d-executed]],
  [[gpui-test-noop-text-system]], [[plan-detail-vs-delegation-calibration]],
  [[composer-delegation-profile]], [[premature-layer-boundary-binding]].

## Execution order (STRICT, serial on `terminal-ws` — no worktrees, no merge)

1. **2a (input) — FIRST.** Self-contained: declares AND fills egress rename, `key_encoder`/`key_event`
   fields, `Frame.cursor`/`CursorPos`, `EngineCommand::{Key,Focus,LocalScroll}`, `input_forwarder`,
   `ime_preedit`, `on_key_down`/`on_key_up` render hooks, `input_gate.rs`. **`VtEngine::new` keeps its
   current arity** (no `presentation_tx`). Execute via **subagent-driven-development** per the plan
   header — composer-2.5 per task + per-task review; cross-family review by a non-author family
   (gpt-5.6 default — [[review-spend-policy]]; codex/gpt-5.5 = free fallback). Gate after each task.
2. **2d (presentation) — SECOND**, on 2a's committed code. Self-contained: T1 declares the presentation
   surface (create `engine/presentation.rs`, wire the presentation channel through
   `WorkerChannels`/`worker_channels`/`spawn_worker`/`EngineHandle`, **add `presentation_tx` to
   `VtEngine::new`** + update every call site, register bare `on_title_changed`, add the
   `drain_presentation_events` render hook + presentation methods); T2 sanitize; T3 adds
   `FrameCell.hyperlink_uri: Option<Arc<str>>` + OSC-8 extraction; T4 adds `next_host_request_id` +
   `on_mouse_down` + URL validation/gesture. Same execution flow + cross-family review (different
   family than 2a's reviewer for diversity).
3. **Then 2b** (clipboard/OSC-52 policy — needs 2d's presentation egress; **owns** the
   `on_clipboard_write` registration + the cap-before-clone that 2d deliberately deferred), **then 2c**
   (mouse — opens with the **XTSHIFTESCAPE `mouse_shift_capture` safe-FFI spike**, still unresolved).
   Neither is planned yet.

## Why serial (and why Task 0 is gone)

The gpt-5.6 review found 2a and 2d both edit `VtEngine::new`, `WorkerChannels`/`spawn_worker`,
`EngineHandle::spawn`, `build_frame`, and `TerminalTab::render`. In **parallel** worktrees each plan's
struct literals omit the other's fields — an invisible merge hazard (compiles in isolation, drops a
field on merge). Task 0 was invented to pre-declare every shared field as an inert placeholder so each
worktree was a single writer. **Serial removes the hazard at the root:** 2d edits 2a's *real* committed
code, so it sees 2a's actual struct/fn shapes and adds its own fields directly — no placeholders, no
merge, no dead-code carried through 2a. Cost: 2a and 2d run sequentially (no wall-clock parallelism),
judged not worth the Task-0 ceremony for a solo/agent-delegated build.

## Non-negotiables baked into the plans (don't regress during execution)

These were folded from the gpt-5.6 review — the plans contain the tests; keep them honest:
- **2a:** keydown handles special/modified keys only, `EntityInputHandler` owns committed text
  (no double-emit); `LensKey` = FULL physical key set (Ctrl-C → `\x03`, Kitty release/repeat); encoded
  user input is **never-drop** (route egress saturation → reconnect, not drop-oldest — that's replies
  only); a `Feed` is an **atomic ordering unit** (worker chunking = Stop-preemption + bounded quantum
  ONLY, **not** mid-Feed input interleave); off-fg forwarder never blocks the GPUI foreground, Drop is
  non-blocking; read-only downgrade **revokes already-queued input** (access epoch); `Frame.cursor` is
  viewport-safe `Option` (hide preedit off-viewport, never `unwrap_or(0)`); real-keystroke tests
  through the painted window (NoopTextSystem false-greens — [[gpui-test-noop-text-system]]).
- **2d:** `url::Url` validation (exact http/https + non-empty host; reject whitespace/controls/
  backslash/`#frag`/`?x`/bare-`http://`); **OSC-52 registration deferred to 2b** (2d only declares the
  event variant); `Arc<str>`-interned URIs + minimized `grid_ref` (it warns against render-loop use —
  guard the 1c perf verdict); latest-title coalesce; **click-only** (hover deferred).

## House rules (unchanged)

- `xtask gate` = `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`, workspace tests.
  **For `lens-terminal`, per-task clippy MUST include `--features test-util` (and `live-tests`).**
  Never pipe the gate through `tail`.
- Commit verified work; push is a separate call ([[commit-when-finished]]). Solo-merge straight to
  branch, no PRs ([[integration-workflow]]).
- Ghostty reference: re-clone `ghostty-org/ghostty` @ `a887df42` into scratchpad if a citation needs
  re-verifying (prior clone was ephemeral).

## First action

Commit the serial re-cut (this handoff + the two edited plan docs + the Task-0 deletion), then execute
**2a** task-by-task (composer-2.5 → per-task cross-family review → gate → commit on `terminal-ws`).
When 2a is done + green, execute **2d** on top of it.
