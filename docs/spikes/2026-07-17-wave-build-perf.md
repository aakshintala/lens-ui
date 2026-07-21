# Wave Build (B1–B5) — Post-build Perf Re-measure (Task 11)

> **RESOLVED (2026-07-17, follow-up session).** The overage triage below is closed. Attribution
> (demo-gated `LENS_PERF_NO_SWEEP`/`NO_SPINNER_ROT` toggles + a frame-rate sweep) **overturned the
> prime suspect**: disabling *both* the canvas sweep paint and the spinner re-transform bought only
> ~1% of ~13% — they are NOT the cost. The cost is the **per-frame full `SessionCardView::render`
> tree rebuild**; CPU scales ~linearly with frame rate (30fps 13.8% → 20fps 9.0% → 15fps 7.65% →
> 10fps 5.8%, for 5 fast cards). **Fix shipped: fast-wave cap 30fps → 20fps** (`anim_tick_ms_fast`
> 33→50ms) — a ~35% CPU cut that lands back at the §9 8.8% budget (measured 8.95% default).
> On-device A/B (labeled 20 vs 30fps windows): **indistinguishable**. Key perceptual finding — the
> **sweep** (a band *translating* ~280px) is the frame-rate-sensitive element, NOT the spinner (a
> tiny 22px rotation); the sweep stays smooth to 20fps and only jars by ~10fps, while the spinner is
> fine even at 10fps. The doc's proposed sweep/spinner reductions were therefore correctly declined
> (near-zero value, real visual risk). The sweep-feather item (b) is untouched and still deferred.

**Date:** 2026-07-17 · **Build:** `feat/lens-app-multi-session` @ Task 10 (`5dd3f8f`)
**Binary:** `cargo build --release -p lens-app --features demo` → `target/release/lens-app --demo`
**Rig:** same machine as the animation spike (`docs/spikes/2026-07-17-wave-animation.md`); `top -l N -s 1 -pid <pid> -stats cpu,power`; per-card FPS from the `paint-instr` stderr counters (`spawn_demo_paint_instrumentation`, `--features demo`).

This is the REQUIRED spec §9 perf-completion gate: re-run the spike rig, hold against its budget. Full-scale (>16 cards, scrolling) validation rides with B6; this validates the visible set.

## Spike budget (baseline to hold)

| metric | spike value |
|---|---|
| idle floor (anim off) | ~0.3% CPU |
| per visible animating card @30fps | **~1.7% CPU** |
| 5 animating cards @30fps (timer, capped) | **~8.8% CPU** |

The spike measured **div-based** animation (`.with_animation` vs the timer driver). This build replaced several visuals with heavier techniques since the spike: **canvas `paint_path` sweep** (Task 6, was a div gradient), **clock-rotated SVG `loader-circle` spinner** (Task 5, was a static ring), **canvas arc countdown ring** (Task 7, new). So a modest CPU delta over the div-era budget is expected; the gate is whether it is *material*.

## Measurements

8-card demo (`LENS_DEMO_N=1`): one card per wave — 5 fast-animating (needs-input, failed, awaiting-review, ready @30fps sweep; working @30fps spinner), 1 Scheduled (1 Hz), 2 still (slept, neutral).

### Cadence / FPS — PASS (exactly as designed)

From `paint-instr` render-count deltas over consecutive ~2 s cycles:

| card | Δrender / cycle | effective rate | expected |
|---|---|---|---|
| needs-input / failed / awaiting-review / ready | +60 | **~30 fps** | 30 fps sweep ✓ |
| working (SVG spinner) | +60 | **~30 fps** | 30 fps ✓ |
| scheduled (countdown) | +2 | **~1 Hz** | 1 Hz ✓ |
| slept / neutral | +0 (frozen at 3) | **static** | still, no driver ✓ |

The per-wave cadence (`anim_tick_for`: 33 ms fast / 1000 ms Scheduled / None still) and the still-wave/viewport gate hold **on-device**. No frame drops observed even at 16 cards (all fast replicas stayed at 30 fps). This validates the §8 driver architecture.

### CPU / energy — OVER the div-era budget (finding)

