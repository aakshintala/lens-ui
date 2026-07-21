# Handoff — Terminal Slice 2: EXECUTION (2026-07-17, SERIAL re-cut)

> **UPDATE 2026-07-21 — 2b DONE. NEXT = PLAN then execute 2c (mouse). A fresh session's first
> job is to WRITE THE 2c PLAN (2c is NOT yet planned), then execute it subagent-driven.**
> Slice **2b** (OSC-52 clipboard-write policy + Cmd+V paste) EXECUTED + DONE (2026-07-21) — 5 tasks,
> commits `018820b..df53fa3` (11 code/test + 1 STATUS doc). composer-2.5 per task + **codex `gpt-5.6-sol`
> high-effort** cross-family per-task reviews + fix waves + **Opus whole-slice = SHIP-WITH-FIXES → fixed**.
> Opus caught a cross-task **read-only-gate bypass** no single-task review could see: `dispatch_paste`
> didn't re-check `write_input_allowed()`, so a deferred multiline-warn paste Allowed AFTER a
> Write→ReadOnly downgrade egressed write bytes to a read-only terminal (the epoch layer doesn't cover a
> paste minted from the foreground `pending_pastes` queue post-downgrade) → FIXED (gate in `dispatch_paste`
> + regression test `deferred_paste_allow_after_readonly_downgrade_is_suppressed`, commit `57bccde`).
> Final full gate GREEN (fmt + workspace clippy + lens-terminal `test-util,live-tests` clippy + 132 lib
> tests + benches `--features bench` + demo build). Memory [[terminal-slice-2b-planned-reviewed]] (now the
> EXECUTED record). No live real-window rider run (paste round-trip is an env-gated manual leg; the Cmd+V
> intercept is hermetically proven by `real_cmd_v_keystroke_routes_to_paste_not_key_encoder` via a FIFO
> sentinel) — running it live is the user's call.
>
> **⚠ REVIEWER ROUTING CHANGED (user rule 2026-07-21, in CLAUDE.md):** cross-family reviews default to
> **`gpt-5.6`** and gpt-5.6 must **ALWAYS run via `codex exec -s read-only`, NEVER via `cursor-delegate`**
> ("Never use gpt-5.6 via Cursor — we have Codex for that"). On this box `codex` resolves to `gpt-5.6-sol`
> high-effort; capture stdout to a file (codex does NOT truncate the way cursor-delegate truncates long
> chat replies), the verdict is the last block from the `### Spec Compliance` header — `awk` from there.
> `cursor-delegate` gpt-5.6-sol-high is FORBIDDEN (also hit its monthly usage cap mid-2b; resets 7/25).
> Other diversity families (grok-4.5, gemini-3.5) still go via `cursor-delegate`. See [[codex-as-reviewer]],
> [[review-spend-policy]].
>
> _Prior state (for provenance): 2a (input) DONE 2026-07-17; Critical C2 (egress-replay) CLOSED via
> per-transport egress channels (`fd79d54..2921da8`); 2d (presentation) DONE 2026-07-20
> (`bdd8695..5e6f28b`), Opus whole-slice=SHIP + real-window gate PASSED. `terminal-ws` pushed to
> `origin/terminal-ws` through 2d; 2b commits are LOCAL/unpushed. No `main` merge — user's standing call._

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

## ✅ 2b (clipboard/OSC-52 write + Cmd+V paste) — EXECUTED + DONE (2026-07-21). See header.

Full slice `018820b..df53fa3` (11 code/test commits + 1 STATUS doc). 5 tasks (OSC-52 `on_clipboard_write`
cap-before-clone → foreground `ClipboardPolicy` seam + `ClipboardWriteRequest`/`ClipboardWriteNotice` +
`on_host_event` → `EngineCommand::Paste` bracketed engine-side never-drop/epoch → Cmd+V intercept
read-only-gated/multiline-warn/capped → demo Deny-default + benches + inspect + live rider). composer-2.5
per task + codex `gpt-5.6-sol` per-task reviews + fix waves + **Opus whole-slice = SHIP-WITH-FIXES → fixed**
(read-only-gate bypass on the deferred-warn paste; see header). Three documented deferrals carried in
`docs/STATUS.md`: always-warn-on-multiline (no foreground mode-2004 snapshot until 2c), menu Edit→Paste
not wired (only the keystroke path), empty-OSC-52 mints a prompt (intentional — could be a legit "clear
clipboard"). Memory [[terminal-slice-2b-planned-reviewed]] = full EXECUTED record; ledger has per-task detail.

## ▶ NEXT: PLAN then execute **2c** (mouse) — the LAST Slice-2 phase

**2c is NOT yet planned.** First action for a fresh session: **write the 2c plan** (superpowers:writing-plans;
plans→`docs/superpowers/plans/`; NOT `docs/superpowers` default — see [[spec-plan-location-convention]]),
cross-family review it (**gpt-5.6 via `codex`** — see the reviewer-routing note in the header), then execute
subagent-driven per the **"Execution flow for 2b/2c"** block above (composer-2.5 per task, codex per-task
reviews, gate after each incl. lens-terminal `--features test-util,live-tests`, Opus whole-slice at end).

**2c owns (surface + binding facts):** mouse capture built ONCE → {local selection + Cmd+C copy (moved here
from 2b), mouse-report to the PTY} + **XTSHIFTESCAPE arbitration** (selection-vs-report). It opens with the
**XTSHIFTESCAPE `mouse_shift_capture` safe-FFI spike** (the one still-open Slice-2 unknown). 2c builds the
first **foreground mouse-/terminal-mode snapshot** — which also unblocks 2b's deferred always-warn nuance
(suppress the multiline paste warn when bracketed paste / mode-2004 is active). Pixel→cell hit-testing +
motion coalescing live here too. See the design spec §2c + the parent completion matrix; read
[[terminal-realwindow-harness-pitfalls]] BEFORE any 2c real-window test. Merge `terminal-ws`→`main` and a
backup push of the local 2b commits both remain the **user's standing call**.
