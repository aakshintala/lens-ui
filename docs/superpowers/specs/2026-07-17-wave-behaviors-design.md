# Wave behaviors — design (B1–B5 wave build)

**Date:** 2026-07-17 · **Branch:** `feat/lens-app-multi-session`
**Status:** Visual design **LOCKED** (this doc). Animation **architecture** = approach ②
recommended, **PENDING on-device spike** (`docs/spikes/2026-07-17-wave-animation.md`).
Full plan (`writing-plans`) follows the spike verdict.

Supersedes the "Deferred — DO in B1–B5" behavior placeholders in
`docs/handoffs/2026-07-16-wave-states-shipped.md`. Structural contract (the `Wave`
8-state enum, ladder, tokens, `--demo` cards) already shipped @ `f1f0d6e`.

Visual SSOT: `docs/design/renders/wave-states-motion.html` (live, tunable mockup).

---

## 1. What the wave is for

Peripheral awareness: glance at the board and know which sessions need you. The
hazard is over-motion — if every card moves, none draws the eye. So **motion is a
scarce signal**, and its *character* (not a fine speed gradient) encodes meaning.

## 2. Core principle — Strategy B: motion character encodes CLASS

| Motion | Meaning |
|---|---|
| **Linear sweep** (band travels across the card) | *Your attention is wanted* |
| **Rotation** (spinner) | *The machine is working* |
| **Depleting ring** (arc around the tile) | *Self-driving — will resume itself* |
| **Stillness** | Dormant / nothing notable |

This beat the earlier "one sweep primitive, four speed tiers" model: adjacent speeds
(1.5s vs 2.0s) were perceptually indistinguishable, so Ready/AwaitingReview/Working
all read alike. Distinct *characters* separate them; speed only sub-divides within the
sweep class (loud vs soft).

## 3. The 8-state motion sheet (LOCKED)

| Wave | Ladder | Motion | Speed | Glyph | Card treatment |
|---|---|---|---|---|---|
| **NeedsInput** | 1 | sweep **+ expanding ring** | 1.0s / ring 2.4s | 🔔 | normal |
| **Failed** | 2 | sweep **+ expanding ring** | 1.0s / ring 2.4s | ⚠ | normal |
| **Working** | 3 | **spinner only** (no sweep) | spin ~2.0s | spinner | normal |
| **AwaitingReview** | 4 | sweep | 1.5s | ⌾ | normal |
| **Scheduled** | 5 | **depleting countdown ring** | 1 Hz redraw | ⏰ | normal + live countdown |
| **Ready** | 6 | sweep | 1.5s | ✓ | normal |
| **Slept** | 7 | none | — | ☾ | **content dimmed** + bright **Wake** |
| **Neutral/Idle** | 8 | none | — | ☕ | still |

- Sweep **amplitude = 0.4** everywhere (soft; must not fight text legibility).
- NeedsInput & Failed are **equal severity** ("need you now") → identical motion; the
  **ring takes each state's status color** so orange (NeedsInput) vs red (Failed) stay
  distinguishable.
- AwaitingReview & Ready are the **same "done, your turn" class** → same sweep; they
  separate only by color (purple ⌾ / blue ✓). Intentional — they're siblings.
- The `derive_wave` ladder is unchanged (already shipped): `NeedsInput > Failed >
  Working > AwaitingReview > Scheduled > Ready > Slept > Neutral`.

## 4. Per-state behavior detail

**Sweep primitive.** A soft light band sweeps across the card surface (skewed, ~48%
card width, translating left→right over the period). Built as a translating overlay
child clipped to the card (`overflow_hidden`); the **ring lives *outside* the clip** as
a sibling so it can extend past the border.

**NeedsInput / Failed — the loud pair.** Sweep (1.0s) + a second **expanding ring**
just outside the card border (`inset −2px → −12px`, opacity 0.9 → 0, period 2.4s),
status-colored. The ring is the "right now" escalator layered on the base sweep — *not*
a bigger sweep amplitude.

**AwaitingReview / Ready — the soft pair.** Sweep (1.5s), no ring.
- AwaitingReview's "deep-link to the Canvas artifact" affordance is **DEFERRED** —
  Canvas doesn't exist yet (it's a SPEC-GAPS #11 producer). Ship the wave + ⌾ +
  "AWAITING REVIEW" now; clicking the card focuses the session like any card. No fake
  affordance.
- Ready keeps its existing lifecycle (idle + recent completion, decays after
  `READY_DECAY_MS`, suppressed on focus). The sweep runs while the wave is active.

**Working — spinner only.** Its signature is **rotation**, no sweep. Use gpui-component's
`Spinner`, or a rotating `svg().with_transformation(rotate)`, tinted the working color.

**Scheduled — depleting countdown ring.** No sweep (not attention-wanted). A **thin
depleting arc** around the ⏰ tile encodes real time-to-wake:
`fraction = (wake − now) / (wake − scheduled_at)`, drawn full→empty. Live text in the
activity slot: **"wakes in 2m 59s"** ticking to **"wakes in 45s"** → "waking…".
- **Redraw cadence = 1 Hz** (once per second) while in the Scheduled wave — *not* 60fps
  (imperceptible per-second motion doesn't need it) and *not* the 10s poll tick (too
  steppy for ticking text). One self-notify/sec per scheduled card; cheap.