Steady-state `%CPU` (release, window foregrounded and actively painting — confirmed by climbing paint counts, i.e. NOT occlusion-throttled):

| scenario | system load | steady-state CPU | POWER (top, relative) |
|---|---|---|---|
| 8 cards (5 fast + 1×1Hz + 2 still), loaded system | LA ~5.3 | ~10–14% (median ~12%) | ~14 |
| 8 cards, quieter system | LA ~2.1 | ~10–13% (median ~11.8%) | ~12–13 |
| 16 cards (10 fast, `LENS_DEMO_N=2`), all on-screen | LA ~4 | ~14–16% | ~15–17 |

- **Static cards are free:** slept/neutral never animate (render frozen at 3). ✓
- **Per-fast-card:** (~11.8% − ~0.3% idle) / 5 ≈ **~2.3% CPU/card**, vs the **1.7%** budget → **~35% over**. Reproducible across loaded and quiet runs → not a system-load artifact.
- **Scales sub-linearly:** 10 fast cards ≈ 15% (~1.5%/card marginal), no frame drops — the overage is dominated by per-frame fixed work, not runaway.

### Viewport-gate off-screen culling — NOT exercised here

The demo lays 16 cards inside the window and they all fit (no overflow → all on-screen → all correctly animate). The demo does not force a card off-screen, so this run does not exercise the gate's *culling* path. Off-screen culling correctness is covered by the §4.4 isolation test (`animating_card_does_not_render_a_static_sibling`, Task 9) + the codex-reviewed gate logic (Task 8), and needs a genuinely-overflowing/scrolling window to observe on-device — that rides with B6.

## Verdict

- **Cadence / driver architecture gate: PASS.** 30 fps fast, 1 Hz Scheduled, 0 for still — confirmed on-device, no frame drops to 16 cards. Static cards cost nothing.
- **Absolute-CPU gate: OVER BUDGET by ~30–35%** (~2.3%/card vs 1.7%; ~12% vs 8.8% for the 5-fast-card set). This is a **finding** per Task 11's own criterion ("within ~10% of 1.7%").

### Prime suspect + recommended follow-up (scoped, not done here)

The delta over the div-era budget is the per-frame cost of the visuals added *since* the spike, re-run every 33 ms for every fast card:
1. **Canvas sweep** (`render_sweep_overlay`): 2× `PathBuilder::fill().build()` + 2× `paint_path` (gradient) per card per frame — the parallelogram moves every frame (phase-dependent), so the path cannot be cached.
2. **SVG spinner rotation** (`render_working_spinner`): `with_transformation` re-transform of the `loader-circle` SVG every frame.
3. The whole `SessionCardView::render` element tree is rebuilt each tick (inherent to the timer→notify→full-render driver — same as the spike).

Recommended (a scoped follow-up, needs Instruments profiling + on-device visual verification — deliberately NOT attempted autonomously because it risks the deliberate visual gains of Task 6/5 and overlaps the deferred sweep-feather item):
- Profile which of {sweep, spinner, ring, full re-render} dominates.
- Candidate reductions: drop sweep to 2 stops in one path (single `paint_path`) if it holds visually; consider a cheaper spinner rotation; confirm the countdown truly stays 1 Hz (it does — measured +2/cycle).
- Re-measure clean-idle + Activity Monitor **Energy** tab for the authoritative energy read (top's POWER is only a relative proxy).

### Caveats on this measurement
- `top`'s POWER is a relative energy-impact proxy, not Joules; the authoritative energy sign-off is Activity Monitor's Energy tab on-device (deferred to the user's on-device pass).
- Single machine, single build; the spike's 1.7%/8.8% baseline was on the same machine, so the *comparison* is apples-to-apples, but a clean-idle re-run with a profiler is the basis for any optimization decision.

## Carry-forward
- **[finding → triage]** canvas-era per-card CPU ~30% over the div-era spike budget — accept (subtle-shimmer quality bought it, scales fine, no frame drops) OR do the scoped sweep/spinner profile+reduce above. **User/on-device call.**
- Overlaps the deferred Task 6 sweep-feather visual item (gpui AABB-gradient limitation) — both are "revisit the sweep" and are best addressed together in one on-device sweep pass.
