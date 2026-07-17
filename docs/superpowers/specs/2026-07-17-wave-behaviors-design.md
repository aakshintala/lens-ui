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
☾ Slept · ☕ Idle — these are the glyph *choices*.

**Icon set = Lucide SVGs, bundled via `AssetSource` (LOCKED 2026-07-17, overrides the
earlier "emoji now").** Rationale: the Working spinner already forces `AssetSource` + a
bundled SVG (gpui-component ships no icon SVGs — spike gotcha #3), and gpui-component's
`IconName`/`Icon` already *expects* Lucide-named SVGs ("Icons from Lucide", its README).
So we bundle all 8 from one MIT/ISC set and get **monochrome, status-color-tintable**
marks — emoji can't be tinted and fight the status-colored tile (§1–§2: tile color
carries status, glyph is the secondary mark). Lucide names: `bell` (NeedsInput),
`triangle-alert` (Failed), `loader-circle` (Working spinner), `eye`/`circle-dot`
(AwaitingReview), `alarm-clock` (Scheduled), `check` (Ready), `moon` (Slept), `coffee`
(Idle). A bespoke icon set later is a pure SVG swap, not an infra change. **Spinner
mechanism:** rotate `loader-circle.svg` via `with_transformation`, angle driven by the
same phase-from-clock as the sweep (NOT `.with_animation`) — one driver.

## 6. Structural shell (B1/B2/B4/B5) — port from the render SSOT

Mechanical port from `board-home.html`, no open design questions:
- **B1** — 44px status-colored **icon-tile** replaces the throwaway text pill (deletes
  `wave_label` / `pill_text_color` from `card/chrome.rs`; the tile hosts the glyph /
  spinner / countdown ring).
- **B2** — context-window **progress bar** (`.pbar`). **Fill = utilization threshold color,
  NOT the card's wave color** (as-built amendment, §11): green ≤50%, amber ≤75%, red above —
  a budget signal independent of status. Track stays `white 0.06`.
- **B4** — layout order: **tile-left** + stacked STATUS / title / harness·model. The
  harness·model line sits **inside the header meta column** (aligned under the title, past
  the tile), and the 44px tile is **vertically centered** against that 3-line stack (§11).
- **B5** — **Slept** dim + Wake button; **Failed** Retry affordance (currently faked as
  activity text). Real card is **280×160** (`CARD_WIDTH_PX`/`CARD_HEIGHT_PX`; grown from 148
  in the visual pass, §11); the mockup is 300×150.

## 7. Colors — placeholders, tuned at end of build

Current `lens-dark.json` status tokens (placeholders): ready `#4c8dff`, working
`#36c98a`, needs_input `#ff8a3d`, failed `#ff5d5d`, slept `#7a8493`, neutral `#eab308`,
scheduled `#8b9bf5`, awaiting_review `#c084fc`. The 4-way distinguishability
(ready/working/scheduled/awaiting_review) folds into the **one end-of-build tuning pass**
via the reload loop (`⌘⇧T`, `LENS_THEME_DIR=crates/lens-ui/src/theme`), for both
dark + light.

## 8. Animation architecture — LOCKED (spike-proven, `docs/spikes/2026-07-17-wave-animation.md`)

**Constraints:** (1) each card is mounted `.cached(style)` (keyed on bounds); (2) §4.4
forbids notifying `FleetStore`/siblings — the `session_card_view_observes_own_card_only`
isolation test must stay green; (3) static cards must not burn frames.

**Spike verdict = GO.** Proven on-device: `.cached()` cards *do* repaint under
`cx.notify(card_entity)` (via `request_animation_frame`), `paint == render` every frame,
and §4.4 isolation holds (static neighbors stayed flat while a card animated at 120fps —
a ~1400× paint separation).

**Driver = frame-capped timer self-notify (approach ②), NOT `.with_animation`.** The cost
data forced this: gpui's built-in `.with_animation` costs ~21% CPU for 5 cards because its
per-frame `request_layout` machinery is the hog. A timer-driven self-notify is ~40% cheaper
even before the fps cap.
- Per animating card: a `cx.spawn` loop —
  `background_executor().timer(tick).await → this.update(cx, |_,cx| cx.notify())` — held in
  an `Option<Task<()>>` that is **live only while the card's wave animates** (drop = cancel).
- **Frame cap ≈ 30fps** (`tick ≈ 33ms`). Saves ~40% vs native 120Hz; imperceptible.
- **Phase = pure fn of `UiClock::now_millis()`** → deterministically testable with
  `ManualUiClock`. ⚠ **Do the period modulo in i64 before casting to f32** — epoch-millis
  exceeds f32's mantissa and the phase freezes otherwise (spike gotcha #1).
- **Viewport-gate (build task):** only visible cards animate → cost bounded by screen
  capacity (~15 cards), NOT fleet size. **Approach ③ rejected:** a shared ticker all cards
  observe → re-renders all → fails §4.4.

