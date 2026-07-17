# Handoff — Wave Build B1–B5 follow-ups

**Written:** 2026-07-17 · **Branch:** `feat/lens-app-multi-session` · **HEAD:** `86a74aa` (branch-only, NOT merged/pushed)
**Plan executed:** `docs/superpowers/plans/2026-07-17-wave-build-b1-b5.md` (all 11 tasks DONE)
**SDD ledger (per-task detail, decisions, adjudications):** `.superpowers/sdd/progress.md` → the `PLAN 4 — Wave Build B1-B5` section (git-ignored scratch; if `git clean -fdx` nuked it, reconstruct from `git log 52c31eb..HEAD`)
**Perf write-up:** `docs/spikes/2026-07-17-wave-build-perf.md`

## TL;DR of where things stand

The wave build is **code-complete, gate-green, and reviewed by two model families** (Opus + codex, whole-branch; all 4 findings fixed in `a698a9a`, codex verify-the-fixes clean). What's left is **stuff that genuinely needs a human on-device** plus two triage calls. Nothing below blocks compilation or tests — the branch builds and `cargo run -p xtask -- gate` passes (default **and** `--features demo`).

Commits in scope: `52c31eb..86a74aa` (12 wave-build feature commits `8265aef..3f9ce8f`, review-fix `a698a9a`, docs `86a74aa`).

**Verify green before starting any follow-up:**
```bash
cargo run -p xtask -- gate          # fmt + workspace clippy (default+demo) + tests + drift. Do NOT pipe through tail.
```

---

## Follow-up 1 (PRIMARY GATE) — on-device visual acceptance

