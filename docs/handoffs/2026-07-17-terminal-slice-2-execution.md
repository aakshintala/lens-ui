# Handoff — Terminal Slice 2: EXECUTION (2026-07-17, SERIAL re-cut)

> **UPDATE 2026-07-20 — 2a + C2 + 2d ALL DONE. NEXT = 2b (then 2c). Both 2b/2c still need PLANS.**
> Slice **2a** (input) DONE (2026-07-17). Critical **C2** (egress-replay) CLOSED via per-transport
> egress channels + T4 (`fd79d54..2921da8`). Slice **2d** (presentation) EXECUTED + DONE (2026-07-20):
> titles/OSC-8 hyperlinks/click→OpenUrlRequest/inspect+benches, 6 tasks composer-2.5 + grok-4.5
> per-task reviews + fix waves + **Opus whole-slice = SHIP**. Opus caught a title-clear-vs-full-channel
> invariant divergence (per-task reviews couldn't see it) → FIXED via a **tri-state latest-title slot**
> (`Set|Clear` authoritative; drain never falls back to the droppable channel). The **end-of-slice
> real-window gate PASSED** on a real macOS display and caught 3 latent bugs in the never-executed
> Task-4 harness (frame-clobber-by-sampler / sync-read-of-async-`cx.emit` / dropped `Subscription`) —
> all fixed; production click path was correct. Full 2d slice `bdd8695..5e6f28b`. **`terminal-ws`
> PUSHED to `origin/terminal-ws` through 2d** (backup; **no `main` merge — user's standing call**).
> Cross-family reviews used **grok-4.5** (`cursor-grok-4.5-high`) — gpt-5.6 endpoints stall; grok is the
> proven 2d/C2 reviewer. Memories [[terminal-slice-2d-executed]], [[terminal-realwindow-harness-pitfalls]]
> (READ THIS before any 2b/2c real-window test). **A fresh session's first job: WRITE THE 2b PLAN**
> (see "Then 2b" below), then execute it subagent-driven, then plan+execute 2c.

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
- **Branch:** `terminal-ws` (pushed to `origin/terminal-ws` **through 2d** @ `004d344`; **not** merged
  to `main` — user holds the whole terminal workstream on-branch). 2a + C2 + 2d all committed + pushed.
  Ledger `.superpowers/sdd/progress.md` (gitignored scratch, on-disk recovery map — full per-task
  history for 2a/C2/2d).
- **Durable context:** memories [[terminal-slice-2-design-ghostty-precedent]] (design + spike
  resolution), [[terminal-parallel-worktree-task0-foundation]] (**superseded** — records why Task 0
  was needed for the PARALLEL structure; serial dropped it), [[terminal-slice-1d-executed]],
  [[gpui-test-noop-text-system]], [[plan-detail-vs-delegation-calibration]],
  [[composer-delegation-profile]], [[premature-layer-boundary-binding]].

## Execution order (STRICT, serial on `terminal-ws` — no worktrees, no merge)

1. ~~**2a (input) — FIRST.**~~ ✅ **DONE** (2026-07-17, `ef93567..78d8e0c`). Gate-green,
   real-window keystroke-validated.
2. ~~**2d (presentation) — SECOND.**~~ ✅ **DONE** (2026-07-20, `bdd8695..5e6f28b`). All 6 tasks +
   grok-4.5 per-task reviews + fix waves + Opus whole-slice SHIP; tri-state title fix; real-window
   gate passed. Memory [[terminal-slice-2d-executed]].
3. **▶ 2b (clipboard/OSC-52) — NEXT, needs a PLAN first.** Owns what 2d deliberately deferred:
   re-thread `presentation_tx` into an `on_clipboard_write` registration at `VtEngine` construction
   (2d left a note at `vt.rs` ~L97 that 2b re-threads it; the title clone may already consume one
   clone); the **cap-BEFORE-clone** on decoded OSC-52 bytes (drop/deny over-cap before allocating owned
   MIME copies); allow/deny + allow-once/session policy + copy notice + cap−1/cap/cap+1 tests; map
   drain → `TerminalEvent::ClipboardWriteRequest`. 2d already declared the
   `EnginePresentationEvent::ClipboardWrite { location, contents }` variant + `ClipboardLocation` +
   `ClipboardMimePart` — 2b fills the registration + policy. **Binding facts:** OSC-52 IGNORES callback
   results (`terminal.rs` ~L1345 — do NOT claim `Busy` backpressure); the demo must **Deny/no-op**, never
   auto-allow. Ground truth: `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` (OSC-52
   policy). **Write the plan** (superpowers:writing-plans; specs→docs/specs, plans→docs/superpowers/plans
   per [[spec-plan-location-convention]]), review it, then execute subagent-driven.
4. **Then 2c (mouse) — LAST, needs a PLAN.** Opens with the **XTSHIFTESCAPE `mouse_shift_capture`
   safe-FFI spike** (still unresolved — resolve it first). Mouse reporting / selection / paste.
   Not planned.

**Execution flow for 2b/2c** (same as 2a/2d): **subagent-driven-development** — composer-2.5 per task,
**grok-4.5 (`cursor-grok-4.5-high`) cross-family per-task reviews** (gpt-5.6 endpoints stall; grok is
proven), fix waves for Critical/Important, gate after each task incl. lens-terminal
`--features test-util,live-tests`, then an **Opus whole-slice review** at the end. **Any real-window
test: READ [[terminal-realwindow-harness-pitfalls]] FIRST** — 2d's never-executed harness hid 3 bugs
(feed frames THROUGH the engine, poll `cx.emit` across renders, HOLD the `Subscription`; the RUN is the
only proof). Run real-window gates only with a **user heads-up** (opens macOS windows).

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

## ✅ 2d (presentation) — EXECUTED + DONE (2026-07-20). Superseded; see header.

Full slice `bdd8695..5e6f28b` (10 code commits). 6 tasks (presentation channel + latest-title slot;
OSC title sanitize→`reported_title`; `FrameCell.hyperlink_uri` OSC-8 extraction; click→`OpenUrlRequest`
+ `url::Url` validation; OSC-52 declared-only/deferred-to-2b; inspect counters + benches) —
subagent-driven, composer-2.5 per task + grok-4.5 per-task reviews + fix waves + **Opus whole-slice =
SHIP**. Opus caught a title-clear-vs-full-channel invariant divergence → FIXED via a **tri-state
latest-title slot** (`3cfc270`). **Real-window gate PASSED** on a real display (`presentation_realwindow`
click→OpenUrlRequest e2e + `render_realwindow` perf all in-budget) and caught 3 latent harness bugs
(fixed, `5e6f28b`; production correct). Memories [[terminal-slice-2d-executed]] (full record) +
[[terminal-realwindow-harness-pitfalls]].

## ▶ NEXT: PLAN then execute **2b** (clipboard/OSC-52), then plan+execute **2c** (mouse)

Neither is planned. See **Execution order item 3 (2b)** and **item 4 (2c)** above for the surface each
owns + binding facts. **First action for a fresh session: write the 2b plan** (superpowers:writing-plans;
plans→`docs/superpowers/plans/`), cross-family review it (grok-4.5), then execute subagent-driven per the
**"Execution flow for 2b/2c"** block above. Merge `terminal-ws`→`main` remains the **user's standing call**.