**Measured cost (release, moving animation):** idle floor 0.3%; **~1.7% CPU per visible
animating card @30fps**; 5 cards = 8.8%. Managed by cap + viewport-gate.

**Countdown ring (Scheduled) is NOT in this loop** — 1 Hz redraw + one wake at
`scheduled_wake_at`; drawn via `canvas` arc (no conic-gradient in gpui).

### 8a. Build gotchas carried from the spike
1. **f32 epoch-millis precision** — i64 modulo before f32 cast (froze the whole animation).
2. **Sweep fidelity** — the div 2-stop gradient is a flat vertical bar; the real technique is
   **`canvas` + `window.paint_path`** (a skewed gradient parallelogram, `PathBuilder::fill()`,
   respects the clip mask), asset-free. Deferred to build tuning.
3. **Working spinner** — gpui-component `Spinner` needs `icons/loader.svg`, which the crate
   does NOT ship; bundle an SVG + `AssetSource`, or draw a canvas/rotated-svg arc. Currently
   a static ring placeholder.
4. **Board scroll (B6)** — the demo/board doesn't scroll; N>~2 clips cards. Out of this scope.

## 9. Testing approach

- **Unit (deterministic):** phase-from-`UiClock` is a pure function → assert sweep
  offset / countdown fraction at chosen `now` with `ManualUiClock`. `derive_wave` ladder
  tests already exist.
- **Isolation (acceptance):** extend `session_card_view_observes_own_card_only` — an
  animating neighbor must not bump a static card's `paint_count`.
- **On-device:** the spike's CPU/GPU + visual-smoothness pass; end-of-build color tuning
  via the reload loop.
- **Perf/energy completion gate (REQUIRED — end of the whole build, not a footnote):**
  re-run the spike's exact rig — release build, `top -l N -s 1 -pid <pid> -stats
  cpu,power` for CPU% + energy-impact, FPS from paint-count deltas — at the default
  8-card demo (+ `LENS_DEMO_N=2` for headroom). **Regression budget** (from the spike, so
  it's a gate not a vibe check): idle floor ~0.3%, ~1.7% CPU per visible animating card
  @30fps, ~8.8% for 5. If the productionized custom draw (canvas `paint_path` sweep,
  canvas countdown arc, rotated spinner SVG) pushes materially past that, it's a finding,
  not a pass. Full-scale (>8 cards, scrolling) rides with B6 — this build validates the
  visible set only.

## 10. Out of scope (this build)

Board packing B6–B8 (adaptive grid, ordinal slots, group lanes); the Canvas deep-link
producer (SPEC-GAPS #11); the wake-*firing* scheduler (#11 — the wave only needs the
repaint at T, not the firing); bespoke icon set; light-theme final tuning (folds into the
one end-of-build pass).

## 11. On-device visual pass — as-built amendments (2026-07-17)

Decisions taken during the on-device visual acceptance pass. These **supersede the mockup**
(`board-home.html`) where they conflict; the mockup remains the SSOT for anything not listed
here. All are on `feat/lens-app-multi-session`, gate-green.

- **Card height 148 → 160** (`CARD_HEIGHT_PX`). The 6-row stack + icon tile + pbar was too
  tight at 148 once line-heights were corrected (below); 160 gives margin without collapse.
- **Line-height fix** — gpui's `text_*` helpers set font-size but not line-height, so
  `overflow_hidden` (required for ellipsis) shaved ascenders on single-line rows. `ellipsize_line`
  now sets `line_height(1.4)`; the activity row uses `min_h(16)` + `text_xs`.
- **Context bar = utilization threshold color** (not wave color): green ≤50%, amber ≤75%,
  red above (`t.base.success/warning/danger`). See §6 B2.
- **Repo row uses Lucide SVG glyphs** — `folder` + `git-branch` (bundled ISC, tinted via
  `text_color`) replace the `📁`/`⑂` emoji/fork text, for consistency with the tile glyphs.
  Rendered as a flex row (`repo_entry`/`render_repos_row`), not a formatted string. The
  multi-repo overflow tooltip (`·+N`) uses the same glyphs and is **gated to `repos.len() > 1`**
  (a single-repo tooltip only duplicates the visible line). Tooltips use gpui-component's
  themed `Tooltip` box.
- **Harness·model in the header meta** — moved into the title column (aligned under the title),
  and the tile is vertically centered against the resulting 3-line stack. The wrapper carries
  **no `overflow_hidden`** — the Scheduled countdown ring's `inset(-4)` canvas must not clip.
- **Title tooltip** — the ellipsized title reveals its full text on hover (gated to a real title).
- **Board grid** (`board/mod.rs`) — gap `28px` (≥ 2× the expanding-ring's 12px reach, so
  neighbors' breathe animations don't bleed), `28px` padding, `justify_center` + `content_start`.
  The demo window is sized (`run_demo`, demo-only) so the 8 cards land as a centered 4×2.
  A responsive/scrolling grid still rides with B6 (§8a.4).
