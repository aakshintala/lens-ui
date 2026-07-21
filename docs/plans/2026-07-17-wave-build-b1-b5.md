# Wave Build (B1–B5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the `SessionCard` anatomy (B1 icon-tile, B2 progress bar, B5 Slept-dim/Wake + Failed-Retry) and productionize the wave-motion system (Lucide glyphs, canvas sweep, rotating spinner, Scheduled countdown ring, viewport-gated 30fps driver) against the LOCKED design.

**Architecture:** Each card is a `SessionCardView` rendering `SessionCard` state, mounted inside a `.cached(280×148)` wrapper. Animation is driven by a **per-card frame-capped timer** (`cx.spawn` loop → `cx.notify` self-only, §4.4-safe) that re-renders the card; every animated visual is a **pure function of `UiClock::now_millis()`** (deterministically testable, i64-modulo-before-f32-cast). Glyphs are **Lucide SVGs** bundled via a gpui `AssetSource` and tinted through `svg().text_color(status)`. The sweep and Scheduled countdown ring are drawn with gpui `canvas` + `PathBuilder`/`paint_path`.

**Tech Stack:** Rust · gpui `0.2.2` · gpui-component `0.5.1` · Lucide icons (ISC) · `rusqlite`-backed stores (unaffected here).

## Global Constraints

Every task's requirements implicitly include these. Copy exact values.

- **Versions:** gpui `0.2.2`, gpui-component `0.5.1`. No new heavy deps (Lucide = static SVG bytes, no crate).
- **§4.4 render isolation:** a `SessionCardView` observes ONLY its own `card` entity (`cx.observe(&card, …)` at `view.rs:56`). The animation driver self-notifies via `this.update(cx, |_, cx| cx.notify())` and MUST NOT notify `FleetStore` or sibling cards. The acceptance test `session_card_view_observes_own_card_only` (`view.rs`) must stay green.
- **Fixed card geometry:** `CARD_WIDTH_PX = 280.0`, `CARD_HEIGHT_PX = 148.0` (`card/model.rs:9-10`). The `.cached()` wrapper is pinned to that (`view.rs:169-177`); do not change it.
- **Phase math:** every animated quantity is `f(UiClock::now_millis())`. **Do the period modulo in `i64` BEFORE casting to `f32`** — epoch-millis (~1.8e12) exceeds f32's 24-bit mantissa and the phase freezes otherwise (`motion.rs:19-25` shows the correct pattern; copy it).
- **Driver:** timer self-notify, NOT `.with_animation` (measured ~21% CPU/5cards; the timer @30fps is ~8.8%). Held in `Option<gpui::Task<()>>`, live only while the wave animates (drop = cancel).
- **Frame cap:** 30fps (`tick = 33ms`) for the sweep/spinner/ring class; **1 Hz** (`1000ms`) for the Scheduled countdown. The env override of the cap exists ONLY under the `demo` cargo feature (Task 10).
- **Colors are PLACEHOLDERS.** Never hardcode raw hex at a call site — read `t.status.*` / `t.base.*` tokens and derive tints with `gpui_component::Colorize::opacity`/`.mix` (already imported patterns: `chrome.rs:123` `t.base.overlay.opacity(0.55)`). Final color/light tuning is one end-of-build pass via the reload loop (`⌘⇧T`, `LENS_THEME_DIR=crates/lens-ui/src/theme`).
- **Visual verification is on-device, not pixel-asserted.** Under `#[gpui::test]`/`TestAppContext` the text/SVG system is a `NoopTextSystem` — font/shape/paint asserts are false-green (memory `gpui-test-noop-text-system`). Testable units = the **pure functions** (phase, fraction, format, icon-path). Visuals are checked by running `cargo run -p lens-app --release --features demo -- --demo` and looking, plus the compile/clippy gate.
- **Gate (must pass at every commit):** `cargo xtask gate` (fmt --check + clippy -D warnings + tests for the production crates). Never pipe it through `tail` (masks the exit code — memory `xtask-gate-scope`).
- **Reference SSOTs (use these for every visual value):** motion params `docs/design/renders/wave-states-motion.html`; card structure `docs/design/renders/board-home.html`; the design spec `docs/specs/2026-07-17-wave-behaviors-design.md`.

---

## File Structure

**Created:**
- `crates/lens-ui/assets/icons/*.svg` — 8 Lucide glyph SVGs (bell, triangle-alert, loader-circle, alarm-clock, check, moon, coffee, circle-dot) + `LICENSE.lucide` (ISC).
- `crates/lens-ui/src/assets.rs` — `LensAssets` (gpui `AssetSource`) serving the embedded SVG bytes.

**Modified:**
- `crates/lens-ui/src/lib.rs` — `pub mod assets;`.
- `crates/lens-ui/src/card/motion.rs` — icon-path map, `spin_period`/`spin_phase`, `anim_tick_for`, canvas sweep, countdown fraction + wake-countdown format, spinner render.
- `crates/lens-ui/src/card/chrome.rs` — icon-tile (svg glyph + faint-tint bg), `.pbar`, Slept-dim, Wake/Retry action button, Scheduled countdown-ring host + live text.
- `crates/lens-ui/src/card/view.rs` — driver generalized to `anim_tick_for` + viewport-gate; new `on_wake` handler; retry rewired to the seam; demo-gated env knobs; eprintln removed.
- `crates/lens-ui/src/card/mod.rs` — exports.
- `crates/lens-ui/src/fleet/store.rs` — `wake_session` / `retry_session` seams.
- `crates/lens-app/src/main.rs` — register `LensAssets`; `demo`-feature-gate the demo path + `LENS_DEMO_N`.
- `crates/lens-ui/Cargo.toml`, `crates/lens-app/Cargo.toml` — `demo` feature.

---

## Task 1: Lucide asset infrastructure (`AssetSource`)

Foundation for every glyph (Task 2) and the spinner (Task 5). Without a registered `AssetSource`, gpui `svg()` renders nothing (gpui-component ships no SVGs).

**Files:**
- Create: `crates/lens-ui/assets/icons/{bell,triangle-alert,loader-circle,alarm-clock,check,moon,coffee,circle-dot}.svg`
- Create: `crates/lens-ui/assets/icons/LICENSE.lucide`
- Create: `crates/lens-ui/src/assets.rs`
- Modify: `crates/lens-ui/src/lib.rs`
- Modify: `crates/lens-app/src/main.rs:76` (live `Application::new()`), `:113` (demo `Application::new()`)
- Test: `crates/lens-ui/src/assets.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `lens_ui::assets::LensAssets` (unit struct, `impl gpui::AssetSource`); constant `lens_ui::assets::ICON_PATHS: [&str; 8]`. Glyph SVGs are addressable as `"icons/<name>.svg"`.

- [ ] **Step 1: Add the 8 Lucide SVGs (ISC).** Download each icon's SVG from https://lucide.dev (or the `lucide-static` package `icons/<name>.svg`) — exact node names: `bell`, `triangle-alert`, `loader-circle`, `alarm-clock`, `check`, `moon`, `coffee`, `circle-dot`. Save verbatim to `crates/lens-ui/assets/icons/<name>.svg`. Keep the Lucide format (stroke-based, `stroke="currentColor"`, `fill="none"`) — gpui renders an SVG as an alpha mask tinted by the element's `text_color`, so stroke coverage becomes the tinted glyph. Reference shape (this is exactly the Lucide `bell.svg` payload — the others follow the identical wrapper):

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326"/></svg>
```

