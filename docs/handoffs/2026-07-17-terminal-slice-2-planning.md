# Handoff — Terminal Slice 2: design DONE + reviewed, ready to plan (2026-07-17)

**Self-contained driver for a fresh session** whose job is `writing-plans` → build for
Slice 2 (interaction). The design is done and double cross-family reviewed; do NOT
re-brainstorm the architecture — plan against it.

## State

- **Spec (ground truth, review-clean):**
  `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` — read it fully first.
  It is subordinate to the parent workstream design
  `docs/specs/2026-07-16-terminal-workstream-design.md` (frozen requirements + public
  surface). Commits `79d7f90` (initial) → `9c569ea` (folded review) on `terminal-ws`,
  **pushed to `origin/terminal-ws`**.
- **Durable context:** memory `terminal-slice-2-design-ghostty-precedent` (the Ghostty
  input-threading precedent + the concrete `libghostty-vt` binding gaps). Slice 1 context:
  `terminal-slice-1d-executed`, `terminal-render-ptyattach-spikes-executed`,
  `gpui-test-noop-text-system`.
- **Branch:** `terminal-ws` (fast-forwards `main`; **not** merged — user is holding the
  whole terminal workstream on-branch). `main` is also 12 commits ahead of `origin/main`
  (lens-ui shell work), likewise unmerged/holding.

## The design in one breath

**Option A:** single-owner engine thread owns the non-`Send` `Terminal`; the foreground
lowers raw GPUI events into typed `Send` commands on **ONE ordered ingress+input stream**
(`Feed` VT + `Key`/`Mouse`/`Paste`/`Selection`/`Copy`/`Focus`/`LocalScroll`), processed
**strictly in arrival order, `Feed` chunked, no drain-ahead** — so a key encodes against
its stream-position modes (Ghostty-equivalent). Encoding + selection run on the owner
(they need `&Terminal`); the pure encoders (`key::Encoder`/`mouse::Encoder`/`paste::encode`)
take explicit `set_*` options → hermetic tests. Terminal-derived render state flows through
the immutable `Frame`; app overlays (IME preedit) draw foreground-side.

**Phases + order:** `2a` (input/IME/focus/read-only) ∥ `2d` (OSC output) → `2b`
(clipboard/OSC 52) → `2c` (mouse). 2a∥2d are the independent parallel pair (isolated
worktrees, each cross-family reviewed by a *different* family — the 1a∥1b pattern). 2b needs
2a **and** 2d's `EnginePresentationEvent` egress (OSC 52 arrives on it). 2c needs 2b+2a.

## Non-negotiables the plans MUST honor (review-caught; don't regress)

1. **Never block the GPUI foreground.** Foreground enqueues non-blocking; an off-foreground
   forwarder does any bounded blocking; `Stop`-severable so C3 teardown `take()` never
   stalls. Never-drop kept; bound memory via mouse-coalesce + paste-cap; explicit
   reject-with-marker fallback only if a hard ceiling is imposed.
2. **Ordered-stream fairness is a hard contract:** pinned max `Feed` chunk; `Stop`
   preempts `Feed` draining; test = 64 KiB `Feed` with an interleaved `Key`.
3. **Two-suite tests:** (a) pure-encoder mapping (no thread, explicit `set_*`); (b)
   engine-thread integration with a **command-id/oneshot ack = "bytes accepted by the test
   outbound receiver," `recv_timeout`** (deterministic). No sleeps / frame-polling for sync.
   Real-window harness for `InputHandler`/mouse hit-testing (`NoopTextSystem` false-greens).
4. **Read-only gate** = effective access ∧ `input_enabled`: suppress key/paste/mouse/
   **focus-report**; allow `Selection`/`Copy`/**`LocalScroll`**/resize.
5. **Format-aware mouse coalescing** (skip SGR-pixels; reset dedup on format/mode/button/
   policy change).
6. **`Copy`** async, capacity-1, cancellation-safe; foreground never `recv`s.
7. **OSC 52:** named decoded-byte cap applied *before* cloning the callback payload;
   preserve/reject `ClipboardWrite::location()` + MIME `contents()`; copy notice; test
   cap−1/cap/cap+1 + `?`-read → no host request.

## Two binding gaps — each phase OPENS with a spike (don't plan straight into UI)

- **2c — XTSHIFTESCAPE:** no safe `mouse_shift_capture` getter in `libghostty-vt`.
  Task 0 = a small safe-FFI accessor over the C state (investigate whether the C API
  exposes it). **Fallback:** config-only (never/always) arbitration + readable DEC modes,
  defer the terminal-requested override + amend the parent matrix.
- **2d — progress + notifications:** `Terminal::on_*` has **no** progress/notification
  effect; `osc::CommandType` tags carry no payload accessor. Task 0 = spike a payload-
  bearing effect/getter (may touch the `-sys`/C layer). **Fallback:** defer both features
  to a Slice-2 follow-up + amend the parent matrix. Titles (`on_title_changed`+`title()`),
  OSC 52 (`on_clipboard_write`), hyperlinks (`FrameCell` URI) are unaffected and proceed.

## Verification / house rules (unchanged)

- `xtask gate`: `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`,
  workspace tests. **For `lens-terminal`, per-task clippy MUST include
  `--features test-util` (and `live-tests`)** — 1d learned this the hard way (a
  `pub(crate)` break hid for ~6 tasks). Never pipe the gate through `tail`.
- Subagent-driven: composer-2.5 authors per-task TDD commits; ≥1 cross-family review per
  seam by a *different* family; Opus-inline review is a valid zero-codex-cost diverse pass.
- Ghostty reference: re-clone `ghostty-org/ghostty` @ `a887df42` into scratchpad if you need
  to re-verify a citation (the prior clone was ephemeral).

## Next action

Invoke `writing-plans` for **2a** and **2d** (the independent pair). Plan each against the
landed Slice-1 APIs (`crates/lens-terminal/src/{engine,bridge,policy,runtime}.rs`,
`lib.rs`) — the engine's `EngineCommand::Feed` channel is where the ordered input stream
extends. Do NOT reopen the Slice-1 engine threading contract.
