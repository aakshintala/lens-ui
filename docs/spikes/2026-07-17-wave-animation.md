# Spike — wave animation feasibility + cost (B3)

**Date:** 2026-07-17 · **Branch:** `feat/lens-app-multi-session` · **Verdict: GO**
Design: `docs/specs/2026-07-17-wave-behaviors-design.md` §8.

## Question

Can continuous per-frame card animation (the "wave") run on the `SessionCard` — which
is mounted inside a gpui `.cached()` wrapper — without violating §4.4 render isolation,
and **at what CPU/energy cost at scale**?

## Method

Built the sweep + (later) a frame-capped timer driver on the real card path behind
`--demo`, instrumented with the card view's existing `render_count`/`paint_count`, plus
env knobs: `LENS_DEMO_N` (replicate the 8 preset cards), `LENS_ANIM_MS` (animation tick
interval / frame-rate cap), `LENS_ANIM_DBG` (log per-frame phase). Measured headless via
`top -l N -s 1 -pid … -stats cpu,power` (per-process CPU% + energy-impact — no sudo) and
FPS from paint-count deltas. Release build.

## Findings

### 1. Feasibility — GO (proven)
`window.request_animation_frame()` / `cx.notify(card_entity)` **do** repaint a `.cached()`
card each frame; the cache does not freeze it, and `paint == render` (every frame paints).
**§4.4 isolation holds**: with a neighbor animating at ~120fps, static cards' paint stayed
pinned (3–4) while animating cards climbed to 5621 — a ~1400× separation. Self-notify
never touches `FleetStore`.

### 2. Cost — real, but managed by the architecture
Clean N=1 (5 animating cards visible), release, **moving** animation:

| Driver / config | ~5 cards | per-card |
|---|---|---|
| idle floor (anim off) | **0.3%** | — |
| **timer @30fps** (approach ②, capped) | **8.8%** | ~1.7% |
| timer @120fps (native) | 14.8% | ~2.9% |
| `.with_animation` (gpui built-in, 120fps) | ~21% | ~4% |

- **Dropping `.with_animation` for a timer-driven self-notify is the biggest win**
  (21%→15% at the same fps): its per-frame `request_layout` machinery was the hog, not
  the painting.
- **30fps cap saves ~40% more** (15%→8.8%) and is imperceptible for a subtle shimmer.
  (Not the 4× I first assumed — a fixed per-notify render cost doesn't scale with fps.)
- **Idle floor 0.3%** — static cards are essentially free.
- **Net ≈ 1.7% CPU per visible animating card @30fps.**

### 3. Locked animation architecture (approach ②, refined by the cost data)
- **Timer-driven self-notify, NOT `.with_animation`.** Per animating card: a `cx.spawn`
  loop `background_executor().timer(tick).await → this.update(cx, |_,cx| cx.notify())`,
  live only while the card's wave animates (`Task` dropped → cancelled when it stops).
- **Frame cap ~30fps** (`tick ≈ 33ms`).
- **Phase = pure fn of the clock** (`UiClock::now_millis`) → deterministically testable
  with `ManualUiClock`, and the value advances (see gotcha #1).
- **Viewport-gate** (build task): only visible cards should animate. Off-screen cards
  already freeze (gpui skips compositing clipped content), so this bounds cost to screen
  capacity (~15 cards), **not fleet size** — make it intentional, not accidental.

## Gotchas the build MUST carry

1. **f32 precision on epoch-millis (froze everything).** `(now_ms as f32).rem_euclid(period)`
   quantizes — epoch-millis (~1.8e12) exceeds f32's 24-bit mantissa (~131k-ms step), so
   the phase is constant and the card repaints a frozen image. **Do the modulo in i64
   first, then cast the small remainder** (`now_ms.rem_euclid(period_ms) as f32 / period_ms`).
   `.with_animation` avoided this by deriving delta from small `Instant` durations.
2. **Sweep fidelity — div gradient is a flat vertical bar.** gpui `linear_gradient` is
   2-stop only and divs can't skew, so the div band reads "bleh". The real technique is
   **`canvas` + `window.paint_path`**: a skewed gradient **parallelogram** (`PathBuilder::fill()`,
   respects the clip mask), phase-driven, asset-free. Deferred to build tuning; confirmed available.
3. **Working spinner needs a bundled SVG asset.** gpui-component `Spinner` renders
   `IconName::Loader → "icons/loader.svg"`, but the crate ships **no** icon SVGs and the
   app registers no `AssetSource` → the spinner animates an invisible glyph. Build must
   bundle an SVG + `Application::new().with_assets(..)`, or draw a canvas/rotated-svg arc.
   Currently a **static ring placeholder**.
4. **Demo board doesn't scroll** — N>~2 overflows the window and clips/freezes cards
   (that's board work, B6; it confounded the >N=1 measurements).

## Spike code state (uncommitted-then-committed as the B3 seed)
Touched `crates/lens-ui/src/card/{motion.rs (new),chrome.rs,view.rs,mod.rs}` +
`crates/lens-app/src/main.rs`. `motion.rs` = sweep/ring/spinner/glyph + phase fns;
`view.rs` = the 30fps timer driver + `anim_task` field + phases; the icon-tile (B1) and
sweep landed too. The plan **productionizes** this (viewport-gate, canvas sweep, spinner
asset, countdown ring, tests); it is not final.

## Deferred to the build (not spike scope)
Canvas `paint_path` sweep; viewport-gating; Working spinner asset; Scheduled 1Hz
countdown ring (`canvas` arc) + wake-at-T repaint; light-theme + color tuning; render
benchmark.