Add `crates/lens-ui/assets/icons/LICENSE.lucide` containing the Lucide ISC license text (from https://github.com/lucide-icons/lucide/blob/main/LICENSE).

- [ ] **Step 2: Write the failing test** — `crates/lens-ui/src/assets.rs`:

```rust
//! Embedded asset provider (gpui `AssetSource`). Serves the Lucide glyph SVGs the
//! card tile + spinner render via `svg().path("icons/<name>.svg")`. gpui-component
//! ships no icon SVGs, so the app MUST register this before any card paints.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Every glyph path served by `LensAssets`. Keep in sync with the files under
/// `assets/icons/`; `card::motion::wave_icon_path` returns members of this set.
pub const ICON_PATHS: [&str; 8] = [
    "icons/bell.svg",
    "icons/triangle-alert.svg",
    "icons/loader-circle.svg",
    "icons/alarm-clock.svg",
    "icons/check.svg",
    "icons/moon.svg",
    "icons/coffee.svg",
    "icons/circle-dot.svg",
];

/// Compile-time-embedded (path, bytes) table for the bundled Lucide SVGs.
const ICON_BYTES: &[(&str, &[u8])] = &[
    ("icons/bell.svg", include_bytes!("../assets/icons/bell.svg")),
    ("icons/triangle-alert.svg", include_bytes!("../assets/icons/triangle-alert.svg")),
    ("icons/loader-circle.svg", include_bytes!("../assets/icons/loader-circle.svg")),
    ("icons/alarm-clock.svg", include_bytes!("../assets/icons/alarm-clock.svg")),
    ("icons/check.svg", include_bytes!("../assets/icons/check.svg")),
    ("icons/moon.svg", include_bytes!("../assets/icons/moon.svg")),
    ("icons/coffee.svg", include_bytes!("../assets/icons/coffee.svg")),
    ("icons/circle-dot.svg", include_bytes!("../assets/icons/circle-dot.svg")),
];

/// gpui `AssetSource` over the embedded Lucide glyphs. Register once:
/// `Application::new().with_assets(LensAssets)`.
pub struct LensAssets;

impl AssetSource for LensAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICON_BYTES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICON_BYTES
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_path_loads_nonempty_svg() {
        for path in ICON_PATHS {
            let bytes = LensAssets
                .load(path)
                .expect("load ok")
                .unwrap_or_else(|| panic!("missing asset: {path}"));
            assert!(!bytes.is_empty(), "empty svg: {path}");
            assert!(
                bytes.windows(4).any(|w| w == b"<svg"),
                "not an svg: {path}"
            );
        }
    }

    #[test]
    fn list_icons_dir_returns_all_eight() {
        let listed = LensAssets.list("icons/").expect("list ok");
        assert_eq!(listed.len(), 8, "listed: {listed:?}");
    }

    #[test]
    fn unknown_path_is_none_not_err() {
        assert!(LensAssets.load("icons/nope.svg").expect("ok").is_none());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib assets::tests`
Expected: FAIL — `assets` module not declared (unresolved `lens_ui::assets`) / `include_bytes!` missing files if SVGs absent.

- [ ] **Step 4: Declare the module.** In `crates/lens-ui/src/lib.rs`, add alongside the other `pub mod` lines (alphabetical):

```rust
pub mod assets;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p lens-ui --lib assets::tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Register the asset source in the app.** In `crates/lens-app/src/main.rs`, both `Application::new()` sites become `Application::new().with_assets(lens_ui::assets::LensAssets)`:
  - `main.rs:76` — `Application::new().run(move |cx: &mut App| {` → `Application::new().with_assets(lens_ui::assets::LensAssets).run(move |cx: &mut App| {`
  - `main.rs:113` — `Application::new().run(|cx: &mut App| {` → `Application::new().with_assets(lens_ui::assets::LensAssets).run(|cx: &mut App| {`

- [ ] **Step 7: Verify build + gate**

Run: `cargo build -p lens-app && cargo xtask gate`
Expected: builds; gate green.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-ui/assets crates/lens-ui/src/assets.rs crates/lens-ui/src/lib.rs crates/lens-app/src/main.rs
git commit -m "feat(card): bundle Lucide glyph SVGs via a gpui AssetSource"
```

---

## Task 2: B1 — icon-tile renders a tinted Lucide glyph on a faint tint

Replaces the emoji glyph with a status-tinted SVG, and fixes the tile background: the seed fills the tile with the **solid** status color (`chrome.rs:79 .bg(border)`), but both references show a **faint ~14% tint** with the glyph in full status color (`wave-states-motion.html:43-45`, `board-home.html:52`).

**Files:**
- Modify: `crates/lens-ui/src/card/motion.rs:107-119` (replace `wave_glyph`)
- Modify: `crates/lens-ui/src/card/chrome.rs:73-89` (`render_icon_tile`), `:10` (import)
- Modify: `crates/lens-ui/src/card/mod.rs` (exports, if `wave_glyph` was re-exported — it is not; internal only)
- Test: `crates/lens-ui/src/card/motion.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `lens_ui::assets` icon paths (Task 1); `Wave::status_color` (`wave.rs:70`).
- Produces: `motion::wave_icon_path(wave: Wave) -> Option<&'static str>` — `Some("icons/<name>.svg")` for a glyph wave, `None` for `Working` (Working uses the spinner, Task 5). Consumed by `chrome::render_icon_tile` and Task 5.

- [ ] **Step 1: Write the failing test** — add to `crates/lens-ui/src/card/motion.rs` (new `tests` module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_icon_path_maps_every_glyph_wave() {
        assert_eq!(wave_icon_path(Wave::NeedsInput), Some("icons/bell.svg"));
        assert_eq!(wave_icon_path(Wave::Failed), Some("icons/triangle-alert.svg"));
        assert_eq!(wave_icon_path(Wave::AwaitingReview), Some("icons/circle-dot.svg"));
        assert_eq!(wave_icon_path(Wave::Scheduled), Some("icons/alarm-clock.svg"));
        assert_eq!(wave_icon_path(Wave::Ready), Some("icons/check.svg"));
        assert_eq!(wave_icon_path(Wave::Slept), Some("icons/moon.svg"));
        assert_eq!(wave_icon_path(Wave::Neutral), Some("icons/coffee.svg"));
    }

    #[test]
    fn working_has_no_static_glyph() {
        assert_eq!(wave_icon_path(Wave::Working), None);
    }

    #[test]
    fn every_glyph_path_is_a_bundled_asset() {
        for wave in [
            Wave::NeedsInput, Wave::Failed, Wave::AwaitingReview, Wave::Scheduled,
            Wave::Ready, Wave::Slept, Wave::Neutral,
        ] {
            let path = wave_icon_path(wave).unwrap();
            assert!(
                crate::assets::ICON_PATHS.contains(&path),
                "{wave:?} → {path} not in ICON_PATHS"
            );
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib card::motion::tests`
Expected: FAIL — `wave_icon_path` not found.

- [ ] **Step 3: Replace `wave_glyph` with `wave_icon_path`.** In `crates/lens-ui/src/card/motion.rs`, delete `wave_glyph` (`:107-119`) and add:

```rust
/// Lucide glyph asset path for the tile, tinted at the call site via `text_color`.
/// `None` for `Working` — it renders the rotating spinner instead (see `render_working_spinner`).
pub fn wave_icon_path(wave: Wave) -> Option<&'static str> {
    Some(match wave {
        Wave::NeedsInput => "icons/bell.svg",
        Wave::Failed => "icons/triangle-alert.svg",
        Wave::AwaitingReview => "icons/circle-dot.svg",
        Wave::Scheduled => "icons/alarm-clock.svg",
        Wave::Ready => "icons/check.svg",
        Wave::Slept => "icons/moon.svg",
        Wave::Neutral => "icons/coffee.svg",
        Wave::Working => return None,
    })
}
```

- [ ] **Step 4: Rewrite `render_icon_tile`.** In `crates/lens-ui/src/card/chrome.rs`, update the import at `:10` (drop `wave_glyph`, add `wave_icon_path`; keep `render_working_spinner`), and replace `render_icon_tile` (`:73-89`):

```rust
use super::motion::{render_sweep_overlay, render_working_spinner, wave_icon_path, wave_status_line};
```

```rust
fn render_icon_tile(wave: Wave, status: Hsla) -> Div {
    // Faint status-tinted surface (mockup: color-mix(status 14%, bg2)) — NOT a solid fill.
    let mut tile = div()
        .flex_shrink_0()
        .w(px(44.0))
        .h(px(44.0))
        .rounded(px(11.0))
        .bg(status.opacity(0.14))
        .border_1()
        .border_color(status.opacity(0.30))
        .flex()
        .items_center()
        .justify_center();
    if let Some(path) = wave_icon_path(wave) {
        tile = tile.child(
            svg()
                .path(path)
                .w(px(21.0))
                .h(px(21.0))
                .text_color(status),
        );
    } else {
        // Working → spinner (Task 5 makes this rotate; a static ring until then).
        tile = tile.child(render_working_spinner(status));
    }
    tile
}
```

Add `svg` to the gpui import at `chrome.rs:1-4` (append `svg` to the `use gpui::{…}` list).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p lens-ui --lib card::motion::tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Verify build + on-device glyphs**

Run: `cargo run -p lens-app --release -- --demo`
Expected: each tile shows a **line-icon glyph tinted to the status color** on a faint tinted square (bell/triangle/check/moon/coffee/circle-dot/alarm-clock). If a glyph is invisible, the SVG tinting assumption is wrong — check that `svg().text_color(...)` tints (it should render the stroke as a mask); confirm the asset registered (Task 1 Step 6). Working still shows the static ring (Task 5).

- [ ] **Step 7: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/motion.rs crates/lens-ui/src/card/chrome.rs
git commit -m "feat(card): B1 tile renders tinted Lucide glyph on faint tint (was emoji + solid fill)"
```

---

## Task 3: B2 — context-window progress bar (`.pbar`)

Adds the status-colored progress bar under the foot row. Reference: `board-home.html:75-76,131` — a 4px track (`rgba(255,255,255,.06)`) with a status-colored fill at `ctx%`.

**Files:**
- Modify: `crates/lens-ui/src/card/chrome.rs` (extend `format_ctx_pct` neighborhood + append the bar to `root`)
- Test: `crates/lens-ui/src/card/chrome.rs` tests (add a fraction-math unit)

**Interfaces:**
- Produces: `chrome::ctx_fraction(context_window: Option<u64>, last_total_tokens: Option<u64>) -> f32` — clamped 0.0..1.0; the pbar fill width. `format_ctx_pct` (`:57-62`) stays for the text.

- [ ] **Step 1: Write the failing test** — add to the `#[cfg(test)] mod tests` in `crates/lens-ui/src/card/chrome.rs`:

```rust
    #[test]
    fn ctx_fraction_clamps_and_ratios() {
        assert_eq!(ctx_fraction(Some(200_000), Some(50_000)), 0.25);
        assert_eq!(ctx_fraction(Some(100), Some(250)), 1.0, "over-full clamps to 1");
        assert_eq!(ctx_fraction(None, Some(10)), 0.0, "no window → 0");
        assert_eq!(ctx_fraction(Some(0), Some(10)), 0.0, "zero window → 0 (no div-by-0)");
        assert_eq!(ctx_fraction(Some(200_000), None), 0.0, "no tokens → 0");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib card::chrome::tests::ctx_fraction_clamps_and_ratios`
Expected: FAIL — `ctx_fraction` not found.

- [ ] **Step 3: Add `ctx_fraction`.** In `crates/lens-ui/src/card/chrome.rs`, next to `format_ctx_pct`:

```rust
/// Fill fraction (0.0..1.0) for the context-window progress bar.
fn ctx_fraction(context_window: Option<u64>, last_total_tokens: Option<u64>) -> f32 {
    match (context_window, last_total_tokens) {
        (Some(w), Some(t)) if w > 0 => (t as f32 / w as f32).clamp(0.0, 1.0),
        _ => 0.0,
    }
}
```

- [ ] **Step 4: Render the bar.** In `render_card_chrome`, compute the fraction near the other `let` bindings (after `:130` `let ctx_pct = …`):

```rust
    let ctx_frac = ctx_fraction(card.context_window, card.last_total_tokens);
    let pbar_track = gpui::white().opacity(0.06);
```

Then append the bar as the LAST child of `root` — after the foot `.child(...)` block that ends at `:250`, before the `if let Some(phase) = sweep_phase` block (`:252`). Insert:

```rust
    root = root.child(
        div()
            .h(px(4.0))
            .w_full()
            .rounded(px(2.0))
            .overflow_hidden()
            .bg(pbar_track)
            .child(
                div()
                    .h_full()
                    .w(relative(ctx_frac))
                    .bg(border),
            ),
    );
```

Add `relative` to the gpui import at `chrome.rs:1-4` (append to the `use gpui::{…}` list). `relative(f)` yields a `DefiniteLength` = fraction of the parent width (gpui `geometry.rs`).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p lens-ui --lib card::chrome::tests`
Expected: PASS.

- [ ] **Step 6: Verify on-device + gate**

Run: `cargo run -p lens-app --release -- --demo` → each card shows a thin status-colored bar (needs-input ~ shows its %, failed ~0%). Then `cargo xtask gate`.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-ui/src/card/chrome.rs
git commit -m "feat(card): B2 context-window progress bar"
```

---

## Task 4: B5 — Slept dim + Wake button, Failed Retry button (visual + wired seam)

Reference: `wave-states-motion.html:65-68` (`.wake` button), `board-home.html:201,227` (Retry/Resume in `.cact`), `:77` (`.dim{opacity:.5}`), and the design decision **"visual + wired seam"**: render the buttons; wire them to real `FleetStore` seam methods that currently no-op (behavior lands with the state-model wake=respawn plumbing). Slept dims **individual children** (not a parent opacity) so the Wake button stays bright (spec §4).

**Files:**
- Modify: `crates/lens-ui/src/fleet/store.rs` (add `wake_session`, `retry_session`)
- Modify: `crates/lens-ui/src/card/chrome.rs` (`render_card_chrome`: `on_wake` param, action button, Slept child-dim, Failed activity → error text)
- Modify: `crates/lens-ui/src/card/view.rs` (pass `on_wake`; rewire retry to the seam)

**Interfaces:**
- Consumes: `SessionCard.last_task_error` (`model.rs:45`), `SessionCard.lifecycle` (`:46`).
- Produces:
  - `FleetStore::wake_session(&self, id: &SessionId)` and `FleetStore::retry_session(&self, id: &SessionId)` — seams (no-op TODO).
  - `render_card_chrome` gains a new closure param `on_wake: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static` (inserted after `on_kebab_toggle` in the signature so the retry param keeps its meaning). Callers (view.rs) pass all five.

- [ ] **Step 1: Add the FleetStore seams.** In `crates/lens-ui/src/fleet/store.rs`, next to `send_session_command` (`:64`):

```rust
    /// Wake a Slept session. SEAM: real behavior = respawn the actor from the
    /// persisted connection context (state-model wake=respawn), which FleetStore
    /// does not yet retain. TODO(state-model P3+): re-run `spawn_live_session`.
    pub fn wake_session(&self, _id: &SessionId) {
        // Intentional no-op until the wake=respawn plumbing lands. The button is a
        // real affordance wired to this seam, not a dead element.
    }

    /// Retry a Failed session. SEAM: real behavior = re-poke / respawn the session.
    /// TODO(state-model P3+): route to the actual retry path once it exists.
    pub fn retry_session(&self, _id: &SessionId) {
        // Intentional no-op — see `wake_session`.
    }
```

- [ ] **Step 2: Verify seams compile**

Run: `cargo build -p lens-ui`
Expected: builds (unused-var lint satisfied by the `_id` prefix).

- [ ] **Step 3: Add the action button + Slept dim to chrome.** In `crates/lens-ui/src/card/chrome.rs`:

(a) Extend the signature — add `on_wake` right after `on_kebab_toggle` (`:113`):

```rust
    on_kebab_toggle: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_wake: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_sleep: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_send: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_retry: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
```

(b) Failed activity shows the ERROR (not "Retry"); Retry moves to the button. Replace the `let activity = …` block (`:133-137`):

```rust
    let dim = wave == Wave::Slept;
    let activity = if wave == Wave::Failed {
        card.last_task_error
            .as_ref()
            .map(|e| format!("✕ {}", e.message))
            .unwrap_or_else(|| "failed".into())
    } else {
        card.activity_summary.clone()
    };
```

(c) Build the optional top-right action button. Insert before `let mut root = …` (`:139`):

```rust
    // Slept → bright Wake; Failed → Retry. Both sit top-right, full-opacity even when dimmed.
    let action: Option<Div> = match wave {
        Wave::Slept => Some(("Wake", t.status.slept, on_wake_action(on_wake))),
        Wave::Failed => Some(("Retry", t.status.failed, on_wake_action(on_retry))),
        _ => None,
    }
    .map(|(label, accent, el)| el.child(label).text_color(overlay_fg).bg(accent.opacity(0.30))
        .border_1().border_color(accent.opacity(0.55)));
```

Rather than the closure gymnastics above, use the concrete helper — add this free fn to `chrome.rs`:

```rust
/// A top-right pill button (Wake / Retry). `on_click` is the wired handler.
fn action_button(
    id: &'static str,
    label: &'static str,
    accent: Hsla,
    fg: Hsla,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .id(id)
        .cursor_pointer()
        .rounded(px(7.0))
        .px_2()
        .py(px(4.0))
        .text_xs()
        .text_color(fg)
        .bg(accent.opacity(0.30))
        .border_1()
        .border_color(accent.opacity(0.55))
        .on_click(on_click)
        .child(label)
}
```

Then replace the `action` binding with:

```rust
    let action: Option<Div> = match wave {
        Wave::Slept => Some(action_button("card-wake", "Wake", t.status.slept, overlay_fg, on_wake)),
        Wave::Failed => Some(action_button("card-retry", "Retry", t.status.failed, overlay_fg, on_retry)),
        _ => None,
    };
```

(d) Add the button to the header, and dim the content when Slept. The header (`:152-179`) currently ends with the kebab child. Add the action as a header child (before the kebab) and apply `.opacity(0.42)` to the dimmable pieces when `dim`. Concretely, wrap the tile + meta column in `.when(dim, |d| d.opacity(0.42))`, and add `action`:

Change the tile child:
```rust
        .child(render_icon_tile(wave, border).when(dim, |t| t.opacity(0.42)))
```
Change the meta column: append `.when(dim, |c| c.opacity(0.42))` to the `div().flex_grow()…` child (after `.child(ellipsize_line(title))` closes at `:171`).
Insert the action button before the kebab child (`:173`):
```rust
        .children(action)
```
(`.children(Option<Div>)` renders it when `Some`.)

(e) Dim the remaining rows when Slept. On the `harness_model` child (`:212`), the activity slot (`:213-228`), the repos row (`:229-239`), and the foot (`:240-250`), append `.when(dim, |d| d.opacity(0.42))` to each. Import the extension: `.when` is `gpui::prelude::*` (already imported at `chrome.rs:3`).

(f) The Failed activity slot no longer needs the retry click (Retry is now the button). Remove the `if wave == Wave::Failed { activity_slot = activity_slot.cursor_pointer().on_click(on_retry); }` block (`:224-226`).

- [ ] **Step 4: Update the caller.** In `crates/lens-ui/src/card/view.rs`, the `render_card_chrome(...)` call passes handlers in order. Insert the `on_wake` listener after `on_kebab_toggle` and repoint the retry listener (currently `SessionCommand::Send{empty}` at `:143-151`) to the seam:

```rust
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = !view.kebab_open;
                            cx.notify();
                        }),
                        // on_wake
                        cx.listener(|view, _, _, cx| {
                            let fleet = view.fleet.clone();
                            let sid = view.session_id.clone();
                            fleet.update(cx, |f, _| f.wake_session(&sid));
                        }),
                        // on_sleep (unchanged)
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = false;
                            view.send_command(SessionCommand::Sleep, cx);
                        }),
                        // on_send (unchanged)
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = false;
                            view.send_command(
                                SessionCommand::Send { text: String::new(), model_override: None },
                                cx,
                            );
                        }),
                        // on_retry → seam
                        cx.listener(|view, _, _, cx| {
                            let fleet = view.fleet.clone();
                            let sid = view.session_id.clone();
                            fleet.update(cx, |f, _| f.retry_session(&sid));
                        }),
```

- [ ] **Step 5: Verify build + on-device**

Run: `cargo run -p lens-app --release -- --demo`
Expected: the Slept card's content is dimmed with a bright **Wake** pill top-right; the Failed card shows the error text in the activity line with a **Retry** pill top-right. Clicking them does nothing (seam) — expected in demo.

- [ ] **Step 6: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/fleet/store.rs crates/lens-ui/src/card/chrome.rs crates/lens-ui/src/card/view.rs
git commit -m "feat(card): B5 Slept dim + Wake button, Failed Retry button (visual + FleetStore seams)"
```

---

## Task 5: Working spinner — clock-phase-driven rotating Lucide `loader-circle`

Working's signature is **rotation**, no sweep (spec §3). Replace the static ring with a `loader-circle` SVG rotated by a clock-phase angle. Deliberately NOT gpui-component's `Spinner` — it self-animates outside our frame-cap/viewport-gate/§4.4 driver. Working must now animate (it currently returns `wave_animates == false`).

**Files:**
- Modify: `crates/lens-ui/src/card/motion.rs` (`spin_period`, `spin_phase`, rewrite `render_working_spinner`, extend `wave_animates`)
- Test: `crates/lens-ui/src/card/motion.rs` tests

**Interfaces:**
- Produces:
  - `motion::spin_phase(now_ms: i64) -> f32` — 0.0..1.0 rotation fraction (period 2.0s).
  - `motion::render_working_spinner(status: Hsla, now_ms: i64) -> impl IntoElement` — signature GAINS `now_ms` (was `(color)`); update the `chrome.rs` call.
  - `wave_animates(Wave::Working) == true`.

- [ ] **Step 1: Write the failing test** — add to `motion.rs` tests:

```rust
    #[test]
    fn spin_phase_advances_and_wraps() {
        assert_eq!(spin_phase(0), 0.0);
        assert!((spin_phase(1000) - 0.5).abs() < 1e-4, "half period");
        assert!(spin_phase(2000).abs() < 1e-4, "wraps at 2s");
    }

    #[test]
    fn spin_phase_survives_epoch_millis() {
        // i64-modulo-before-cast: a real epoch value must not quantize to a frozen phase.
        let a = spin_phase(1_700_000_000_123);
        let b = spin_phase(1_700_000_000_123 + 500);
        assert!((a - b).abs() > 1e-3, "phase must advance at epoch scale");
    }

    #[test]
    fn working_animates_now() {
        assert!(wave_animates(Wave::Working));
        assert!(!wave_animates(Wave::Slept));
        assert!(!wave_animates(Wave::Neutral));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-ui --lib card::motion::tests`
Expected: FAIL — `spin_phase` missing; `wave_animates(Working)` is false.

- [ ] **Step 3: Implement.** In `crates/lens-ui/src/card/motion.rs`:

Add near `sweep_period`:
```rust
const SPIN_PERIOD_MS: i64 = 2000;

/// Spinner rotation fraction (0..1) at `now_ms` — pure fn of the clock (period 2.0s).
/// i64 modulo BEFORE the f32 cast (epoch-millis overflow — see `sweep_phase`).
pub fn spin_phase(now_ms: i64) -> f32 {
    now_ms.rem_euclid(SPIN_PERIOD_MS) as f32 / SPIN_PERIOD_MS as f32
}
```

Extend `wave_animates` (`:28-30`):
```rust
/// True for any wave that runs a per-frame (or per-second) animation.
pub fn wave_animates(wave: Wave) -> bool {
    sweep_period(wave).is_some() || matches!(wave, Wave::Working | Wave::Scheduled)
}
```
(Scheduled is included now — its 1 Hz countdown is driven by the same task; the tick interval is chosen per-wave in Task 8's `anim_tick_for`. Until Task 7/8 land, Scheduled animating is harmless — it just re-renders a static card once/tick.)

Rewrite `render_working_spinner` (`:79-85`):
```rust
use gpui::{Transformation, radians, svg};
use std::f32::consts::TAU;

/// Rotating Lucide `loader-circle`, tinted to the working color, angle from the clock.
/// Rotation is render-only (no layout/hitbox effect) — pivots around the element center.
pub fn render_working_spinner(status: Hsla, now_ms: i64) -> impl IntoElement {
    svg()
        .path("icons/loader-circle.svg")
        .w(px(22.0))
        .h(px(22.0))
        .text_color(status)
        .with_transformation(Transformation::rotate(radians(spin_phase(now_ms) * TAU)))
}
```
(Merge the new imports into the existing `use gpui::{…}` at `motion.rs:3` and add `use std::f32::consts::TAU;` at the top.)

- [ ] **Step 4: Update the tile call.** In `crates/lens-ui/src/card/chrome.rs`, `render_icon_tile` needs `now_ms`. Thread it: change `render_icon_tile(wave: Wave, status: Hsla)` → `render_icon_tile(wave: Wave, status: Hsla, now_ms: i64)`, pass `now_ms` to `render_working_spinner(status, now_ms)`, and at the call site in `render_card_chrome` header (`:157`) pass a `now_ms` param. Add `now_ms: i64` to the `render_card_chrome` signature (after `sweep_phase: Option<f32>` at `:111`) and have `view.rs` pass `self.clock.now_millis()` (already computed as `now_ms` at `view.rs:81`).

- [ ] **Step 5: Run test + on-device**

Run: `cargo test -p lens-ui --lib card::motion::tests` → PASS.
Run: `cargo run -p lens-app --release -- --demo` → the Working tile shows a **rotating** arc-spinner tinted green; all other tiles unchanged.

- [ ] **Step 6: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/motion.rs crates/lens-ui/src/card/chrome.rs crates/lens-ui/src/card/view.rs
git commit -m "feat(card): Working spinner = clock-phase-driven rotating loader-circle (Working now animates)"
```

---

## Task 6: Canvas sweep — `paint_path` skewed parallelogram (replaces the flat div band)

The div 2-stop gradient reads as a flat vertical bar (spike gotcha #2). Replace with a **skewed parallelogram** drawn via `canvas` + `PathBuilder::fill()` + `window.paint_path`, clipped to the card bounds. Mockup params (`wave-states-motion.html:36-40`): band width 48% of card, skew −14°, travel left→right, peak alpha = status × (24% × amplitude 0.4) = **0.096**. gpui `linear_gradient` is 2-stop → keep the two-half-band feather (transparent→peak, peak→transparent).

**Files:**
- Modify: `crates/lens-ui/src/card/motion.rs` (rewrite `render_sweep_overlay`; keep `sweep_phase`/`sweep_period` unchanged)
- Test: `crates/lens-ui/src/card/motion.rs` tests (assert the existing `sweep_phase` math — the visual is on-device)

**Interfaces:**
- Consumes: `sweep_phase` (`:19`), `CARD_WIDTH_PX`/`CARD_HEIGHT_PX` (`model.rs:9-10`).
- Produces: `render_sweep_overlay(status: Hsla, phase: f32) -> impl IntoElement` (signature unchanged — internals become a `canvas`).

- [ ] **Step 1: Write/confirm the failing test** — add to `motion.rs` tests (pins the position math the canvas consumes):

```rust
    #[test]
    fn sweep_phase_is_a_clock_ratio() {
        // 1.0s period for the loud pair; midpoint at 500ms.
        assert!((sweep_phase(Wave::NeedsInput, 500).unwrap() - 0.5).abs() < 1e-4);
        assert_eq!(sweep_phase(Wave::NeedsInput, 0), Some(0.0));
        // 1.5s for the soft pair.
        assert!((sweep_phase(Wave::Ready, 750).unwrap() - 0.5).abs() < 1e-4);
        // non-sweep waves → None.
        assert_eq!(sweep_phase(Wave::Working, 123), None);
        assert_eq!(sweep_phase(Wave::Slept, 123), None);
    }

    #[test]
    fn sweep_phase_survives_epoch_millis() {
        let a = sweep_phase(Wave::NeedsInput, 1_700_000_000_123).unwrap();
        let b = sweep_phase(Wave::NeedsInput, 1_700_000_000_123 + 100).unwrap();
        assert!((a - b).abs() > 1e-3, "phase must advance at epoch scale");
    }
```

- [ ] **Step 2: Run to verify it fails/passes**

Run: `cargo test -p lens-ui --lib card::motion::tests::sweep_phase_is_a_clock_ratio`
Expected: PASS if `sweep_phase` already behaves (it does — this pins it before we refactor rendering). If it FAILS, fix `sweep_phase` first.

- [ ] **Step 3: Rewrite `render_sweep_overlay` as a canvas.** Replace `motion.rs:38-74` (the `SWEEP_ALPHA`/`SWEEP_ANGLE` consts + the div impl):

```rust
// Peak alpha of the moving highlight = status × (24% × amplitude 0.4) — mockup value; soft
// so text stays legible. Tunable in the end-of-build pass.
const SWEEP_PEAK_ALPHA: f32 = 0.096;
// Skew of the band (degrees), matching the mockup's skewX(-14deg).
const SWEEP_SKEW_DEG: f32 = 14.0;
// Band width as a fraction of card width.
const SWEEP_BAND_FRAC: f32 = 0.48;

/// Clipped sweep overlay at `phase` (0..1), drawn as a skewed gradient parallelogram via
/// `canvas` + `paint_path`. No `with_animation` — the card view's timer re-renders us with a
/// fresh clock-derived phase. Two half-bands fake a symmetric feather (gpui gradients are 2-stop).
pub fn render_sweep_overlay(status: Hsla, phase: f32) -> impl IntoElement {
    use gpui::{
        Background, Bounds, ContentMask, PathBuilder, Point, canvas, linear_color_stop,
        linear_gradient, point,
    };

    let peak: Background = status.opacity(SWEEP_PEAK_ALPHA).into();
    let edge: Background = status.opacity(0.0).into();

    canvas(
        move |_, _, _| (),
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let h = bounds.size.height;
                let card_w = bounds.size.width;
                let band_w = card_w * SWEEP_BAND_FRAC;
                let skew = h * SWEEP_SKEW_DEG.to_radians().tan();

                // Center-x of the band as it travels from off-left to off-right.
                let travel = card_w + band_w;
                let cx = -band_w * 0.5 + phase * travel;
                let x = |dx: f32| bounds.origin.x + gpui::px(dx);
                let top = bounds.origin.y;
                let bot = bounds.origin.y + h;

                // A parallelogram spanning [left..right] at the bottom, sheared right by `skew`
                // at the top. `half` builds one gradient half.
                let mut half = |x0: f32, x1: f32, from: Background, to: Background| {
                    let mut b = PathBuilder::fill();
                    b.move_to(point(x(x0), bot));
                    b.line_to(point(x(x0 + skew.0), top));
                    b.line_to(point(x(x1 + skew.0), top));
                    b.line_to(point(x(x1), bot));
                    b.close();
                    if let Ok(path) = b.build() {
                        // 90° = left→right horizontal feather.
                        window.paint_path(path, linear_gradient(90.0, linear_color_stop_bg(from, 0.0), linear_color_stop_bg(to, 1.0)));
                    }
                    let _ = (linear_color_stop, linear_color_stop_bg); // keep imports honest
                };

                let left = cx - band_w * 0.5;
                let mid = cx;
                let right = cx + band_w * 0.5;
                half(left, mid, edge, peak);
                half(mid, right, peak, edge);
                let _ = (Point::default(), skew, band_w);
            });
        },
    )
    .absolute()
    .size_full()
}
```

**Note for the implementer:** `linear_gradient(angle, from, to)` takes `impl Into<LinearColorStop>`. Build stops from a `Background`'s underlying color. Simplify by passing `Hsla` stops directly instead of `Background`:

```rust
    let peak = status.opacity(SWEEP_PEAK_ALPHA);
    let edge = status.opacity(0.0);
    // …
    window.paint_path(
        path,
        linear_gradient(90.0, linear_color_stop(from, 0.0), linear_color_stop(to, 1.0)),
    );
```
where `from`/`to` are `Hsla` (`peak`/`edge`). Drop the `linear_color_stop_bg` helper — it does not exist; `linear_color_stop(color: impl Into<Hsla>, pct: f32)` is the real API (`gpui color.rs:793`). The `half` closure then takes `Hsla` args. Delete the `let _ = …` keep-honest lines.

Final clean `half` closure:
```rust
                let mut half = |x0: f32, x1: f32, from: Hsla, to: Hsla| {
                    let mut b = PathBuilder::fill();
                    b.move_to(point(x(x0), bot));
                    b.line_to(point(x(x0 + skew.0), top));
                    b.line_to(point(x(x1 + skew.0), top));
                    b.line_to(point(x(x1), bot));
                    b.close();
                    if let Ok(path) = b.build() {
                        window.paint_path(
                            path,
                            linear_gradient(90.0, linear_color_stop(from, 0.0), linear_color_stop(to, 1.0)),
                        );
                    }
                };
```
Imports for the closure: `use gpui::{Bounds, ContentMask, Hsla, PathBuilder, canvas, linear_color_stop, linear_gradient, point, px};` (fold into the file's existing gpui `use`).

- [ ] **Step 4: Run tests + on-device**

Run: `cargo test -p lens-ui --lib card::motion::tests` → PASS.
Run: `cargo run -p lens-app --release -- --demo` → the NeedsInput/Failed/AwaitingReview/Ready cards show a **soft diagonal light band** sweeping left→right (skewed, feathered), not a flat vertical bar. If the band is clipped wrong or upside-down, adjust `skew` sign / `top`/`bot` (tuning, per spec §8a #2).

- [ ] **Step 5: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/motion.rs
git commit -m "feat(card): canvas paint_path skewed-parallelogram sweep (replaces flat div band)"
```

---

## Task 7: Scheduled countdown ring + live "wakes in Xm Ys" text

Scheduled draws a **depleting arc** around the tile (real time-to-wake) plus a live countdown line, redrawn at **1 Hz** (Task 8 wires the cadence; this task builds the visuals + math). Arc via `canvas`/`arc_to` (no conic-gradient in gpui). Fraction needs a start timestamp — add `scheduled_started_at` to the card.

**Files:**
- Modify: `crates/lens-ui/src/card/model.rs` (add `scheduled_started_at`, init, demo already sets `scheduled_wake_at`)
- Modify: `crates/lens-app/src/main.rs` (demo card sets `scheduled_started_at`)
- Modify: `crates/lens-ui/src/card/motion.rs` (`countdown_fraction`, `format_wake_countdown`, `render_countdown_ring`)
- Modify: `crates/lens-ui/src/card/chrome.rs` (host the ring in the tile for Scheduled; Scheduled activity line = the live countdown)
- Test: `crates/lens-ui/src/card/motion.rs` tests

**Interfaces:**
- Consumes: `SessionCard.scheduled_wake_at` (`model.rs:61`), new `scheduled_started_at`.
- Produces:
  - `SessionCard.scheduled_started_at: Option<i64>`.
  - `motion::countdown_fraction(started_at: Option<i64>, wake_at: Option<i64>, now_ms: i64) -> Option<f32>` — remaining fraction 0..1, `None` if either bound missing.
  - `motion::format_wake_countdown(remaining_ms: i64) -> String` — "wakes in Xm Ys" / "wakes in Xs" / "waking…".
  - `motion::render_countdown_ring(status: Hsla, fraction: f32) -> impl IntoElement` — a 44px canvas arc.

- [ ] **Step 1: Write the failing test** — add to `motion.rs` tests:

```rust
    #[test]
    fn countdown_fraction_depletes() {
        let start = 10_000;
        let wake = 10_000 + 180_000; // 3m window
        assert_eq!(countdown_fraction(Some(start), Some(wake), start), Some(1.0));
        assert_eq!(countdown_fraction(Some(start), Some(wake), wake), Some(0.0));
        let mid = countdown_fraction(Some(start), Some(wake), start + 90_000).unwrap();
        assert!((mid - 0.5).abs() < 1e-3);
        // past wake clamps to 0; missing bound → None.
        assert_eq!(countdown_fraction(Some(start), Some(wake), wake + 5_000), Some(0.0));
        assert_eq!(countdown_fraction(None, Some(wake), start), None);
    }

    #[test]
    fn format_wake_countdown_shapes() {
        assert_eq!(format_wake_countdown(179_000), "wakes in 2m 59s");
        assert_eq!(format_wake_countdown(45_000), "wakes in 45s");
        assert_eq!(format_wake_countdown(0), "waking…");
        assert_eq!(format_wake_countdown(-1), "waking…");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-ui --lib card::motion::tests`
Expected: FAIL — functions missing.

- [ ] **Step 3: Add the model field.** In `crates/lens-ui/src/card/model.rs`, add after `scheduled_wake_at` (`:61`):

```rust
    /// §2 waves: epoch-ms when the current schedule began — the countdown-ring denominator
    /// (`(wake − now) / (wake − started)`). Set alongside `scheduled_wake_at`.
    pub scheduled_started_at: Option<i64>,
```
Init `scheduled_started_at: None,` in `SessionCard::new` (`:106` area).

In `crates/lens-app/src/main.rs`, the `scheduled` demo card (`:216-219`) sets `scheduled_wake_at`; add:
```rust
    scheduled.scheduled_started_at = Some(now);
```

- [ ] **Step 4: Implement the math + ring.** In `crates/lens-ui/src/card/motion.rs`:

```rust
/// Remaining fraction (0..1) of a scheduled wake window; `None` if either bound is missing.
pub fn countdown_fraction(started_at: Option<i64>, wake_at: Option<i64>, now_ms: i64) -> Option<f32> {
    let (start, wake) = (started_at?, wake_at?);
    let span = wake - start;
    if span <= 0 {
        return Some(0.0);
    }
    let remaining = (wake - now_ms).max(0) as f32 / span as f32;
    Some(remaining.clamp(0.0, 1.0))
}

/// Live countdown label. `remaining_ms <= 0` → "waking…".
pub fn format_wake_countdown(remaining_ms: i64) -> String {
    if remaining_ms <= 0 {
        return "waking…".into();
    }
    let secs = (remaining_ms + 999) / 1000; // ceil to whole seconds
    if secs >= 60 {
        format!("wakes in {}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("wakes in {secs}s")
    }
}

/// Depleting arc around the 44px tile, drawn full→empty as `fraction` goes 1→0.
/// `canvas` + `arc_to` stroke (gpui has no conic-gradient). NOT in the 30fps loop — 1 Hz.
pub fn render_countdown_ring(status: Hsla, fraction: f32) -> impl IntoElement {
    use gpui::{Bounds, ContentMask, PathBuilder, canvas, point, px};
    let frac = fraction.clamp(0.0, 1.0);
    canvas(
        move |_, _, _| (),
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            if frac <= 0.0 {
                return;
            }
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let stroke = px(2.0);
                let center = bounds.center();
                let radius = (bounds.size.width.min(bounds.size.height) - stroke) / 2.0;
                let sweep_deg = frac * 360.0;
                // 0° = top, clockwise.
                let polar = |deg: f32| {
                    let r = deg.to_radians();
                    point(center.x + radius * r.sin(), center.y - radius * r.cos())
                };
                let mut b = PathBuilder::stroke(stroke);
                b.move_to(polar(0.0));
                if sweep_deg >= 359.9 {
                    // full ring = two half-arcs (single arc_to can't close 360°).
                    b.arc_to(point(radius, radius), px(0.0), true, true, polar(180.0));
                    b.arc_to(point(radius, radius), px(0.0), true, true, polar(0.0));
                } else {
                    b.arc_to(point(radius, radius), px(0.0), sweep_deg > 180.0, true, polar(sweep_deg));
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, status);
                }
            });
        },
    )
    .absolute()
    .inset(px(-4.0))
}
```

- [ ] **Step 5: Host the ring + live text in chrome.** In `crates/lens-ui/src/card/chrome.rs`:

(a) The Scheduled tile hosts the ring as a sibling of the glyph. In `render_icon_tile`, when `wave == Wave::Scheduled`, add the ring behind the glyph. Extend the signature to accept a countdown fraction: `render_icon_tile(wave, status, now_ms, countdown: Option<f32>)`, and when `Some(frac)`, add `.child(render_countdown_ring(status, frac))` to the tile (it uses `.absolute().inset(-4px)` so it draws around the tile). Import `render_countdown_ring`.

(b) Compute the fraction + live text in `render_card_chrome`:
```rust
    let countdown = motion::countdown_fraction(card.scheduled_started_at, card.scheduled_wake_at, now_ms);
    // Scheduled activity line = the live countdown (overrides activity_summary).
    let activity = if wave == Wave::Scheduled {
        card.scheduled_wake_at
            .map(|w| motion::format_wake_countdown(w - now_ms))
            .unwrap_or_else(|| activity.clone())
    } else {
        activity
    };
```
Pass `countdown` into `render_icon_tile`. Add `use super::motion;` or extend the existing `use super::motion::{…}` import to include `countdown_fraction`, `format_wake_countdown`, `render_countdown_ring`.

- [ ] **Step 6: Run tests + on-device**

Run: `cargo test -p lens-ui --lib card::motion::tests` → PASS.
Run: `cargo run -p lens-app --release -- --demo` → the Scheduled card shows a **partial arc** around the ⏰ tile and an activity line "wakes in ~2m …". (The per-second ticking arrives with Task 8's 1 Hz driver; without it the value is static until another notify.)

- [ ] **Step 7: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/model.rs crates/lens-app/src/main.rs crates/lens-ui/src/card/motion.rs crates/lens-ui/src/card/chrome.rs
git commit -m "feat(card): Scheduled countdown ring (canvas arc) + live 'wakes in Xm Ys' text"
```

---

## Task 8: Viewport-gate the driver + per-wave tick cadence

Two things: (1) generalize the driver's tick to `anim_tick_for(wave)` (30fps for sweep/spin, **1 Hz** for Scheduled), and (2) **viewport-gate**: gpui does NOT auto-cull off-screen `Div` children (verified — only `List`/`UniformList` virtualize), so an off-screen card's timer keeps re-rendering it. Gate the driver on `card_bounds.intersects(window viewport)`.

**Files:**
- Modify: `crates/lens-ui/src/card/motion.rs` (add `anim_tick_for`; keep `wave_animates` as the predicate)
- Modify: `crates/lens-ui/src/card/view.rs` (driver uses `anim_tick_for` + viewport gate)
- Test: `crates/lens-ui/src/card/motion.rs` tests

**Interfaces:**
- Consumes: `view.rs` `last_bounds: Rc<Cell<Option<Bounds<Pixels>>>>` (`view.rs:44`, updated in the canvas paint closure `:157-159`); `window.viewport_size()` → `Size<Pixels>`; `Bounds::intersects`.
- Produces: `motion::anim_tick_for(wave: Wave) -> Option<std::time::Duration>` — `33ms` (or `LENS_ANIM_MS` under `demo`, Task 10) for sweep/spin waves, `1000ms` for Scheduled, `None` otherwise.

- [ ] **Step 1: Write the failing test** — add to `motion.rs` tests:

```rust
    #[test]
    fn anim_tick_cadence_per_wave() {
        use std::time::Duration;
        assert_eq!(anim_tick_for(Wave::NeedsInput), Some(Duration::from_millis(33)));
        assert_eq!(anim_tick_for(Wave::Working), Some(Duration::from_millis(33)));
        assert_eq!(anim_tick_for(Wave::Scheduled), Some(Duration::from_millis(1000)));
        assert_eq!(anim_tick_for(Wave::Slept), None);
        assert_eq!(anim_tick_for(Wave::Neutral), None);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p lens-ui --lib card::motion::tests::anim_tick_cadence_per_wave`
Expected: FAIL — `anim_tick_for` missing.

- [ ] **Step 3: Implement `anim_tick_for`.** In `motion.rs`:

```rust
use std::time::Duration;

/// Frame cap per wave: 30fps for the sweep/spinner class, 1 Hz for the Scheduled
/// countdown, `None` for still waves. (`demo` feature can override the 30fps value — Task 10.)
pub fn anim_tick_for(wave: Wave) -> Option<Duration> {
    if matches!(wave, Wave::Scheduled) {
        return Some(Duration::from_millis(1000));
    }
    if wave_animates(wave) {
        return Some(Duration::from_millis(anim_tick_ms_fast()));
    }
    None
}

/// The fast-class frame cap in ms. Overridable ONLY under the `demo` feature (Task 10);
/// the shipped build is a hard 33ms (≈30fps).
fn anim_tick_ms_fast() -> u64 {
    33
}
```
(`wave_animates` already returns true for Scheduled from Task 5; `anim_tick_for` picks the cadence. `anim_tick_ms_fast` replaces `view.rs::anim_tick_ms` — Task 10 adds the demo-gated env override here.)

- [ ] **Step 4: Rewrite the driver + add the viewport gate.** In `crates/lens-ui/src/card/view.rs`, replace the driver block (`:86-102`) with a version that (a) picks the tick from `anim_tick_for`, and (b) only runs when the card's last painted bounds intersect the window viewport:

```rust
        // Viewport gate: gpui does not auto-cull off-screen Div children, so an off-screen
        // card's timer would keep re-rendering it. Only animate while visible. First frame
        // (no bounds yet) counts as visible; it self-corrects next frame.
        let visible = match self.last_bounds.get() {
            Some(b) => {
                let viewport = gpui::Bounds::new(gpui::Point::default(), _window.viewport_size());
                b.intersects(&viewport)
            }
            None => true,
        };
        let tick = motion::anim_tick_for(wave).filter(|_| visible);
        match (tick, self.anim_task.is_some()) {
            (Some(interval), false) => {
                self.anim_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(interval).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            (None, true) => self.anim_task = None,
            _ => {}
        }
```
Rename the `_window` param to `window` in the `render` signature (`view.rs:78`) since it's now used. Update the `use super::motion::{…}` import to include `anim_tick_for` and drop the now-unused `wave_animates` import if present; delete the old `anim_tick_ms` fn (`view.rs:24-30`).

**Note:** re-entry after scrolling back on-screen is a B6 concern (no scroll container in this build). Here the gate bounds cards that overflow the non-scrolling window. When `tick` changes cadence (e.g. Working→Scheduled on a state change) the `(Some,true)` arm keeps the *old* interval until the task naturally ends; acceptable because a wave transition drops through `derive_wave` → the old task is replaced only on `(None,true)`. If a cadence change mid-wave matters later, cancel-and-respawn on interval change; not needed now (each wave has one cadence).

- [ ] **Step 5: Run tests + on-device**

Run: `cargo test -p lens-ui --lib` → PASS (all card tests).
Run: `LENS_DEMO_N=4 cargo run -p lens-app --release -- --demo` → cards past the window edge do not animate (timer gated); visible cards animate. The Scheduled card's countdown text now ticks once per second.

- [ ] **Step 6: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/motion.rs crates/lens-ui/src/card/view.rs
git commit -m "feat(card): viewport-gate the anim driver + per-wave tick cadence (30fps / 1Hz)"
```

---

## Task 9: §4.4 isolation acceptance test — animating neighbor must not bump a static card

Extend `session_card_view_observes_own_card_only` (`view.rs` tests) so it proves the animation driver stays self-only: a card whose wave animates must not increment a *static* sibling's `paint_count`/`render_count`.

**Files:**
- Modify: `crates/lens-ui/src/card/view.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `spawn_fake_session` (`store.rs:118`), `SessionCardView.render_count`/`paint_count` (`view.rs:42-43`), `ManualUiClock`.

- [ ] **Step 1: Read the existing test** to learn the harness (it's async `#[gpui::test]`, mounts two cached cards via `DualCardBoard`, drives frames). Note: under `TestAppContext` paint is a `NoopTextSystem` (memory `gpui-test-noop-text-system`) — assert on `render_count` deltas (render runs even when paint is faked), not pixels.

- [ ] **Step 2: Write the failing test** — add a sibling test in the same module:

```rust
    #[gpui::test]
    async fn animating_card_does_not_render_a_static_sibling(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(crate::clock::ManualUiClock::new(0));
        let sid_a = SessionId::new("anim");
        let sid_b = SessionId::new("static");

        let (fleet, view_a, view_b, rc_b) = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock.clone(), cx);
            let card_a = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_a.clone(), cx));
            let card_b = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_b.clone(), cx));
            // Card A animates (Working); B is Neutral/still.
            card_a.update(cx, |c, _| c.status = lens_core::domain::scalars::SessionStatusValue::Running);
            let ui_clock = fleet.read(cx).clock();
            let view_a = cx.new(|cx| SessionCardView::new(card_a.clone(), ui_clock.clone(), fleet.clone(), sid_a.clone(), cx));
            let view_b = cx.new(|cx| SessionCardView::new(card_b.clone(), ui_clock, fleet.clone(), sid_b.clone(), cx));
            let rc_b = view_b.read(cx).render_count.clone();
            (fleet, view_a, view_b, rc_b)
        });

        let (_board, _vcx) = cx.add_window_view(|_, _| DualCardBoard {
            view_a: view_a.clone(),
            view_b: view_b.clone(),
        });

        // Let A's driver tick several times.
        let baseline_b = rc_b.get();
        for _ in 0..5 {
            cx.background_executor().timer(std::time::Duration::from_millis(40)).await;
            cx.run_until_parked();
        }
        assert!(view_a.read_with(cx, |v, _| v.render_count.get()) > baseline_b + 2,
            "animating card A must have re-rendered");
        assert_eq!(rc_b.get(), baseline_b,
            "static sibling B must NOT re-render when A animates (§4.4)");
        let _ = fleet;
    }
```
(Adapt the exact constructor/argument shapes to the existing test's — mirror `session_card_view_observes_own_card_only` for `spawn_fake_session`, `clock()`, and `add_window_view` usage.)

- [ ] **Step 3: Run to verify it fails or passes**

Run: `cargo test -p lens-ui --lib card::view::tests::animating_card_does_not_render_a_static_sibling`
Expected: PASS if the driver is correctly self-only (it should be — `this.update(cx, |_, cx| cx.notify())` touches only the card entity). If B's count moves, the driver is leaking a notify to a shared entity — fix the driver, not the test.

- [ ] **Step 4: Gate + commit**

Run: `cargo xtask gate`

```bash
git add crates/lens-ui/src/card/view.rs
git commit -m "test(card): §4.4 isolation — animating neighbor must not re-render a static card"
```

---

## Task 10: `demo` cargo feature-gate + kill the `LENS_ANIM_DBG` eprintln

Compile the demo path + dev env-knobs OUT of the shipped `lens-app`, keep them in a `--features demo` "dev-release" build (the vehicle for the on-device tuning + perf pass). Uses a **cargo feature**, NOT `debug_assertions` (the perf pass needs release opt + the knobs together).

**Files:**
- Modify: `crates/lens-app/Cargo.toml` (add `[features] demo = ["lens-ui/demo"]`)
- Modify: `crates/lens-ui/Cargo.toml` (add `[features] demo = []`)
- Modify: `crates/lens-app/src/main.rs` (gate `--demo`, `run_demo`, `demo_cards`, `LENS_DEMO_N`, `spawn_demo_paint_instrumentation` behind `#[cfg(feature = "demo")]`)
- Modify: `crates/lens-ui/src/card/view.rs` (remove the `LENS_ANIM_DBG` eprintln; move the `LENS_ANIM_MS` override into `anim_tick_ms_fast` under `#[cfg(feature = "demo")]`)
- Modify: `crates/lens-ui/src/card/mod.rs` / `lib.rs` if `spawn_demo_paint_instrumentation` needs feature-gating in its export

- [ ] **Step 1: Add the features.** `crates/lens-ui/Cargo.toml`:
```toml
[features]
demo = []
```
`crates/lens-app/Cargo.toml`:
```toml
[features]
demo = ["lens-ui/demo"]
```

- [ ] **Step 2: Kill the eprintln + gate the fast-tick override.** In `crates/lens-ui/src/card/view.rs`, delete the `LENS_ANIM_DBG` block (`:106-108`). In `motion.rs`, make `anim_tick_ms_fast` read the env override only under the feature:
```rust
fn anim_tick_ms_fast() -> u64 {
    #[cfg(feature = "demo")]
    {
        if let Some(ms) = std::env::var("LENS_ANIM_MS").ok().and_then(|s| s.parse().ok()).filter(|&n| n >= 1) {
            return ms;
        }
    }
    33
}
```

- [ ] **Step 3: Gate the demo path in lens-app.** In `crates/lens-app/src/main.rs`:
  - Prefix `fn run_demo`, `fn demo_cards`, `fn demo_preset_cards` with `#[cfg(feature = "demo")]`.
  - Gate the `--demo` arg parse (`:279-282`) and the `if config.demo { run_demo(); return; }` (`:59-62`) and the `demo` field on `Config` under `#[cfg(feature = "demo")]` — or simpler: keep the `demo` bool but make it always-false without the feature by gating only the arg-parse arm and the dispatch. Cleanest: gate the dispatch:
    ```rust
    #[cfg(feature = "demo")]
    if config.demo {
        run_demo();
        return;
    }
    ```
    and the arg arm:
    ```rust
    #[cfg(feature = "demo")]
    "--demo" => { demo = true; i += 1; }
    ```
    Keep `demo` initialized `let mut demo = false;` and add `let _ = demo;` under `#[cfg(not(feature = "demo"))]` to avoid the unused warning, or gate the field. Use whichever keeps clippy clean.
  - `spawn_demo_paint_instrumentation` is only called from `run_demo`, so it's naturally excluded; gate its *export* in `crates/lens-ui/src/card/mod.rs` (`:11`) and its definition (`view.rs:180-201`) under `#[cfg(feature = "demo")]` so the non-demo build doesn't carry it.

- [ ] **Step 4: Verify both build configurations**

Run: `cargo build -p lens-app` (no feature — production) → builds with NO demo code, NO env reads, NO eprintln in the render path.
Run: `cargo build -p lens-app --features demo` → builds with the demo path.
Run: `cargo clippy -p lens-app -p lens-ui -- -D warnings` and `cargo clippy -p lens-app -p lens-ui --features demo -- -D warnings` → both clean (no unused-var/dead-code).

- [ ] **Step 5: Confirm the demo still runs under the feature**

Run: `cargo run -p lens-app --release --features demo -- --demo` → the 8-card demo works.
Run: `cargo run -p lens-app --release -- --demo` → prints an "unknown flag: --demo" error (demo compiled out) — expected.

- [ ] **Step 6: Gate + commit**

Run: `cargo xtask gate` (production config). Confirm the gate's clippy also covers `--features demo` — if `xtask gate` doesn't build the feature, run the two clippy lines from Step 4 manually and note it.

```bash
git add crates/lens-app/Cargo.toml crates/lens-ui/Cargo.toml crates/lens-app/src/main.rs crates/lens-ui/src/card/view.rs crates/lens-ui/src/card/motion.rs crates/lens-ui/src/card/mod.rs
git commit -m "chore(app): demo cargo feature-gate (dev-release only) + drop LENS_ANIM_DBG eprintln"
```

---

## Task 11: Perf / CPU / GPU / energy completion gate

REQUIRED end-of-build gate (spec §9). Re-run the spike's exact rig and hold against its budget. This is a measurement + write-up task, not a code change (unless a regression forces a fix).

**Files:**
- Modify: `docs/spikes/2026-07-17-wave-animation.md` (append a "Post-build re-measure" section) OR create `docs/spikes/2026-07-17-wave-build-perf.md`

- [ ] **Step 1: Build the measurement target**

Run: `cargo run -p lens-app --release --features demo -- --demo` (leave it running).

- [ ] **Step 2: Measure CPU + energy**

Run (replace `<pid>`): `top -l 5 -s 1 -pid <pid> -stats pid,cpu,power` (find pid via `pgrep -f 'lens-app'`).
Record: steady-state CPU% and energy-impact with the 8-card demo (≈5–6 animating), and again with `LENS_DEMO_N=2` (16 cards; note how many are visible/gated).

- [ ] **Step 3: Measure FPS**

Read the `paint-instr` stderr lines (from `spawn_demo_paint_instrumentation`, available under `--features demo`): paint-count delta / elapsed ≈ FPS. Confirm the fast class caps at ~30fps and the Scheduled card ticks ~1/s.

- [ ] **Step 4: Compare against the budget (regression gate)**

Budget (from the spike): idle floor ~0.3%; **~1.7% CPU per visible animating card @30fps**; ~8.8% for 5. Confirm:
  - Static cards stay ~free (viewport-gated + still waves).
  - Per-visible-animating-card CPU is within ~10% of 1.7%.
  - The canvas sweep + canvas countdown arc + rotating spinner did NOT blow past the div-era cost. If any state is materially heavier, that is a **finding** — profile it (likely the sweep `paint_path` tessellation or arc redraw) and reduce (e.g. lower the sweep tessellation, confirm the countdown is truly 1 Hz not 30fps).

- [ ] **Step 5: Write it up + commit**

Record the numbers, the pass/fail vs budget, and any tuning applied. Note that full-scale (>8 cards, scrolling) validation rides with B6 — this build validates the visible set.

```bash
git add docs/spikes/2026-07-17-wave-build-perf.md
git commit -m "docs(perf): wave-build CPU/energy re-measure vs spike budget"
```

---

## Self-Review

**Spec coverage** (design doc `2026-07-17-wave-behaviors-design.md`):
- §3 motion sheet — NeedsInput/Failed sweep+ring (Task 6 sweep; expanding ring already shipped `motion.rs:88`), Working spinner (Task 5), AwaitingReview/Ready sweep (Task 6), Scheduled countdown ring (Task 7), Slept dim (Task 4), Neutral still (no motion — default). ✓
- §5 glyphs = Lucide SVGs (Tasks 1–2). ✓
- §6 shell: B1 tile (Task 2), B2 pbar (Task 3), B4 tile-left layout (already shipped `chrome.rs:151`), B5 Slept-dim/Wake + Failed-Retry (Task 4). ✓
- §8 driver: timer self-notify + 30fps + i64-phase (Tasks 5–8), viewport-gate (Task 8), canvas sweep (Task 6), countdown ring not in the 30fps loop (Task 8 → 1 Hz). ✓
- §9 testing: phase-from-`ManualUiClock` pure fns (Tasks 5–8 unit tests), §4.4 isolation extension (Task 9), perf gate (Task 11). ✓
- Demo feature-gate + eprintln kill (Task 10 — settled with the user). ✓

**Known coverage notes (not gaps):**
- Expanding ring (NeedsInput/Failed) already exists as a div (`motion.rs:88`) and is kept; its 70%-hold-vs-linear timing tuning folds into the end-of-build color/light pass, not a task.
- The separate "wake-at-`scheduled_wake_at` repaint" (spec §8) is **subsumed** by the 1 Hz Scheduled tick — `derive_wave` self-clears within ≤1s of the wake, flipping the card out of Scheduled. A dedicated one-shot timer is redundant; noted in Task 8.
- B5 Wake/Retry are wired to real `FleetStore` seams that currently no-op (user decision: "visual + wired seam") — behavior lands with the state-model wake=respawn plumbing.

**Type consistency:** `wave_icon_path` (Task 2), `spin_phase`/`render_working_spinner(status, now_ms)` (Task 5), `render_sweep_overlay(status, phase)` (Task 6), `countdown_fraction`/`format_wake_countdown`/`render_countdown_ring` (Task 7), `anim_tick_for` (Task 8) — names are used consistently across the tasks that consume them. `render_card_chrome` gains params `now_ms: i64` (Task 5) and `on_wake` (Task 4) and a `countdown` thread (Task 7); the view.rs call site is updated in each.

**Risks flagged for the implementer:**
1. gpui `svg().text_color()` tinting of stroke-based Lucide SVGs — verified as the standard gpui mask-tint behavior, but confirm on-device in Task 2 Step 6 (first glyph render). If it doesn't tint, the SVGs may need `fill="currentColor"` instead of `stroke`.
2. Canvas sweep geometry (skew sign, clip) is tuning-sensitive (spec §8a #2) — the first cut compiles and animates; refine visually via the reload loop.
3. `xtask gate` may not build `--features demo` — Task 10 Step 6 runs the feature clippy manually; consider adding the demo feature to the gate's clippy list (memory `xtask-gate-scope`) as a follow-up.