- A **repaint at `scheduled_wake_at`** (gpui executor timer, mirroring how
  `READY_DECAY_MS` schedules its decay wake) flips the card out of Scheduled at T. The
  arc is drawn via `canvas`/path (gpui has no conic-gradient) — the most custom draw of
  the set, but **not** in the 60fps animation loop.

**Slept — dim + Wake.** No motion. **Content dimmed** (~0.42 opacity) but the **Wake
button is NOT dimmed** (bright, top-right). ☾ glyph. (NB: dim individual children, not a
parent `opacity`, so the button can stay full-opacity.)

**Neutral/Idle — still.** Fully static, ☕. No motion, no dim.

## 5. Glyphs (finalized this pass — NOT deferred)

🔔 NeedsInput · ⚠ Failed · spinner Working · ⌾ AwaitingReview · ⏰ Scheduled · ✓ Ready ·
☾ Slept · ☕ Idle. Unicode/emoji now; a bespoke icon set is later polish (a build
artifact, per STATUS). The spike verifies emoji render correctly in gpui text.

## 6. Structural shell (B1/B2/B4/B5) — port from the render SSOT

Mechanical port from `board-home.html`, no open design questions:
- **B1** — 44px status-colored **icon-tile** replaces the throwaway text pill (deletes
  `wave_label` / `pill_text_color` from `card/chrome.rs`; the tile hosts the glyph /
  spinner / countdown ring).
- **B2** — context-window **progress bar** (`.pbar`, status-colored fill).
- **B4** — layout order: **tile-left** + stacked STATUS / title / harness·model.
- **B5** — **Slept** dim + Wake button; **Failed** Retry affordance (currently faked as
  activity text). Real card is **280×148** (`CARD_WIDTH_PX`/`CARD_HEIGHT_PX`); the mockup
  is 300×150.

## 7. Colors — placeholders, tuned at end of build

Current `lens-dark.json` status tokens (placeholders): ready `#4c8dff`, working
`#36c98a`, needs_input `#ff8a3d`, failed `#ff5d5d`, slept `#7a8493`, neutral `#eab308`,
scheduled `#8b9bf5`, awaiting_review `#c084fc`. The 4-way distinguishability
(ready/working/scheduled/awaiting_review) folds into the **one end-of-build tuning pass**
via the reload loop (`⌘⇧T`, `LENS_THEME_DIR=crates/lens-ui/src/theme`), for both
dark + light.

## 8. Animation architecture — approach ② (recommended), PENDING SPIKE

**Constraints:** (1) each card is mounted `.cached(style)` (keyed on bounds); (2) §4.4
forbids notifying `FleetStore`/siblings — the `session_card_view_observes_own_card_only`
isolation test must stay green; (3) static cards (Idle/Slept/Neutral/Scheduled) must not
burn frames.

**Approach ② — self-notify loop driven by `UiClock`.** Animating cards call
`window.request_animation_frame()`, which (per gpui 0.2.2 source) captures
`current_view()` and `cx.notify(card_entity)` on the next frame — **self-repaint only,
§4.4-safe by construction**. Phase (sweep offset, spinner angle, countdown fraction) is a
**pure function of `clock.now_millis()`**, matching the existing `derive_wave` +
Ready-decay dual-clock pattern → deterministically testable with `ManualUiClock`. Static
cards schedule nothing (zero cost).

- gpui's built-in `.with_animation` (`AnimationElement`) *also* uses
  `request_animation_frame` internally and computes delta from elapsed `Instant` — same
  primitive, less code, but wall-clock (not `UiClock`) so not test-controllable. Trade
  decided in the plan; both converge on the same repaint primitive.
- **Approach ① fallback:** built-in `.with_animation`. **Approach ③ rejected:** a shared
  ticker entity all cards observe re-renders *all* cards → fails §4.4.

**The one unproven thing (why we spike first):** does `cx.notify(card_entity)` (via
`request_animation_frame`) actually invalidate and repaint the `.cached(style)` wrapper
each frame, or does the cache freeze the animation? If the cache freezes it, pivot
(drop `.cached()` for animating cards, or force invalidation) — documented in the spike.

**The spike must establish (on-device):**
(a) a self-scheduled repaint actually re-paints a `.cached()` card each frame;
(b) the isolation test stays green with a neighbor animating (drive the assertion off the
    card view's existing `paint_count` — static cards' counts stay flat);
(c) sustained **CPU% and GPU%** at 1 / 5 / 10 / 20 simultaneously-animating cards (also
    seeds the deferred lens-ui render benchmark).

**Countdown ring is NOT in the 60fps loop** — 1 Hz redraw + one wake at
`scheduled_wake_at`. It sidesteps the cache-repaint question entirely.

## 9. Testing approach

- **Unit (deterministic):** phase-from-`UiClock` is a pure function → assert sweep
  offset / countdown fraction at chosen `now` with `ManualUiClock`. `derive_wave` ladder
  tests already exist.
- **Isolation (acceptance):** extend `session_card_view_observes_own_card_only` — an
  animating neighbor must not bump a static card's `paint_count`.
- **On-device:** the spike's CPU/GPU + visual-smoothness pass; end-of-build color tuning
  via the reload loop.

## 10. Out of scope (this build)

Board packing B6–B8 (adaptive grid, ordinal slots, group lanes); the Canvas deep-link
producer (SPEC-GAPS #11); the wake-*firing* scheduler (#11 — the wave only needs the
repaint at T, not the firing); bespoke icon set; light-theme final tuning (folds into the
one end-of-build pass).
