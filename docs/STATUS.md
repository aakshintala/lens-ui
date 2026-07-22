# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

_Last curated 2026-07-21 (terminal Slice **2c (mouse) DONE** on `terminal-ws`; **`main` merged INTO `terminal-ws`** to keep the long-lived terminal branch current — `main` itself untouched until the terminal workstream lands. Board B-3 group chrome & rollups SHIPPED on `main` UNPUSHED; B-4 drag/move + write-path next.)_

---

## Next up

- **▶ ACTIVE: shared terminal workstream — Slices 0/1a/1b merged; 1c DONE; 1d COMPLETE (live-proven); Slice 2 SERIAL: 2a (input) DONE + C2 CLOSED, 2d (presentation) DONE + real-window-gated, ▶▶ 2b (clipboard/OSC-52 + Cmd+V paste) EXECUTED + DONE (2026-07-21): all 5 tasks (OSC-52 on_clipboard_write cap-before-clone → foreground ClipboardPolicy seam + ClipboardWriteRequest/Notice + on_host_event → EngineCommand::Paste bracketed engine-side never-drop/epoch → Cmd+V intercept read-only-gated/multiline-warn/capped → demo Deny-default + benches + inspect + live rider), each codex-gpt5.6-reviewed + fix waves, Opus whole-slice = SHIP-WITH-FIXES (caught a cross-task read-only-gate bypass on the deferred-warn paste path — dispatch_paste now gates, regression-tested) → fixed. Full 2b slice `018820b..57bccde` (11 commits). FINAL FULL GATE GREEN (fmt + workspace clippy + lens-terminal test-util,live-tests clippy + 132 lib tests + benches + demo). `terminal-ws` unpushed (backup push + `main` merge = user's call). ▶▶ 2c (mouse) **DONE (2026-07-21)** — full slice `f1922c5..c924fd9`. T5 fg lowering (thin, zero mode logic; immediate forward; `SetAccess`-wired on open AND teardown) → T6 mouse-local toggle + arbitration goldens → T7 coalesce-reset + per-mode motion + benches. **Whole-slice codex review = 9 findings, ALL folded** (F1 wheel epoch-recheck, F2 LocalClick click-time-frame **token correlation**, F3 notch-cap/overflow, F4 Any-motion local-policy, F5 Button-mode latched-button, F6 coalesce tracking-toggle reset, F7 multi-button latch guard, F8 jitter-click via `gesture_dragged`, F9 honest LocalClick-drop). **Re-review = 3 more** (Re-1 pre-egress epoch recheck FIXED; Re-2 F2-token FIXED per user; Re-3 F6-no-move-toggle documented residual). Also root-caused a suite flake (test-only worker stall gate busy-spun → starved build workers → now sleeps). **T8 DONE:** `mouse_realwindow` real-window proof (4 phases: localclick/select+copy/report/read-only PASS) + live **P6** mouse-report round-trip vs omnigent 0.5.1 (`LENS_LIVE_MOUSE_REPORT=1`, PASS) + both specs updated (DP3 engine-side; XTSHIFTESCAPE RESOLVED-DEFERRED). **DP3 AMENDED:** arbitration/latching/coalescing engine-side at ordered-stream position; `Frame` mode hint rejected. **176 lib tests, workspace+test-util clippy clean, fmt, benches compile, stable 8/8.** Plan `docs/superpowers/plans/2026-07-21-terminal-slice-2c-mouse.md`. **NEXT — remaining slices RESHAPED 2026-07-21 (design/grilling pass; design spec Build-sequence revised, memory [[terminal-slice-3plus-replan]]):** old 2-slice tail (Slice 3 lifecycle&fleet → Slice 4 perf) is superseded by **3 → 4 → 5 → 6**: **Slice 3** byte-accounting (thin per-tab retained-bytes *estimate*) + perf acceptance (demo-hosted, thin multi-tab spawner); **Slice 4** lifecycle *mechanisms* (full generation guard, Sleep/wake teardown, `ReplacementWaiting`; `Ended` **inert** — no 0.5.1/**0.6.0** termination signal, verified; module/demo, host-agnostic); **merge `terminal-ws`→`main`** after 3+4 (pure `lens-terminal`+demo, low-risk — first terminal landing on main); then on a fresh branch off main: **Slice 5** lens-ui **minimal** `FleetStore` membership + fleet policy (memory-pressure LRV trim/disconnect, when-to-sleep, `session.superseded` as sub-slice **5-super** lens-core-first) + **Slice 6** full production terminal surface + **E2E-in-app** (old "lens-ui integration out of scope" deliberately expired). Byte-*accurate* FFI = fail-closed conditional (no C-ABI accessor + compressed scrollback; escalate only if the estimate is ordinally unreliable — RSS covers absolute budget). Demo = permanent module isolation/perf rig. **▶▶ SLICE 3 (byte-accounting + perf) DONE (2026-07-22)** — full slice `af0b605..f30f894` on `terminal-ws`, `xtask gate` GREEN. `EngineInspect.total_rows`+`retained_bytes_estimate` (sampled per build; `PER_CELL_BYTES` provisional 4, ordinal-only). **Job A** `stream_perf_realwindow` real-GPUI (in macOS gate): paint_p95 3.1 ms / build_p95 0.57 ms under sustained 4-tab streaming, hidden-tab suppression asserted. **Job B** `rss_probe` bin + `xtask terminal-rss-sweep` (out-of-gate acceptance): 1k–50k rows × {compressible,incompressible} → **estimate ordinally reliable, NO flips → byte-accurate FFI conditional NOT triggered.** **Two real bugs found by Job B + fixed:** (1) 64 MiB worker stack (`67e8192`) — libghostty scrollback overflowed the ~2 MiB default at ~2000+ rows (real product crash); (2) `max_scrollback` is a BYTE budget, not lines (vendored doc wrong). Memory [[terminal-max-scrollback-bytes-and-worker-stack]]. Plan `docs/plans/2026-07-22-terminal-slice-3-byte-accounting-perf.md`; handoff `docs/handoffs/2026-07-22-terminal-slice-3-executed.md`. **Immediate action:** author the **Slice 4** (lifecycle mechanisms) plan (fresh session); after 4, **merge `terminal-ws`→`main`** (first terminal landing).
    - **▶ SLICE 2b (clipboard/OSC-52 write + Cmd+V paste) — DONE (2026-07-21).** RE-CUT per the
      2026-07-20 spec amendment to **OSC-52 output-clipboard write policy + Cmd+V paste ONLY**
      (selection + Cmd+C copy MOVED to 2c — they share 2c's mouse-capture stack + XTSHIFTESCAPE
      arbitration). Policy state is session-scoped behind an injectable **`ClipboardPolicy`** trait
      (in-memory `SessionClipboardPolicy` now; `lens-ui` injects a persisted impl later). Executed
      subagent-driven: composer-2.5 per task + **codex `gpt-5.6-sol` high-effort** cross-family
      per-task reviews (user rule 2026-07-21: gpt-5.6 always via codex, never Cursor) + fix waves +
      Opus whole-slice. Security-critical ordering all in place: cap-before-clone (1 MiB, before any
      owned alloc), post-encode epoch-recheck (covers empty output), reject-not-truncate paste (1 MiB),
      OSC-52 reads never reach the callback, read-only gate on BOTH the immediate and deferred-warn
      paste paths. Plan `docs/superpowers/plans/2026-07-20-terminal-slice-2b-clipboard-paste.md`.
      **Documented deferrals (carried, not bugs):** (1) **always-warn-on-multiline** — the foreground
      has no live mode-2004 (bracketed-paste) snapshot until 2c, so 2b uses the safe over-approximation
      (warn on any multiline, suppressible via "don't warn again"); suppressing the warn while bracketed
      paste is active is deferred to 2c. (2) **Menu Edit→Paste (`OsAction::Paste`) not wired** — only the
      Cmd+V keystroke path is intercepted (`handle_key_down`); the app-menu paste command is a distinct
      gpui path deferred to `lens-ui` menu integration (the standalone demo has no app menu). (3) **empty
      OSC-52 write still mints a host prompt** (writes empty string on Allow) — left intentionally, since
      suppressing could drop a legitimate "clear clipboard" intent; the bounded pending map rate-limits nag.
      **Live paste rider PASSED (2026-07-21)** — `terminal_live` P5 round-trip against a live omnigent
      0.5.1 shell (ephemeral rider-shell bundle, zero LLM cost): real clipboard→bracketed-encode→PTY→shell
      echo→frame paint on a real macOS display (`terminal_live: P5 paste round-trip OK` / `PASS`). P5 drives
      the production `handle_paste` path; the Cmd+V OS-keystroke intercept stays hermetically proven by
      `real_cmd_v_keystroke_routes_to_paste_not_key_encoder` (FIFO sentinel). 2b is now hermetically + live-proven.
  - **▶ SLICE 2 (interaction) — RE-CUT TO SERIAL; 2a + C2 + 2d DONE (2026-07-20).** 2d (presentation)
    executed subagent-driven (composer-2.5 per task + grok-4.5 cross-family per-task reviews + fix waves +
    Opus whole-slice = SHIP); the Opus review caught a title-clear-vs-full-channel invariant divergence
    (per-task reviews structurally couldn't see it) → FIXED via tri-state latest-title slot. The end-of-slice
    real-window gate (run on a real macOS display) caught 3 latent bugs in the never-executed Task-4 harness
    (frame-clobber-by-sampler, sync-read-of-async-emit, dropped Subscription) → all fixed; production click path
    was correct. Both `presentation_realwindow` (click→OpenUrlRequest e2e) and `render_realwindow` (perf all
    in-budget; C2-era over-budget flag confirmed environmental) green. 2d slice `bdd8695..5e6f28b`. Design spec
    [`docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`](./specs/2026-07-17-terminal-slice-2-interaction-design.md).
    **Task 0 DELETED — dissolved into 2a/2d** (serial removes the parallel merge hazard at the root; 2d edits 2a's
    committed code). Plans: `docs/superpowers/plans/2026-07-17-terminal-slice-2-{2a-input,2d-presentation}.md`.
    Phases **2a input/IME/focus/read-only → 2d OSC-output → 2b clipboard/OSC52 → 2c mouse**, serial on `terminal-ws`.
    **2a DONE**: all 6 tasks via composer-2.5 + per-task gpt-5.6 review + fix waves + broad-slice review; gate-green
    (both clippy configs, 77 lens-terminal + 162 lens-client tests, real-window keystroke validated). Reviews caught
    real bugs each task (macOS option-as-alt clobber, reversed epoch-revoke, Tab/Enter/Shift double-emit, worker
    epoch TOCTOU, reply-evicts-user-egress). **✅ Critical C2 CLOSED (2026-07-20)** via **per-transport egress
    channels** (`fd79d54..2921da8`; plan `docs/superpowers/plans/2026-07-18-terminal-c2-per-transport-egress.md`):
    the shared untyped `Vec<u8>` egress became a swappable typed `Sender<EgressFrame>` owned by the worker
    (in-order `SetEgress`), each bridge owning its own receiver → emitted residue stays on the OLD channel
    (drain-dropped on stop); + `access_epoch` bump on every teardown revokes un-encoded upstream input.
    **T4 hardening (option A):** grok/Opus reviews diverged → source-verified adjudication (Opus GO correct;
    grok's Criticals need a live old bridge, unreachable today since every teardown = bridge self-stop). C1
    (false EngineStopped) closed BY-CONSTRUCTION (`signal_stop` synchronous-before-swap + Disconnected-arm
    suppression); C2 (reply-source) NARROWED (inbound-arm re-checks `stop` before feeding) + **join-before-attach
    documented residual** (only needed if a live-bridge teardown path is ever added, e.g. `on_host_event` →
    server downgrade). Reconnecting-input-semantics decision resolved in 1d. Ledger `.superpowers/sdd/progress.md`.
    Architecture = Option A (single-owner engine + ONE ordered command stream). **Progress+notifications spike-resolved
    bucket-C (absent from pinned C ABI) → DEFERRED + parent matrix amended.** Plans: grok authored → Opus review →
    consolidated **gpt-5.6-sol-high** review ($1.67; 8 Critical + 11 Important) → folded + verified. **Merge-seam
    finding resolved via Task 0** (single-writer foundation; both plans collide on `VtEngine::new`/`WorkerChannels`/
    `build_frame`/`render` — see [[terminal-parallel-worktree-task0-foundation]]). **XTSHIFTESCAPE (2c) still open.**
    Memory [[terminal-slice-2-design-ghostty-precedent]]. **Execution handoff (self-contained):
    [`docs/handoffs/2026-07-17-terminal-slice-2-execution.md`](./handoffs/2026-07-17-terminal-slice-2-execution.md).**
  - **✅ SLICE 1d (convergence) — ALL CODE DONE (T1–T9), gate GREEN, AND live rider PASSED, subagent-driven (composer-2.5 build +
    cross-family review: Opus inline per-task + codex on T4 reconnect machine + codex whole-branch). 17 commits
    `f4d3080..39ee7b3` on `terminal-ws` (unpushed). Ledger `.superpowers/sdd/progress.md`.**
    - **Shipped:** bridge thread (Select multiplex attach↔engine, OutboundSaturated/AttachDisconnected/
      EngineStopped policy events); `TerminalRuntime` off-foreground teardown (panic-free unique-Arc stop);
      `open()` C2 background discover/attach + foreground `cx.spawn` wake sampler (durable policy_tx survives
      reconnect); close-code policy + single 30s `RetryWindow` + reconnect state machine (read-only downgrade
      → read-only reconnect; preflight GET; transient-retryable); resize-before-input on connect+reconnect;
      retained-engine reconnect-seed acceptance (viewport Frame equality + fail-closed scrollback delta);
      `TerminalInspect`; standalone GPUI demo (handshake-before-GPUI); live rider harness (real-window,
      4-phase: attach→painted-marker→abort→Reconnecting→reattach+output_gap).
    - **Reviews earned their keep:** codex T4 caught 2 Critical (reconnect race + foreground-block) + 4
      Important → hardened (foreground bridge-spawn eliminates the race; single window; read-only reconnect).
      codex whole-branch caught 6 Important integration issues (engine-death stuck-Live; initial resize
      skipped; rider false-green; abort machinery leaking into production; inspect not durable across
      reconnect) → all folded.
    - **✅ LIVE RUN DONE (2026-07-17) — rider PASSED all 4 phases vs omnigent 0.5.1** (`tests/terminal_live.rs`,
      `--features live-tests,test-util`, real GPUI window): P1 attach→Live · P2 `echo LENSMARK_<pid>\r` to a
      real **zsh** PTY + proved a PAINTED frame carried the marker · P3 abort-attach→Reconnecting · P4
      reattach→Live+`output_gap`. **Both OMNIGENT-OPERATIONAL blockers from the prior attempt resolved (never
      rider bugs):** (1) the design Q is ANSWERED — a declared `terminals:` entry spawns a real tmux-backed
      bare shell (default `command`) SEPARATE from the agent TUI (`runner/app.py:16228`), gated server-side on
      the agent spec's `terminals:` block (`sessions.py:17565`); no billable turn; (2) the recipe = an ephemeral
      `rider-shell` agent bundle declaring `terminals.shell.command: zsh`, launched `omnigent run <bundle>
      --server` (spawns its own runner, `sleep`-held stdin keeps the REPL alive), rider `open()` open-or-creates
      the declared PTY. Full recipe + yaml-shape gotchas in memory [[omnigent-terminal-attach-live-run]]; the
      throwaway session was torn down, server+host daemon left online for future riders. Handoff
      [`docs/handoffs/2026-07-17-terminal-slice-1d-live-run.md`](./handoffs/2026-07-17-terminal-slice-1d-live-run.md).
      Merge `terminal-ws` → user's call (staying on branch through the rest of the workstream).
    - **Deferred (roll-up in ledger):** 1b engine flake `hidden_tab_suppresses_publish_until_visible`
      (pre-existing, timing under parallel load); EngineHandle::spawn readiness result (1b); thread-exhaustion
      foreground-panic extreme edge; per-task lens-terminal clippy should include `--features test-util`
      (missed T3's render_realwindow break for ~6 tasks).
  - **✅ SLICE 1c (render) — DONE, `xtask gate` GREEN on `terminal-ws` (unpushed).** T1–T7 correctness
    (`874c817`→`ae12b8b`) + perf-block resolution (`63f490f`→`f4f0d15`). Real-window harness
    (`tests/render_realwindow.rs`, `harness=false`, `test-util`-gated, xtask executes on macOS **in
    `--release`**): **Menlo gate PASSES**; `Frame` paint (PerRow/PerCell routing + debug-guard that
    PerRow never gets a wide cell), full SGR + `underline_quad_color` (I10a) + invisible-width (I10b),
    render Inspect ring; `TerminalTab` renders via shared `TabRenderState::render_element` (I6). Clippy
    clean `-D warnings` (normal + `test-util`).
    **2 plan deviations (both user-approved, in commit msgs):** (1) `render_test_api` gated on
    `test-util` not `cfg(test)`; (2) Menlo gate drops the emoji's-own-left-edge probe (real hardware:
    ASCII grid/box-drawing/CJK/post-emoji-resync all exact; emoji left edge drifts 2.857px under
    per-row shaping, which the renderer never uses for emoji — wide rows → per-cell).
    **⛔→✅ The "per-cell perf block" was a DEBUG-BUILD ARTIFACT.** The gate measured the harness in
    **debug** (~5.4× slower per-cell than release, from unoptimised `Font` clones). The 8.3ms budget is
    a 120fps *product* target; in **release** (what ships) every workload meets it: dense-wide 200×50
    **2.5–3.7ms**, 400×100 **4.8–6.2ms** (beats the *absolute* 8.3, not just the 20ms interim),
    pathological **2.4–3.3ms**. Shaping is ~0.06ms — the recommended per-glyph shape cache / **C-a
    reopen is RETIRED** (never the bottleneck). Fixes: gate now runs `--release` (`b1cc3e2`);
    resolve-once-per-row cleanup (`ad7e049`; **codex/gpt-5.5 review caught a per-cell decoration
    paint-order flip → fixed `730fc83`**); release-calibrated per-phase budgets (ascii 3.0 / wide-200
    5.5 / wide-400 8.0 / pathological 6.0ms — carry ~30% gate-load margin so no flap). Plan:
    `docs/plans/2026-07-16-terminal-slice-1c-perf-resolution.md`.
    **✅ Engine gate flake FIXED.** Both `engine::handle` flakes
    (`build_failure_retries_on_next_pump` + `stop_publishes_final_frame_before_join`) shared ONE root
    cause: a process-global `static TEST_BUILD_FAILURES` the fault-injection test set, which any
    concurrent test's worker consumed (its own build then wrongly succeeded; bystanders lost a frame).
    Moved injection to a per-handle `Arc<AtomicUsize>` shared only with that handle's worker — no
    cross-test contamination, zero-cost in production. 120/120 clean stress runs.
  - **✅ SLICE 0 (surface freeze) DONE + merged** (`fdba839`→`635eaa7`): froze the opaque public
    names/seam invariants `lens-ui` binds to (`open`/`TerminalTarget`/`AccessIntent`/`TerminalKey`/
    `TerminalOpenOptions`, 7-variant `Lifecycle`, `TerminalHostEvent`/`TerminalEvent`, opaque `Frame`,
    `focus_handle`/`presentation`) + `lens-terminal` & `lens-terminal-demo` crate skeletons. **codex
    (gpt-5.5) review** caught 3 evolvability issues, folded: `Lifecycle` = permanent payload-free
    `Copy` tag (detail rides `Presentation`); `TerminalOpenOptions` `#[non_exhaustive]`+`with_*`
    setters; `Frame` dropped `Clone`/`Default` (shared as `Arc<Frame>`).
  - **✅ SLICE 1a (`lens-client` transport) DONE + merged** (branch `terminal-1a`, 8 commits
    `0f7f23a`→`9e7b16f`): typed REST CRUD (`Terminals` subservice — **superseded** the dead
    Value-leaking `create_terminal`/`delete_terminal`/`transfer_terminal` wrappers, no callers), WS
    attach on a **contained tokio current-thread runtime + tokio-tungstenite** bridged to sync via
    bounded crossbeam queues (NO `transport=`), typed frame codec, close-code **classification**
    (4404/4405/4500/1008; policy deferred to 1d), gated `AttachInspect` ring, benches, feature-gated
    live rider (create/attach/input/resize/delete + 4404). **gpt-5.6-sol review** caught 6 real
    issues, all fixed (`9e7b16f`): `close()` deadlock on outbound saturation, unbounded connect/close,
    **inbound-saturation `Closed` signal lost** (now drops the Sender → guaranteed `Disconnected`),
    **queue bench deadlock**, `bench_api` `Message` leak (now typed), silent runtime-build failure.
    162 lib tests. Plan `docs/plans/2026-07-16-terminal-slice-1a-lens-client-transport.md`.
  Original design ([`specs/2026-07-14-terminal-workstream-design.md`](./specs/2026-07-14-terminal-workstream-design.md))
  assumed **porting Ghostty VT source** via the gpui-ghostty wrapper (adopt/adapt/exclude inventory,
  WP0 provenance gate). **That model is now superseded.** This session:
  - **WP0 plan revised** through Opus review (B1–B7) + **5 rounds of gpt-5.6-sol** review, committed
    (`73738d5`); then **executed Tasks 1–4** subagent-driven (composer-2.5 + cross-family):
    xtask verifier + CLI (17 tests, `db5a0b4`→`354d405`), archive hashes (`d9b2194`), adoption
    inventory 45+742 (`5bb16ec`). **These are now built on the obsolete model — repurpose/discard.**
  - **Task 5 (Zig build probe) hit a macOS-26 wall → resolved:** vanilla ziglang.org Zig ≤0.15.2 can't
    link natively on macOS 26 (Xcode 26 bug, ziglang/zig#31658); the **Homebrew/Nix patched `zig@0.15`
    (0.15.2)** works. Ghostty **v1.3.1** `lib-vt` builds natively with it. Memory
    [[zig-ghostty-macos26-scissor]].
  - **DECIDED MODEL (memory [[terminal-vt-adoption-model]]), PROVEN on hardware:** the terminal C API
    (`terminal.h`/`screen.h`/`render.h`/`grid_ref*.h`) lives ONLY on Ghostty **dev** (release v1.3.1
    `vt.h`=7 parser headers; dev `a887df42`=29 incl. the full terminal surface). **[libghostty-rs]**
    (Uzaaft, MIT/Apache) already binds it — `libghostty-vt-sys` (checked-in bindings) + `libghostty-vt`
    (safe `Terminal`/`vt_write`/`RenderState`/`Cell`/scrollback), pinning Ghostty dev in `build.rs`.
    **VERIFIED:** builds on macOS 26.6 w/ patched `zig@0.15` in 33s, 29 tests pass, example drives a
    real terminal. **Model = VENDOR libghostty-rs + BUILD FROM SOURCE** (patched zig prereq;
    ~25-33s cached build) — NOT a shim, NOT a source port, NOT prebuilt (no CI yet; prebuilt =
    flip-a-switch later). gpui-ghostty = reference only. WP0's provenance apparatus collapses to
    **dependency vetting** (pin libghostty-rs + Ghostty commit + license closure).
  - **✅ MECHANICAL EXECUTION DONE (2026-07-15, this session)** — commits `ae1f385`/`014f9a9`/`e155230`
    on `terminal-ws` (unpushed):
    - **Task 2 — vendored + wired + link-proven.** `vendor/libghostty-rs/` (2 crates @ `46a9d2ac`,
      684K); the Ghostty source is **crates-only, NOT vendored** — reversed the "pinned vendored
      Ghostty source tree" plan: its ~57-152M tree is the same large-artifact-before-CI anti-pattern
      we reject for a prebuilt `.a`, so the pin stays in `build.rs` (`a887df42`, blobless fetch,
      cached) and a `GHOSTTY_SOURCE_DIR` vendor is deferred to the **same CI trigger** as prebuilt.
      Wiring: crates **EXCLUDED** (not member) → Cargo cap-lints them (clippy `--workspace -D warnings`
      stays clean); `.cargo/config.toml` `ZIG`→keg-only `zig@0.15` + a 1-line `build.rs` patch. Proof:
      `spikes/libghostty-link` (bytes→cell); from-source build re-verified post-`cargo clean` (24.94s).
      Provenance + patch list: `vendor/libghostty-rs/README.md`. Memory [[terminal-vt-vendored-executed]].
    - **Task 1 — dead WP0 apparatus removed.** xtask terminal-provenance CLI/lib/tests/fixtures +
      toml/thiserror/sha2 deps; `vendor/gpui-ghostty-e3025981/`; generate-terminal-adoption.sh; WP0
      plan+review docs. codegen/drift/gate intact (`cargo test -p xtask` 2/2).
    - **Docs superseded** — banners on the source-port design + roadmap (VT-adoption + `--workspace`
      gate lines flagged dead; model-independent parts still hold).
  - **✅ DESIGN-PASS SPIKES DONE (2026-07-16, this session)** — both design questions answered;
    merged to `terminal-ws` (unpushed). Spec `docs/specs/2026-07-15-terminal-spikes-design.md`,
    plans `docs/plans/2026-07-15-terminal-spike-{a,b}-*.md`. Memory
    [[terminal-render-ptyattach-spikes-executed]].
    - **Spike A — render viability → VERDICT: full-snapshot repaint contract.** Standalone GPUI
      probe (grok-built, Opus+codex reviewed). S1 (reshape every row every frame, no cache) full-redraw
      p95 = **2.77 ms @ 200×50** ≤ 8.3 ms budget → Ghostty dirty-row tracking is **not** load-bearing;
      per-row `ShapedLine` cache (S2) barely helps (2.45 ms) → shaping isn't the bottleneck. Wide/emoji
      need per-cell glyph placement (per-row `shape_line` drifts). Liftable `paint.rs` kept +
      codex 3-item punch-list (findings `docs/spikes/2026-07-15-terminal-render-viability.md`).
      ⚠ p95 is paint-closure CPU only (no vsync/present) — re-measure end-to-end when building real.
    - **Spike B — PTY-attach contract → DOCUMENTED + LIVE-VERIFIED** vs omnigent 0.5.1
      (`docs/spikes/2026-07-15-pty-attach-contract.md`, corpus in `captures/2026-07-15-pty-attach/`).
      **Wire is transport-independent** (control=default & pty both deliver **raw VT binary**; tmux
      control-mode consumed server-side → **NO tmux parser in the client**). Attach `ws:// /v1/…/attach`,
      101 before terminal lookup, no auth on dev; input=binary bytes (also the `on_pty_write` back-channel),
      resize=JSON text; reconnect to same `terminal_id` = current-screen redraw, **no byte-replay**
      (transient gap); typed close codes 4404(stop, live-confirmed)/4405(detach)/4500(retry).
  - **✅ SLICE 1b (`lens-terminal` engine core) DONE + merged** (branch `terminal-1b`, 8 commits
    `376dd1c`→`8de30f7`): non-`Send` `VtEngine` on a pinned `std::thread`, Lens-owned `Frame` seam
    (no Ghostty types escape `engine/vt.rs`; full SGR carried for 1c), throttled publish-and-wake
    (`ArcSwapOption` + coalesced waker + `recv_timeout` throttle wake), DA/DSR reverse channel,
    hidden-tab suppression, gated `EngineInspect` ring, offline replay tests (`attach`/`resize`
    captures), Criterion benches (parse ~12µs / frame-build ~590µs @ 200×50). Composer self-ran
    cursor Bugbot (3 concurrency fixes). **grok-4.5 review** caught 4 publish/lifecycle edges, all
    fixed (`8de30f7`): `build_frame` `Err` dropped the dirty bit, `SetVisible(true)` no-force,
    `Stop` abandoned a dirty frame, `Drop` joined on the dropping thread (now detach; join via
    `stop()` only, off-foreground). 16 tests. Plan
    `docs/plans/2026-07-16-terminal-slice-1b-lens-terminal-engine.md`.
  - **Process (this session):** Slice 0 authored + reviewed inline (Opus); 1a∥1b built **in parallel
    isolated git worktrees** by **composer-2.5** (subagent-driven, per-task TDD commits), each
    cross-family reviewed by a **different family** (gpt-5.6-sol on 1a, grok-4.5 on 1b), fixes
    delegated back to composer, then merged to `terminal-ws`. **Full gate green:** `cargo fmt`,
    `clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (lens-client 162 /
    lens-core 202 / lens-terminal 14+2 / all crates). `xtask gate` lists updated for the 2 new crates.
    ⚠ **unpushed** on `terminal-ws`.
  - **✅ 1c + 1d PLANS DONE (2026-07-16, this session)** — authored by **grok-4.5**, cross-family
    reviewed (**codex/gpt-5.6 + Opus** source-verification, **15 findings / 5 Criticals** folded),
    revised, Opus diff-verified, committed `f12a933` (**unpushed**).
    `docs/plans/2026-07-16-terminal-slice-1{c,d}-*.md`. Criticals fixed: `#[gpui::test]` NoopTextSystem
    false-green → real-window `harness=false` gate (memory [[gpui-test-noop-text-system]]);
    off-thread→entity wake impossible → `cx.spawn` sampler + `async_channel`; `TerminalRuntime`
    teardown-ownership; reconnect-seed acceptance (leg-2 seed + scrollback probe); fail-closed perf
    gate xtask-executed on macOS. **NEXT = BUILD 1c → 1d (sequential, 1d needs 1c)** — see
    **[`docs/handoffs/2026-07-16-terminal-slice-1c-1d-build.md`](./handoffs/2026-07-16-terminal-slice-1c-1d-build.md)**
    (self-contained driver for a fresh session).
  - **Plan detail (build):** **1c** full-snapshot render layer (lift `spikes/terminal-render/src/paint.rs`
    split at the `collect_rows`/paint seam — engine already produces the owned `Frame`; apply the 3
    codex fixes + per-cell wide/emoji + **full SGR** the `Frame` now carries + **gate system `Menlo`**
    live-GPUI resolution/alignment, bundle-font fallback) → **1d** convergence (wire `open()`/
    `TerminalTab`/`presentation()`, transport↔engine bridge, `cx.notify` waker, close-code **policy**,
    lifecycle subset + gap marker, retained-engine-seed acceptance test, standalone GPUI demo, live
    proof vs omnigent 0.5.1). 1c needs 1b (done); plan 1c/1d against the now-landed APIs.
    GPUI 0.2.2 + omnigent 0.5.1 pins unchanged.

- **▶ Board B-2..B-6 (board-home)** — §4 board is now decomposed into **six specs B-1..B-6**
  (`docs/SPEC-GAPS.md` → "Board (§4) implementation specs"; supersedes the old B6/B7/B8 framing — B7
  "stable ordinal ordering" dissolved into B-1's ordinal slots, no separate sort task).
  **B-1 (data model & persistence) shipped 2026-07-18** (`8100cc8`; `lens-core` `BoardLayout` tree +
  `SqliteBoardStore`, schema v3). Remaining, in dependency order:
  - **B-2 — packing/scroll/culling SHIPPED 2026-07-21** (`db5b7c2..14b474c`, 10 commits, merged to
    main **unpushed**). `lens-core::pack` pure packer (`foot`/`pack`/`cols_for_width`/`intersects_band`);
    `BoardLayout::board_tree` ordered group-aware walk (skips archived); `lens-ui` absolute-masonry
    `overflow_scroll` container (both board N-col + focus rail 1-col via one `pack_and_render`) with
    band-culling; **container-driven visibility gate** (cards init HIDDEN, `set_visible` via `App::defer`)
    that **retired** the paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry` + `last_mode`
    and fixes the scroll/re-entry freeze at the root. **Basis B (locked):** the packer walks an in-memory
    `BoardLayout` fabricated from `FleetStore` by a PROVISIONAL `build_ephemeral_layout` stub — B-4 deletes
    it when it lands the persisted store→replica seam with the first writes. Subagent-driven build: 6 tasks,
    cross-family review each (codex gpt-5.6), Opus whole-branch review **READY**; `xtask gate` green;
    release demo launches clean (live gate confirmed: animating cards tick, Slept frozen). Memory
    [[board-b2-executed]]; plan `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md`; handoff
    `docs/handoffs/2026-07-21-board-b2-executed.md`.
  - **B-3 — group chrome & rollups SHIPPED 2026-07-21** (`3045590..75b78bb`, 7 commits, merged to main
    **UNPUSHED**). Filled the B-2 placeholder arm with real chrome: `board/rollup.rs` pure `GroupRollup`
    fold (Σspend / oldest-`created_at` age / `completed_count`) + formatters; `group_accent` token→color
    resolver (4 SSOT accents + neutral); `absolute_group` renders ring+accent+7%-tint + header-lane
    (`● dot · name · spend·age · ✓N · ⌄`) folded from member cards; `created_at` plumbed onto `SessionCard`
    (Detailed/Rebased); `test_layout` injection seam + `group_chrome_for_test` hook drive a fixture
    integration test (the group path is not runtime-reachable under basis B). Subagent-driven (composer-2.5
    implementers, codex gpt-5.6 cross-family review of board logic = clean + 1 Minor age-overflow fixed,
    Opus whole-branch review = **SHIP**). `xtask gate` green. **Design deviations from the spec bullet:**
    (a) the `group_of(&SessionCard)` seam was NOT built — group membership is threaded as `GroupMeta`
    through `pack_and_render` from the `board_tree` walk, so a card-keyed reverse lookup is unnecessary;
    (b) `✓N` renders `completed_count: 0` — the real Archive-side count wires in **B-6**. Plan
    `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`. **3 Minors carried into B-4** (Opus review):
    the render-dead `group_header_text`/inline-header duplication (add a live rendered-chrome assertion,
    render from one source); the integration test proves data-wiring not pixels (correct under
    NoopTextSystem — B-4 adds the live check); spec §3 fidelity nits (`.border_1()` 1px vs 1.5px; flat
    wash vs glow/vignette — gpui 0.2.2 has no radial gradient, [[wave-card-body-wash]]).
  - **B-4 — drag/move + context-menu grouping — NEXT.** Drives B-1's `move_item`/`ungroup`/`create_group`;
    lands the store→replica **write** path that deletes the ephemeral `build_ephemeral_layout` stub and
    makes groups runtime-reachable (retiring the B-3 `test_layout` seam); collapse toggle + §7 collapsed-tile
    render. Also owed here: verify the B-3 `absolute_group` member-card read-during-render does not re-trip
    the `.cached()` freeze ([[viewport-reentry-freeze]]) once groups go live; if it does, hoist the rollup
    fold into `sync_card_views`.
  - **B-5 — multiple boards + rail switcher** — board CRUD (B-1 seeds only the default board), the
    externally-discovered-session landing policy, and `FleetStore` connection-scoping.
  - **B-6 — archive-as-board surface.**
  - **Wiring gap (partly closed by B-2):** `BoardView` now reads a `BoardLayout` (via `board_tree`) and
    renders from it — but under **basis B** that layout is the ephemeral `build_ephemeral_layout` stub, NOT
    the persisted `SqliteBoardStore`. The real store→replica wiring (spec §6) + all board **write** paths
    ride with **B-4**, which deletes the stub.
  - **Freeze RESOLVED by B-2:** the scroll-into-view / focus↔board re-entry freeze is fixed at the root —
    the container-driven visibility gate (cards init HIDDEN, `set_visible` via `App::defer`) replaced the
    paint-time `last_bounds` gate + `recover_viewport_gates_on_reentry`. [[viewport-reentry-freeze]] closed.
  - Grounding: specs `2026-07-18-board-data-model-persistence-design.md` (B-1) +
    `2026-07-20-board-packing-and-group-rendering-design.md` (B-2+B-3); handoff
    `docs/handoffs/2026-07-20-board-b2-b3-design-and-spike.md`; memories [[board-b1-executed]],
    [[board-b2-b3-design]].

- **▶ `lens-ui` transcript fan-out** — the first real consumer of the Detailed feed + the disk
  `RowSource`/D23 render window (markdown + virtualization spikes both GO). Plugs into the slot API the
  shell skeleton publishes; sibling parallel surfaces (terminal via `lens-terminal::open`, workspace,
  permissions) can fan out against `ContentTab`/`TabHandle`.
  - **Carry-forward arch notes:** a Summary-mode card consumer MUST tolerate occasional
    `Detailed(TranscriptAdvanced)` watermarks (catch-up/deferred-commit emit them regardless of mode).
    §3.5 Ready *policy* (seen_turn detector / `last_completed_at` stamp / per-card decay one-shot /
    focus-suppress) is lens-ui work over §3.4's `last_completed_turn`. Design spec REVIEW-CLOSED:
    `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md` — settled, don't re-litigate.

- **✅ Fixed 2026-07-21 — turn counter only bumped on `response.completed`.** Cancel/incomplete turns
  now bump `state.stream.turn` so the card flashes `Wave::Ready` ("just finished"). As-built (differs
  from the original handoff shape after two codex reviews): **Incomplete/Cancelled** bump the counter;
  **Failed** does NOT (it surfaces via `Wave::Failed`, and status is not folded atomically with the
  event — bumping would flash a transient green). All three **discard** open scratch (not finalize) —
  committing a synthetic local partial would permanently duplicate omnigent's durable `interrupted`
  `/items` row (messages reconcile by `item_id` only). `crates/lens-core/src/reduce/{mod,folds}.rs`;
  handoff `docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md` (as-built appended);
  memory [[turn-counter-noncompleted-bug]]. **Live-verified 2026-07-21:** interrupted a streaming
  claude-sdk turn vs omnigent 0.5.1 — the partial is flushed via `response.output_item.done` (durable
  `/items` row under a **server** id) BEFORE `response.cancelled`, so the reducer commits it under that id
  and the Cancelled-arm discard is a no-op for the message → partial preserved, no loss, no duplicate
  (discard validated; the omnigent source's "Phase 2 TODO / not persisted" docstring is wrong). **One
  follow-up remains:** native `turn.completed/failed/cancelled` are deferred → `Unknown` → ignored, so the
  same bug persists on the native-runner surface. Merge-collision with T-0
  (`lens-transript`) in the same `reduce` match block stands — logically independent, textual merge only.

- **📋 SPEC-GAPS backlog** — independent, un-specced/partial items tracked in
  [`docs/SPEC-GAPS.md`](./SPEC-GAPS.md) (incl. #10 keyboard shortcuts + macOS app menu, Cmd+Q dead).

## Deferred, with a clean seam

- **lens-client modeling follow-on** — flip the 13 byte-verified SSE families `SCHEMA-DERIVED→MODELED`
  (capture done, memory `live-event-recapture-findings`); grow the two under-modeled payloads (`child{}`,
  elicitation `params`). Still-blocked families (`turn.*`, `response.created/queued`, codex reasoning)
  need a codex sub / OpenAI key.
- **lens-client small hardening** — `info.databricks_features: Value` leak; `ClientError::NotFound`
  rename + typed `Validation`/422; `/items` pagination; gated live-reconnect smoke.
- **WS terminal-attach client (Plan 7)** — no `terminal.rs`/`tungstenite` yet; workspace/terminal half
  of the contract is a known build-order deferral (converging with sibling `lens-terminal-ws`).
- **`session.superseded` reducer-drop** (`folds.rs:136` discards `target_conversation_id`) blocks
  terminal supersession-reattach — lens-core must surface it; terminal-integration-era.
- **Notifications v2** — server push for the fully-quit case (needs an upstream omnigent push channel).
- **Reducer normalization** — two status vocabularies (`SessionStatusValue` 6-val live vs
  `SessionStatus` 3-val snapshot) + two usage representations to normalize consumer-side.

## Open small decisions

- **Tunables (verification pass):** auto-sleep threshold (~10m), poll cadence (~10s), ring-buffer size
  (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Undecided UX:** terminal-`transfer` UX, managed-provider selection, policy/skill in-app authoring,
  multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** all status/harness/render glyphs are real Lucide SVGs (bell, triangle-alert,
  loader-circle, alarm-clock, check, moon, coffee, circle-dot, folder, git-branch). Only chrome
  furniture is still unicode — the kebab `⋮` and close `✕` (trivially swappable to `ellipsis-vertical`/
  `x` if/when a fully-bespoke set is wanted).

## Recently shipped (all on `main` unless noted)

- **Board B-3 — group chrome & rollups (2026-07-21):** filled the B-2 group placeholder with real chrome —
  `board/rollup.rs` pure `GroupRollup` fold (Σspend / oldest-`created_at` age / `completed_count`) +
  formatters; `group_accent` token→color resolver; `absolute_group` ring+accent+7%-tint + header-lane folded
  from member cards; `created_at` plumbed onto `SessionCard`; `test_layout` injection seam + `group_chrome_for_test`
  hook + fixture integration test (path runtime-dormant under basis B). `group_of` seam dropped (membership
  threaded as `GroupMeta` via `board_tree`); `✓N`=0 until B-6 Archive source. Subagent-driven (composer-2.5,
  codex gpt-5.6 board-logic review clean+1-Minor-fixed, Opus whole-branch SHIP). `xtask gate` green. **`3045590..75b78bb`,
  merged to main (UNPUSHED)**. Plan `docs/plans/2026-07-21-board-b3-group-chrome-rollups.md`.
- **Board B-2 — packing/scroll/culling (2026-07-21):** `lens-core::pack` pure packer + `board_tree`
  walk + `lens-ui` absolute-masonry `overflow_scroll` container (board N-col + rail 1-col via one
  `pack_and_render`) with band-culling + container-driven visibility gate that retired the paint-time
  `last_bounds` gate/`recover_viewport_gates_on_reentry`/`last_mode` (freeze fixed at root). Basis B:
  ephemeral `build_ephemeral_layout` stub feeds the tree (real store→replica = B-4). Subagent-driven
  (6 tasks, composer-2.5 implementers, codex gpt-5.6 per-task review, Opus whole-branch **READY**);
  `xtask gate` green; release demo launches clean. **`db5b7c2..14b474c`, merged to main (UNPUSHED)**.
  Memory [[board-b2-executed]]; handoff `docs/handoffs/2026-07-21-board-b2-executed.md`.
- **Board B-1 — data model & persistence (2026-07-18):** `lens-core` `BoardLayout` recursive
  Board→(Card|Group) tree + `SqliteBoardStore` (control-tier `lens.db`, schema **v2→v3** additive, lazy
  placement no backfill), ordinal-slot placement, mutation ops (place/move/ungroup/group/archive/…),
  bidirectional startup reconcile (lazy-place live, prune tombstoned). Adversarial review (grok-4.5 +
  probe tests; grok's "HIGH id-collision" refuted empirically) → 5 hardening fixes (high-water-mark id
  seed, tombstone place-guard, cycle seen-guard, deterministic reconcile order, +7 tests). 30 board
  tests, full `xtask gate` green. Committed **`8100cc8` (UNPUSHED)**. Spec
  `2026-07-18-board-data-model-persistence-design.md`; memory [[board-b1-executed]]; handoff
  `docs/handoffs/2026-07-18-board-b1-executed.md`.
- **Wave build B1–B5 + follow-ups (2026-07-17):** Lucide glyph tiles, context pbar, Slept/Wake/Retry
  seams, `loader-circle` spinner, canvas `paint_path` sweep, Scheduled countdown, viewport-gated
  20fps/1Hz anim driver, `demo` feature-gate; on-device visual pass; per-wave card-body wash; header
  3-tier type + host pill + per-wave activity line; **perf 30→20fps** (~35% CPU, `wave-perf-fps-attribution`).
  Spec `2026-07-17-wave-behaviors-design.md` §11. Handoff `2026-07-17-wave-build-visual-pass-merged.md`.
  - **Viewport re-entry freeze — RESOLVED (2026-07-17):** focus→board no longer freezes the off-screen
    card's spinner/pulse. Reset lives in `BoardView`'s fleet-observe effect; 3 regression tests; codex
    review addressed. Memory [[viewport-reentry-freeze]]. **Unpushed on `main`** (see below).
- **§18 Theming substrate (2026-07-16):** `crates/lens-ui/src/theme/` — `LensTheme` global (base+status
  tokens, hex↔Hsla serde, dark+light JSON), `cx.lens_theme()`, gpui-component bridge, external-file load
  + `cmd-shift-t` reload, `shortcuts.rs`. **On `main`, load-bearing for the cards.** Palettes tuned
   during the 2026-07-17 §11 on-device visual pass (bg ramp, wave status colors, context-bar thresholds,
   per-wave wash intensities) — no longer placeholders; residual fine-tuning is cheap via the reload
   loop. Memory [[lens-ui-theming-fork]].
- **`lens-ui` shell skeleton Plan 2 + card/board audit (2026-07-15/16):** §4–§7 skeleton merged; wave
  colors un-swapped, Needs-input=orange, icon-tile readout. Gate now covers lens-ui/lens-app.
- **lens-core §3 ActorFeed gate (2026-07-15):** unified `ActorFeed` FIFO, scheduler dual-mode,
  seed-on-spawn + emit-on-Demote, enriched `SummaryUpdate`. Grok-authored plan, subagent-driven.
  Memory [[grok45-as-plan-author]].
- **state-model engine P0–P3 (2026-07-08 → 07-12):** domain types → pure reducer → two-tier SQLite
  persistence → actor + store + commands + P3-3a/b lifecycle. All merged. Memories `state-model-*`.
- **lens-client (2026-06-25 → 07-10):** REST surface (Plans 2a–2e), SSE event modeling (Plan 3 series),
  benchmarks, pre-consumer hardening (Plan 4), omnigent pin `0.3.0.dev0 → 0.5.1`. Memories `plan3*`, `plan4*`.

## Housekeeping

- **`main` is AHEAD of `origin` by 5 (unpushed, as of 2026-07-18):** `759eb3a` (status fix) ·
  `c855ab6` (SPEC-GAPS §4 board → B-1..B-6) · `c21e669` (docs relocate → specs/plans) · `8100cc8`
  (B-1 board data model) · this docs-status commit. `origin/main` is at `b8727ab`. Push decision
  deferred to the user.
