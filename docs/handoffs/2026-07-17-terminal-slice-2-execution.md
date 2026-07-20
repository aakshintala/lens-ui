# Handoff — Terminal Slice 2: EXECUTION (2026-07-17, SERIAL re-cut)

> **UPDATE 2026-07-20 — 2a DONE + C2 CLOSED. NEXT = 2d.** Slice 2a (input) executed + DONE
> (2026-07-17). The deferred Critical **C2 (egress-replay across reconnect/downgrade) is now CLOSED**
> via per-transport egress channels + T4 hardening — full slice `fd79d54..2921da8`, plan
> `docs/superpowers/plans/2026-07-18-terminal-c2-per-transport-egress.md`. C1 (false EngineStopped)
> robust by-construction; C2 (reply-source) narrowed + **join-before-attach documented residual** (only
> needed if a live-bridge teardown path is ever added — see the C2 invariant comment in
> `TerminalTab::teardown_transport_off_foreground`). **A fresh session resumes at Slice 2d**
> (presentation), then 2b → 2c. gate-green, `terminal-ws` unpushed since C2 (merge = user's call).
> Cross-family reviews: gpt-5.6 endpoints stalled all session → grok-4.5 was used (user-approved).

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

## Progress (2026-07-17)

- **Serial re-cut DONE + reviewed** (commits `41902ed` + `ef93567`).
- **✅ 2a (input) EXECUTED + DONE** — commits `ef93567..78d8e0c` (incl. bridge panic fix `ec6f007`).
  All 6 tasks via composer-2.5 + per-task gpt-5.6 review + fix waves + a broad whole-slice review.
  **Gate green**: workspace fmt + clippy (no test-util) + lens-terminal clippy (test-util,live-tests) +
  77 lens-terminal + 162 lens-client lib tests + `tests/input_realwindow.rs` all 8 phases validated on a
  real macOS display (Tab→`\t`, Enter→`\r`, Shift+a→"A", ArrowUp→`\x1b[A`, all single-emit — no double-emit).
  Full per-task log + all review findings/adjudications: ledger `.superpowers/sdd/progress.md` (Slice 2 section).

## ✅ Critical **C2** — CLOSED (2026-07-20). Superseded; see header.

The 2a-slice review's deferred Critical (retained-engine egress survives the reconnect/downgrade
boundary, epoch-unrevocable) is **CLOSED** — full slice `fd79d54..2921da8`, plan
`docs/superpowers/plans/2026-07-18-terminal-c2-per-transport-egress.md`. The untyped shared `Vec<u8>`
egress became a swappable typed `Sender<EgressFrame>` owned by the worker (in-order `SetEgress`), each
bridge owning its own receiver, so emitted residue stays on the OLD channel (drain-dropped on stop);
plus an `access_epoch` bump on every teardown revokes un-encoded upstream input. C1 (false
EngineStopped) closed by-construction; C2 (reply-source) narrowed with a documented join-before-attach
residual (only live if a live-bridge teardown path is added). The Reconnecting-input-semantics decision
was resolved in 1d. **Nothing to do here — proceed straight to 2d.**

## NEXT: execute **2d** (presentation) on top of 2a

`docs/superpowers/plans/2026-07-17-terminal-slice-2d-presentation.md` — self-contained, lands on 2a's committed
code (declares its own presentation surface + `VtEngine::new` `presentation_tx` arity). Same flow:
subagent-driven-development, composer-2.5 per task, per-task cross-family review (a family *other* than 2a's
gpt-5.6 for diversity — grok-4.5 or gemini-3.5 via cursor-delegate), gate incl. `--features test-util`. Then 2b, 2c.

## First action (this re-cut session — DONE)

~~Commit the serial re-cut, then execute 2a task-by-task.~~ Completed — see Progress above.