None of the visuals were confirmed on-device this session (autonomous run can't judge pixels). This is the real remaining gate before merge. Under gpui `TestAppContext` the text/SVG system is a `NoopTextSystem` — font/shape/paint asserts are false-green (memory `gpui-test-noop-text-system`), so the ONLY way to validate the look is to run it and look.

```bash
cargo run -p lens-app --release --features demo -- --demo
# 8 cards, one per wave state. LENS_DEMO_N=2 → 16 cards (tests overflow, not scroll).
# Reload loop for token/color tuning: ⌘⇧T with LENS_THEME_DIR=crates/lens-ui/src/theme
LENS_THEME_DIR=crates/lens-ui/src/theme cargo run -p lens-app --release --features demo -- --demo
```

Checklist (each is a card in the demo):
- [ ] **Glyphs render + tint (RISK #1).** Each tile should show a **line-icon glyph in the status color** on a faint (14%) tinted square: bell / triangle-alert / circle-dot / alarm-clock / check / moon / coffee. **If a glyph is INVISIBLE**, the assumption that gpui `svg().text_color()` tints a stroke-based Lucide SVG as a mask is wrong → change the bundled SVGs from `stroke="currentColor"` to `fill="currentColor"` (files in `crates/lens-ui/assets/icons/*.svg`; they're Lucide v1.24.0, ISC). This is the single most likely thing to be broken.
- [ ] **Working spinner** rotates (a green arc-spinner, `loader-circle`), smooth at ~30fps.
- [ ] **Sweep** on NeedsInput/Failed/AwaitingReview/Ready: a soft diagonal light band sweeping left→right — NOT a flat vertical bar. Watch the **skew direction** (see Follow-up 2 — mockup is `skewX(-14deg)`, code uses `+14°`; cosmetic, tune here).
- [ ] **Scheduled**: a depleting arc around the ⏰ tile + activity line "wakes in Xm Ys" ticking once/second.
- [ ] **Slept**: card content dimmed (~42%) **including the progress bar**, with a **bright Wake pill** top-right. **Failed**: error text in the activity line + a bright **Retry pill**.
- [ ] **Click Wake/Retry** → should NOT focus/promote the card (fixed via `stop_propagation` — verify it holds on-device; there's no unit test for this, see ledger F3). Same for the kebab `⋮`.
- [ ] **B2 pbar**: thin status-colored fill under the foot; needs-input shows its %, failed ~0%.
- [ ] **Dark + light** both legible; run `LENS_THEME=light` too.

Final color/light tuning is one end-of-build pass via the reload loop — placeholder alphas (sweep peak `0.096`, dim `0.42`, tile tint `0.14`) are tunable in `crates/lens-ui/src/card/motion.rs` + `chrome.rs`.

---

## Follow-up 2 (TRIAGE) — one on-device "sweep pass" covering perf + sweep-feather

Two deferred items both point at `render_sweep_overlay` (`crates/lens-ui/src/card/motion.rs:89`), so do them together with Instruments open.

**(a) CPU overage — a real finding.** On-device release measurement: **~12% CPU for 5 fast-animating cards vs the spike's 8.8% budget** (~2.3%/card vs 1.7%, ~30–35% over). Reproducible loaded + quiet. Cadence is PERFECT (30fps fast / 1Hz Scheduled / 0 static, confirmed from `paint-instr` stderr; no frame drops to 16 cards) — the overage is the **new-since-spike** per-frame work:
  - canvas sweep (`render_sweep_overlay`): 2× `PathBuilder::fill().build()` + 2× `paint_path` per card per frame; the parallelogram moves every frame (phase-dependent) so the path can't be cached.
  - SVG spinner (`render_working_spinner`, `motion.rs:148`): `with_transformation` re-transform every frame.
  - Whole `SessionCardView::render` tree rebuilt each tick (inherent to timer→notify→full-render — same as the spike).

  **Decision:** accept (subtle-shimmer quality bought it; scales sub-linearly — 16 cards ≈ 15%; no frame drops) OR profile + reduce. Rig to reproduce:
  ```bash
  cargo run -p lens-app --release --features demo -- --demo &
  PID=$(pgrep -f 'target/release/lens-app' | head -1)
  top -l 8 -s 1 -pid "$PID" -stats cpu,power   # steady-state CPU
  # FPS from paint-instr stderr: render-count Δ per ~2s cycle (fast=+60→30fps, scheduled=+2→1Hz)
  ```
  Candidate reductions (need on-device visual check — they risk the Task 6/5 gains): collapse the sweep to a single `paint_path` with one gradient if it holds visually; cheaper spinner rotation; confirm countdown truly 1Hz (it is). Use Activity Monitor's **Energy** tab for the authoritative energy read (top's POWER is only a relative proxy).

**(b) Sweep feather not uniform across the slant** (codex Task 6 finding, deferred). gpui evaluates a path gradient over its **axis-aligned bounding box**, so a 2-stop 90° gradient can't follow the skewed parallelogram edges — the feather isn't uniform along the slant. Not fixable within gpui's 2-stop `linear_gradient` API without a different technique. Plan scoped this to §8a on-device tuning. If the look is unacceptable on-device, options: multi-band approximation, or a per-pixel shader if gpui exposes one. Peak alpha is only `0.096` so it may be imperceptible — judge on-device before investing.

Also cosmetic in the same function: `SWEEP_SKEW_DEG = 14.0` (`motion.rs:82`) shears the top RIGHT; the mockup `wave-states-motion.html` is `skewX(-14deg)` (opposite lean). Flip the sign if the mockup direction is wanted.

---

## Follow-up 3 (B6 carry-forward) — viewport re-entry can leave a card stuck

**Unreachable in this build; do NOT fix here — it rides with B6's scroll container.** Mechanism (codex verify-the-fixes): `SessionCardView::render` (`view.rs:84`) reads the **previous** painted `last_bounds`; the canvas paint closure updates `last_bounds` LATER (`view.rs:160`) **without** notifying. So a card that moves off→on screen can, on its first re-entry render, still see stale off-screen bounds → `visible=false` → `desired=None` → `anim_interval` stays `None` → no task spawned → no re-render → stuck non-animating until some other notify fires.

Why it's safe now: this is a **non-scrolling window**. Overflow cards (>window) start off-screen and stay there; the only bounds changes (window resize, board↔focused relayout) come WITH a re-render that refreshes `last_bounds`; live SSE folds notify regularly and self-heal. The stuck path specifically needs scrolling that moves a card's viewport position without re-rendering its view — i.e. B6's scroll container. **When building B6:** the scroll container must invalidate/re-render (or at least re-evaluate the gate for) cards as they cross the viewport edge. Do NOT try to fix it by notifying from the paint closure — that breaks gpui's render/paint separation. Add a Scheduled↔offscreen↔onscreen transition test at that point.

---

## Follow-up 4 — merge decision (yours)

Solo-project convention (memory `integration-workflow` / `commit-when-finished`): merge straight to `main`, no PR; **don't auto-push** (separate call). Gate = all tests pass + zero warnings/dead-code — currently GREEN. But **don't merge before Follow-up 1** (the on-device visual pass) — glyph-tint risk #1 could force an SVG `fill`-vs-`stroke` change first. Suggested order: Follow-up 1 → fold any visual fixes → optional Follow-up 2 → merge → push when ready.

---

## Design SSOTs / references (for any visual work)
- Motion params: `docs/design/renders/wave-states-motion.html` (sweep/skew/alpha; pbar-in-dim-group at :111)
- Card structure: `docs/design/renders/board-home.html` (pbar track = rgba(255,255,255,.06))
- Spec: `docs/superpowers/specs/2026-07-17-wave-behaviors-design.md`

## By-design / not-bugs (don't "fix" these)
- **B5 Wake/Retry → no-op `FleetStore` seams** (`wake_session`/`retry_session` in `fleet/store.rs`) — real behavior lands with state-model wake=respawn plumbing. The buttons are real affordances wired to the seam.
- **pbar track = `gpui::white().opacity(0.06)`** — intentional mockup neutral, not a status token.
- **Scheduled "wake-at" repaint** is subsumed by the 1Hz tick (`derive_wave` self-clears within ≤1s of wake).

## Delegation notes (how this was driven — reuse next session)
- Build = `cursor-delegate` composer-2.5 per task, fed a `scripts/task-brief` file. Composer nails static shapes but: (1) the plan's code is NOT rustfmt-clean → composer MUST `cargo fmt` before commit; (2) composer's per-task clippy gate WITHOUT `--tests` misses test-module compile errors (bit us on the T10 `Duration` cfg — the full `xtask gate --all-targets` is the arbiter); (3) composer can STALL/ERROR on the final response but have already committed — always trust-but-verify with a direct gate run.
- Review = codex (`codex exec -s read-only`, gpt-5.6-sol, free, cross-family) on tricky seams + whole-branch; Opus (this agent) coordinated + adjudicated. The two whole-branch families DIVERGED (Opus SHIP vs codex FIX-FIRST) — running both caught 2 real bugs one family missed. Memories: `gpui-nested-click-stop-propagation`, `whole-branch-review-needs-a-builder`, `xtask-gate-scope`.
