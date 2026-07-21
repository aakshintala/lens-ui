# Handoff — wave-behaviors design LOCKED + animation spike DONE; next = `writing-plans` for B1–B5

**Date:** 2026-07-17 · **Branch:** `feat/lens-app-multi-session` — branch-only, not merged/pushed.
**Gate:** lens-ui/lens-app green (fmt/clippy/38 tests). lens-core has a **known flaky** d30
timing race (passes on rerun) — not from this work.

Picks up from `docs/handoffs/2026-07-16-wave-states-shipped.md` ("NEXT: wave build B1–B5").
This session: brainstormed the wave *behaviors* → locked the visual design → ran the
animation feasibility+cost spike. **Next session = write the B1–B5 plan** (`writing-plans`),
then execute subagent-driven.

---

## What this session produced

1. **Visual design LOCKED** — `docs/specs/2026-07-17-wave-behaviors-design.md`
   (the SSOT for the build). Live tunable mockup: `docs/design/renders/wave-states-motion.html`.
2. **Animation spike EXECUTED → GO** — `docs/spikes/2026-07-17-wave-animation.md` (verdict +
   cost + gotchas). Spec §8 now holds the LOCKED architecture.
3. **Spike code = the B1/B3 seed** (committed this session, NOT final): the icon-tile (B1) +
   the sweep + a 30fps timer driver landed in the real card path.

## The design in one screen (from spec §2–§7)

**Strategy B — motion *character* encodes class**, not a speed ladder:
- **Linear sweep** = "your attention is wanted" (NeedsInput/Failed 1.0s **+ expanding ring**;
  AwaitingReview/Ready 1.5s, no ring). Amplitude 0.4.
- **Rotation (spinner)** = "machine working" (Working; no sweep).
- **Depleting countdown ring** = "self-driving" (Scheduled; 1 Hz; "wakes in 2m 59s").
- **Still** = dormant (Slept = dim content + bright **Wake** button; Neutral/Idle = ☕, still).

Glyphs FINAL: 🔔 NeedsInput · ⚠ Failed · spinner Working · ⌾ AwaitingReview · ⏰ Scheduled ·
✓ Ready · ☾ Slept · ☕ Idle. Colors = current placeholders (end-of-build tuning pass).
Ladder unchanged (already shipped): NeedsInput>Failed>Working>AwaitingReview>Scheduled>Ready>Slept>Neutral.

## Animation architecture — LOCKED (spec §8, spike-proven)

- **Driver = frame-capped timer self-notify (approach ②), NOT `.with_animation`.** Per card:
  `cx.spawn` loop `timer(≈33ms).await → this.update(|_,cx| cx.notify())`, in an
  `Option<Task<()>>` live only while the wave animates. `.with_animation` was ~21% CPU/5cards
  (its per-frame layout is the hog); timer @30fps ≈ 8.8% (~1.7%/card); floor 0.3%.
- **Phase = pure fn of `UiClock::now_millis()`** (testable w/ `ManualUiClock`).
  ⚠ **i64 modulo BEFORE f32 cast** — epoch-millis overflows f32 mantissa → frozen phase.
- **§4.4 isolation holds** (self-notify only). **Viewport-gate** = build task.
- Countdown ring (Scheduled) = 1 Hz + a wake-at-`scheduled_wake_at` repaint (mirrors
  `READY_DECAY_MS`), drawn via `canvas` arc — NOT in the 30fps loop.

## What the B1–B5 PLAN must cover

**Structural shell (mechanical port from `board-home.html` render SSOT):**
- **B1** icon-tile (44px) — *seed landed*; finalize glyph/spinner/countdown-ring hosting,
  delete the throwaway pill (`wave_label`/`pill_text_color` in `chrome.rs`).
- **B2** context-window progress bar (`.pbar`). **B4** layout order (tile-left + stacked).
- **B5** Slept dim + Wake button; Failed Retry affordance.

**Animation system (productionize the spike seed):**
- Timer driver + 30fps cap + phase-from-clock (i64-modulo) — *seed landed, harden + test*.
- **Viewport-gate** the driver (only visible cards animate).
- **Sweep via `canvas` + `paint_path`** (skewed gradient parallelogram) — replaces the flat
  div band (spike gotcha #2; technique confirmed available, asset-free).
- **Expanding ring** (NeedsInput/Failed) — *seed landed (div), phase-driven*.
- **Scheduled countdown ring** — `canvas` arc, 1 Hz, `(wake−now)/(wake−start)` deplete +
  live "wakes in Xm Ys" + wake-at-T repaint. **Not built in spike.**
- **Working spinner** — bundle an SVG asset (`AssetSource`) or draw a canvas/rotated-svg arc
  (gpui-component `Spinner` needs `icons/loader.svg` the crate doesn't ship). *Static ring
  placeholder now.*

**Testing:** unit (phase-from-`ManualUiClock` pure fns); extend the §4.4 isolation
acceptance test (animating neighbor must not bump a static card's `paint_count`); on-device
CPU pass + end-of-build color/light tuning via the reload loop (`⌘⇧T`).

## Spike code as-built (the seed to productionize)

Files: `crates/lens-ui/src/card/motion.rs` (**new** — sweep/ring/spinner/glyph + `sweep_phase`/
`ring_phase`/`wave_animates`), `card/chrome.rs` (icon-tile + `sweep_phase: Option<f32>` param,
pill removed), `card/view.rs` (`anim_task` field + 30fps timer driver + phases), `card/mod.rs`
(exports), `lens-app/src/main.rs` (`LENS_DEMO_N` replication + paint instrumentation).

Env knobs (spike affordances, keep or strip in the plan): `LENS_DEMO_N` (replicate 8 cards),
`LENS_ANIM_MS` (tick/fps cap, default 33), `LENS_ANIM_DBG` (log phase). Run:
`cargo run -p lens-app --release -- --demo`.

**Known-incomplete in the seed (the plan finishes these):** flat div sweep (→ canvas path);
static spinner (→ asset); no countdown ring; no viewport-gate; no animation unit tests; the
`LENS_ANIM_DBG` eprintln.

## Start-here for the new session
1. `superpowers:writing-plans` from spec `2026-07-17-wave-behaviors-design.md` + this handoff.
2. Structure B1–B5 as: finish structural shell (B1/B2/B4/B5) + productionize the animation
   system (driver/viewport-gate/canvas-sweep/countdown-ring/spinner-asset) + tests.
3. Then execute subagent-driven (composer per task + cross-family seam review), per project rules.
4. Sequencing after: board packing B6–B8 (scroll!) → light checkpoint → transcript → …
