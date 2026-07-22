# Handoff — Terminal Slice 1c (render) + 1d (convergence) BUILD

Resume artifact to **drive the build** of the terminal workstream's remaining Slice 1
work. Planning is done, reviewed, and committed. Start a fresh session and execute.

## State (one line)

Slices 0/1a/1b are **done + cross-family-reviewed + merged** on `terminal-ws`
(**unpushed**). The **1c + 1d plans are authored, cross-family reviewed (codex/gpt-5.6 +
Opus, 15 findings folded incl. 5 Criticals), revised, Opus-verified, and committed**
(`f12a933`). Next = **build 1c → 1d** (subagent-driven). Nothing else is in flight.

## Read first

- **The two plans (execute these, don't re-plan):**
  - `docs/plans/2026-07-16-terminal-slice-1c-lens-terminal-render.md` (8 tasks, 1546 lines)
  - `docs/plans/2026-07-16-terminal-slice-1d-convergence.md` (9 tasks, 842 lines)
- **Design SSOT:** `docs/specs/2026-07-16-terminal-workstream-design.md` (render contract,
  threading/`Frame` seam, lifecycle, completion matrix). `docs/STATUS.md` ACTIVE block.
- **Memories (critical → skim):**
  - `[[gpui-test-noop-text-system]]` — **LOAD-BEARING.** `#[gpui::test]` fakes the text
    system; render tests run in a real `Application::new().run()` `harness=false` binary.
  - `[[terminal-render-ptyattach-spikes-executed]]` (render verdict + PTY contract),
    `[[grok45-as-plan-author]]`, `[[composer-delegation-profile]]`,
    `[[parallel-worktree-composer-delegation]]`, `[[whole-branch-review-needs-a-builder]]`,
    `[[xtask-gate-scope]]`, `[[terminal-spikes-process-learnings]]`.

## The recipe (same as 1a/1b)

`superpowers:subagent-driven-development`. Per task:
1. **Author/build:** delegate to **composer-2.5** via `cursor-delegate` (isolated worktree
   per slice if running work in parallel — but **1c → 1d is SEQUENTIAL: 1d needs 1c**).
   TDD, one commit per task deliverable (the plans give exact commit messages).
2. **Seam review:** ≥1 **cross-family** review per seam by a family ≠ author — codex
   (`codex exec -s read-only`, free gpt-5.6, stdin-piped prompt) or grok-4.5. Fold fixes.
3. **Whole-slice final:** fresh **Opus** review of the full slice diff before merge.
4. Merge to `terminal-ws`. Then start 1d.

## Gate (must be green before each merge — memory [[xtask-gate-scope]])

- `cargo fmt --all` + `cargo clippy --workspace --all-targets -- -D warnings` +
  `cargo test --workspace`.
- **macOS ADDITION for 1c:** xtask `gate` **executes** `cargo test -p lens-terminal
  --test render_realwindow` (the real-window `harness=false` perf/paint gate) — NOT
  `--no-run`. The plan wires this into `crates/xtask/src/main.rs`.
- **Never pipe clippy/gate through `tail`** (masks the exit code).

## The plans already bake in these committed mechanisms — DO NOT re-decide them

These were the review's 5 Criticals; the fix is load-bearing. If a build step seems to
contradict them, the build is wrong, not the plan:

- **1c C1/C5 — real-window test harness.** All text-system/paint/Menlo-gate/**perf**
  assertions live in ONE `crates/lens-terminal/tests/render_realwindow.rs`
  (`[[test]] harness = false`), driven from a real canvas paint callback. `cx.quit()`
  exits the process on macOS → single process, sequential phases, `std::process::exit(1)`
  on failure. Only pure logic (`resolve_cell_paint`, `row_needs_per_cell`,
  `RenderInspectShared`) stays under ordinary `#[test]`. **No `#[gpui::test]` for anything
  touching text/paint.**
- **1c I6 — one shared `TabRenderState`** (owns `latest_frame` + `cell_metrics` + the
  canvas-building code) embedded by both `TerminalTab` and the test host. No inert-`Client`
  exists; don't invent one.
- **1d C2 — foreground wake.** `open()` → `cx.spawn` continuation; blocking discover/attach
  → `cx.background_executor().spawn(blocking).await` → two-arg `weak.update(cx, …)`. Engine
  waker + bridge events only `try_send` onto `async_channel`; a foreground `cx.spawn`
  `futures::select!` sampler drains them and calls `weak.update(cx, …)`. **Never**
  `weak.update` from an OS thread; **`AsyncApp` is not `Send`.** Mirrors `lens-ui`
  `poller.rs` / `lens-app` `fleet_verify.rs`.
- **1d C3 — `TerminalRuntime` teardown.** Owns `Option<{bridge,attach,Arc<EngineHandle>}>`;
  foreground `take()`s it → background/detached teardown: join bridge → `attach.close()` →
  `Arc::try_unwrap(engine).stop()` (unique after bridge join). `Drop` offloads to a
  detached thread (never joins on the foreground).
- **1d C4 — reconnect-seed acceptance.** Parse the capture into legs; the leg-2 reconnect
  seed is `docs/spikes/captures/2026-07-15-pty-attach/reconnect.frames.jsonl:9`; compare
  full `Frame` (it's `PartialEq`); add a `VtEngine` `scrollback_rows` probe to assert the
  exact scrollback delta (viewport equality alone can't prove no history dup).

## Open risks the builder must watch (fail-closed — escalate, don't soft-pass)

1. **Menlo gate is a real hardware gate.** If system Menlo genuinely misaligns on the
   dev machine → STOP and reopen `lens-fonts` (bundle a font). Don't weaken the assert.
2. **PerCell 400×100 perf** may miss 8.3ms (spike ~16.5ms). 1c fail-closes vs an interim
   **20.0ms**; absolute 8.3ms @400×100 is explicitly **Slice 4**. Record real p95.
3. **Reconnect-seed** may reveal Ghostty duplicates history on clear+redraw into a retained
   engine → fail-closed, escalate (needs a design decision, not a silent pass).
4. **1d I13** — attach `set_inspect_enabled` is `cfg(test/test-util)` only; the plan commits
   to a production enable path (a tiny 1a change) or reads-without-toggling. Confirm which.
5. **⚠ perf caveat:** the spike's 2.77ms was paint-closure CPU only. 1c re-measures
   end-to-end in `render_realwindow` — treat THAT as the verdict.

## Exact next actions

1. New session → read the two plans + `[[gpui-test-noop-text-system]]`.
2. Build **Slice 1c** task-by-task (composer per task, TDD, cross-family seam review,
   Opus whole-slice final), gate green (incl. `render_realwindow` on macOS), merge to
   `terminal-ws`.
3. Build **Slice 1d** the same way (needs 1c landed). Live vertical proof (T9) needs a
   running omnigent 0.5.1 — see the `installing-omnigent-from-source` skill.
4. At session end: update `docs/STATUS.md` (memory `[[end-of-session-status-update]]`),
   consider pushing `terminal-ws` (user call — it's been unpushed across all of Slice 1).

## Process gotchas (from 1a/1b)

- Delegated gate MUST include `fmt --check`; codex review needs a stdin-piped prompt.
- cursor-delegate `capability: plan` returns text only; use `capability: write` to land
  files. Composer follows specs faithfully (incl. flaws) → the cross-family review earns
  its keep. `[[composer-delegation-profile]]`.
