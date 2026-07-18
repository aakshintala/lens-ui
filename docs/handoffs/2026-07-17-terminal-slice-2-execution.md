# Handoff — Terminal Slice 2: EXECUTION (2026-07-17)

**Self-contained driver for a fresh session** whose job is to **execute** Slice 2. Planning is
DONE and gpt-5.6-reviewed; do NOT re-plan or re-review the plans — execute them.

## State

- **Three plans, execution-ready**, in `docs/superpowers/plans/`:
  - `2026-07-17-terminal-slice-2-task0-foundation.md` — shared skeleton (lands FIRST).
  - `2026-07-17-terminal-slice-2a-input.md` — input/IME/focus/read-only.
  - `2026-07-17-terminal-slice-2d-presentation.md` — titles/hyperlinks/presentation egress.
- **Design spec (ground truth):** `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`;
  parent `docs/specs/2026-07-16-terminal-workstream-design.md` (matrix + Open-contract-gaps
  amended for the progress/notification defer).
- **Branch:** `terminal-ws` (pushed to `origin/terminal-ws`; **not** merged to `main` — user holds
  the whole terminal workstream on-branch). All Slice-2 planning artifacts are **uncommitted** on
  `terminal-ws` (the three plan docs + the parent-spec matrix amendment + CLAUDE.md gpt-5.6 edit).
  **Commit those planning docs first** (or fold into Task 0's first commit).
- **Durable context:** memories [[terminal-slice-2-design-ghostty-precedent]] (design + spike
  resolution), [[terminal-parallel-worktree-task0-foundation]] (why Task 0 + the single-writer rule),
  [[terminal-slice-1d-executed]], [[gpui-test-noop-text-system]], [[parallel-worktree-composer-delegation]].

## Execution order (STRICT)

1. **Task 0 (shared foundation) — FIRST, on `terminal-ws` directly (no worktree).**
   Mechanical, inert (egress rename + presentation channel + `Frame.cursor`/`FrameCell.hyperlink_uri`
   + `VtEngine` key fields + `TerminalTab` interaction fields + render hook-points, all no-op
   placeholders). **composer-2.5 executes** (it's a deterministic transcription — see
   [[plan-detail-vs-delegation-calibration]]); Opus reviews + runs the gate; commit. Acceptance =
   **compiles + existing suite green unchanged + clippy clean + every placeholder inert.**
2. **2a ∥ 2d — isolated git worktrees off the post-Task-0 `terminal-ws`** (the 1a∥1b pattern,
   [[parallel-worktree-composer-delegation]]). **subagent-driven-development** per each plan's header
   (composer-2.5 per task + per-task review). **Each worktree cross-family reviewed by a DIFFERENT
   family** (reviews now default to **gpt-5.6** — [[review-spend-policy]]; codex/gpt-5.5 = free fallback).
3. **Merge 2a + 2d → `terminal-ws`.** After Task 0 the merge is **additive** (each plan is a single
   writer to every shared definition; they touch disjoint lines of `build_frame`/`render`). Run the
   **full gate on the merged tree** regardless.
4. **Then 2b** (clipboard/OSC-52 policy — needs 2a + 2d's presentation egress; **owns** the
   `on_clipboard_write` registration + the cap-before-clone that 2d deliberately deferred), **then 2c**
   (mouse — opens with the **XTSHIFTESCAPE `mouse_shift_capture` safe-FFI spike**, still unresolved).
   Neither is planned yet.

## Why Task 0 (do not skip)

The gpt-5.6 review proved 2a and 2d are **not** independent: both edit `VtEngine::new`,
`WorkerChannels`/`spawn_worker`, `EngineHandle::spawn`, `build_frame`, and `TerminalTab::render`, and
each plan's struct literals omit the other's new fields (an **invisible** merge hazard — compiles in
isolation, drops a field on merge). Task 0 pre-declares every shared field/param/literal/hook-point as
inert placeholders so each plan is a **single writer** to disjoint bodies/lines. The plans are already
rebased onto this (each has a "Builds on Task 0" table + a handoff table of what it fills).

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

Commit the uncommitted Slice-2 planning docs, then execute **Task 0** (composer-2.5 → Opus review →
gate → commit on `terminal-ws`). Then set up the 2a ∥ 2d worktrees.
