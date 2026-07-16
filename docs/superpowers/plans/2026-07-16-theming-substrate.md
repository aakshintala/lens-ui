# §18 Theming Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the load-bearing semantic token surface for `lens-ui` — a `LensTheme` global (base + status tokens, two themes dark+light), a `cx.lens_theme()` accessor, a bridge that pushes our base palette into gpui-component via its public `Theme::apply_config`, external-file loading, and a manual reload command — then migrate the card `chrome.rs` hexes onto tokens (A2).

**Architecture:** New leaf module `crates/lens-ui/src/theme/`. `LensTheme` is a plain gpui `Global` holding decoded `Hsla` values (parse once). Our custom surfaces read `base.*`/`status.*` as `Hsla` via `cx.lens_theme()`; gpui-component's own widgets get our palette because we build a `ThemeConfig` from our base tokens and call the public `Theme::apply_config` (which also *stores* the config as the mode's `dark_theme`/`light_theme`, so a later `Theme::change` re-applies ours, not gpui-component's default). Disk reads are split from applying so the gpui foreground thread never blocks. `theme` does not depend on `card::Wave`; the `Wave → status color` map lives in `card/wave.rs`.

**Tech Stack:** Rust, gpui 0.2.2, gpui-component 0.5.1 (`Theme`, `ThemeConfig`, `ThemeConfigColors`, `ThemeMode`, `Colorize`), serde + serde_json + anyhow.

**Source spec:** `docs/superpowers/specs/2026-07-16-theming-substrate-design.md` (cross-family reviewed). This plan folds three corrections discovered by verifying the spec against the real crate + repo:

1. **Deps are NOT already present.** Spec §8 claims "serde, serde_json, anyhow are already in lens-ui" — they are not (`crates/lens-ui/Cargo.toml` has only async-channel, crossbeam-channel, futures, gpui, gpui-component, lens-client, lens-core, smallvec). Task 1 adds them.
2. **Reload needs a non-fallback loader.** Spec §3.4's single `load()` falls back to the *embedded* default when an external file is bad (correct for startup). But §3.4's reload prose requires "keep the current active theme — do not reset to embedded" on a bad edit. Those conflict in one function. Resolution: two named loaders — `load(mode, dir) -> Result<LensTheme>` reads the external file *only* and returns `Err` on failure (no fallback; the reload handler calls it → bad edit → `Err` → keep current); `load_or_embedded(mode, dir) -> Result<LensTheme>` wraps `load` with the embedded fallback (startup). See Task 3 / Task 5.
3. **`apply_config` visibility (already verified, no action):** the brainstorm handoff recorded `apply_config` as `pub(crate)`; that is the `ThemeColor`-level one (schema.rs:415). The one the spec uses is `pub fn Theme::apply_config(&mut self, &Rc<ThemeConfig>)` (schema.rs:645) — genuinely public. The spec is correct.

## Global Constraints

- **No-blocking-I/O on the gpui foreground thread** (AGENTS.md:21 / `.agents/rust-ui.md:8`). Disk reads happen off the fg thread. Startup: synchronous read is permitted because it runs *before the window opens* (render loop not yet pumping frames) — spec §3.4. Reload: window is live → read on `cx.background_executor()`, apply on fg.
- **Keep startup light (enforced guarantee).** With no `LENS_THEME_DIR` set (the shipped/production default), startup does **zero disk I/O** — it parses the `include_str!`-embedded JSON (~1 KB) in memory and reads two env vars (`LENS_THEME`, `LENS_THEME_DIR`). Disk is touched only when `LENS_THEME_DIR` points at an external file (dev tuning). Do not add filesystem probing, directory scans, or a config-dir search to the startup path — the embedded default must be reachable with no `stat`/`read` when the override is unset.
- **No `panic!`/`expect`/`unwrap` in the theme module** (no-process-panic). Startup `load` `Err` (embedded default itself unparseable — a build bug) → `eprintln!` + `process::exit(1)`, matching the existing `main.rs` pattern.
- **Views read tokens in `render`, never cache an `Hsla` in entity state** — else a reload won't repaint them (spec §3.2, grok review).
- **`theme` is a leaf** — it must not depend on `card::Wave` or anything in `card/`.
- **No new external crates beyond serde/serde_json/anyhow** — everything else (gpui-component's `Colorize`/`Theme`/`ThemeConfig`/`ThemeConfigColors`/`ThemeMode`) is already a dep; loading uses `std::fs`/`std::env`.
- **gpui-component field facts (verified against 0.5.1 source):** `ThemeConfigColors` fields are `Option<SharedString>` (hex strings). Every field used in the adapter exists: `background, foreground, border, muted, muted_foreground, popover, popover_foreground, primary, primary_foreground, secondary, accent, input, caret, ring, selection, scrollbar, scrollbar_thumb, list, list_active, list_hover, progress_bar, sidebar, sidebar_foreground, sidebar_border, title_bar, title_bar_border, tab, tab_active, tab_active_foreground, tab_foreground, success, warning, danger, info, overlay`. `Theme` derefs to `ThemeColor` (runtime fields are `Hsla`). `Colorize::to_hex(&self) -> String` and `Colorize::parse_hex(&str) -> anyhow::Result<Self>`. `ThemeMode` is `#[serde(rename_all = "snake_case")]` → `"dark"`/`"light"`; has `is_dark()`.

---

## File Structure

- `crates/lens-ui/Cargo.toml` — **modify**: add `serde`, `serde_json`, `anyhow` deps.
- `crates/lens-ui/src/lib.rs` — **modify**: `pub mod theme;`, re-export `ActiveLensTheme`/`LensTheme`.
- `crates/lens-ui/src/theme/mod.rs` — **create**: `LensTheme`, `ActiveLensTheme`, `parse_theme`, `load`/`load_or_embedded`, `select_mode`, `to_theme_config`, `apply`, `install_at_startup`, `reload`. The module's tests.
- `crates/lens-ui/src/theme/tokens.rs` — **create**: `BaseTokens`, `StatusTokens`, `hex_hsla` serde module.
- `crates/lens-ui/src/theme/lens-dark.json` — **create**: "Lens Dark" (base + status).
- `crates/lens-ui/src/theme/lens-light.json` — **create**: "Lens Light" (base + status).
- `crates/lens-ui/src/actions.rs` — **modify**: add `ReloadTheme` action.
- `crates/lens-ui/src/board/mod.rs` — **modify**: `on_reload_theme` handler + `.on_action` wiring.
- `crates/lens-ui/src/card/wave.rs` — **modify**: `impl Wave { fn status_color }`.
- `crates/lens-ui/src/card/chrome.rs` — **modify**: hex → token migration (A2).
- `crates/lens-app/src/main.rs` — **modify**: call `theme::install_at_startup(cx)` after both `gpui_component::init(cx)` sites; bind `ReloadTheme` key.

---

### Task 1: Deps + token structs + hex serde

**Files:**
- Modify: `crates/lens-ui/Cargo.toml`
- Modify: `crates/lens-ui/src/lib.rs`
- Create: `crates/lens-ui/src/theme/mod.rs`
- Create: `crates/lens-ui/src/theme/tokens.rs`

**Interfaces:**
- Produces: `mod hex_hsla` (serde with-module, hex↔`Hsla`); `pub struct BaseTokens` (all `Hsla` fields per spec §3.1); `pub struct StatusTokens { ready, working, needs_input, failed, slept, neutral: Hsla }`. Both derive `Debug, Clone, Copy, Serialize, Deserialize`.

- [ ] **Step 1: Add deps to `crates/lens-ui/Cargo.toml`**

Under `[dependencies]`, add (versions match the rest of the workspace — `lens-core`/`lens-client` use `serde = "1"` / `serde_json = "1"`):

```toml
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: Create `crates/lens-ui/src/theme/tokens.rs`**

```rust
use gpui::Hsla;
use serde::{Deserialize, Serialize};

/// serde `with`-module converting hex strings ↔ `Hsla`, reusing gpui-component's
/// `Colorize::parse_hex`/`to_hex`. Bare `Hsla` serde is RGBA-shaped, not hex — so every
/// `Hsla` field carries `#[serde(with = "hex_hsla")]`.
pub(crate) mod hex_hsla {
    use gpui::Hsla;
    use gpui_component::Colorize; // parse_hex + to_hex; reachable via crate-root glob re-export
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Hsla, D::Error> {
        let s = String::deserialize(d)?;
        Hsla::parse_hex(&s).map_err(|e| D::Error::custom(format!("bad hex {s:?}: {e}")))
    }
    pub fn serialize<S: Serializer>(c: &Hsla, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&c.to_hex())
    }
}

/// The curated base subset we author. Maps onto gpui-component `ThemeConfigColors` in the
/// adapter (`theme::to_theme_config`); interaction families (`*_hover`/`*_active`/`*_foreground`)
/// are NOT authored — `apply_config` derives them. Starting cut; grow/shrink is data + one
/// adapter line, never call-site churn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BaseTokens {
    // surfaces
    #[serde(with = "hex_hsla")] pub background: Hsla,
    #[serde(with = "hex_hsla")] pub foreground: Hsla,
    #[serde(with = "hex_hsla")] pub border: Hsla,
    #[serde(with = "hex_hsla")] pub muted: Hsla,
    #[serde(with = "hex_hsla")] pub muted_foreground: Hsla,
    #[serde(with = "hex_hsla")] pub popover: Hsla,
    #[serde(with = "hex_hsla")] pub popover_foreground: Hsla,
    #[serde(with = "hex_hsla")] pub accent: Hsla,
    #[serde(with = "hex_hsla")] pub accent_foreground: Hsla,
    // chrome
    #[serde(with = "hex_hsla")] pub sidebar: Hsla,
    #[serde(with = "hex_hsla")] pub sidebar_foreground: Hsla,
    #[serde(with = "hex_hsla")] pub sidebar_border: Hsla,
    #[serde(with = "hex_hsla")] pub title_bar: Hsla,
    #[serde(with = "hex_hsla")] pub title_bar_border: Hsla,
    #[serde(with = "hex_hsla")] pub tab: Hsla,
    #[serde(with = "hex_hsla")] pub tab_active: Hsla,
    #[serde(with = "hex_hsla")] pub tab_active_foreground: Hsla,
    #[serde(with = "hex_hsla")] pub tab_foreground: Hsla,
    // controls
    #[serde(with = "hex_hsla")] pub input: Hsla, // gpui-component `input` is the input *border* color
    #[serde(with = "hex_hsla")] pub caret: Hsla,
    #[serde(with = "hex_hsla")] pub ring: Hsla,
    #[serde(with = "hex_hsla")] pub selection: Hsla,
    #[serde(with = "hex_hsla")] pub scrollbar: Hsla,
    #[serde(with = "hex_hsla")] pub scrollbar_thumb: Hsla,
    #[serde(with = "hex_hsla")] pub list: Hsla,
    #[serde(with = "hex_hsla")] pub list_active: Hsla,
    #[serde(with = "hex_hsla")] pub list_hover: Hsla,
    #[serde(with = "hex_hsla")] pub progress_bar: Hsla,
    // generic component-state
    #[serde(with = "hex_hsla")] pub success: Hsla,
    #[serde(with = "hex_hsla")] pub warning: Hsla,
    #[serde(with = "hex_hsla")] pub danger: Hsla,
    #[serde(with = "hex_hsla")] pub info: Hsla,
    // overlay scrim
    #[serde(with = "hex_hsla")] pub overlay: Hsla,
}

/// One saturated color per wave state. Consumers use it directly or a derived tint via
/// `Colorize::opacity/mix` (the mixes are code, not tokens — D2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusTokens {
    #[serde(with = "hex_hsla")] pub ready: Hsla,
    #[serde(with = "hex_hsla")] pub working: Hsla,
    #[serde(with = "hex_hsla")] pub needs_input: Hsla,
    #[serde(with = "hex_hsla")] pub failed: Hsla,
    #[serde(with = "hex_hsla")] pub slept: Hsla,
    #[serde(with = "hex_hsla")] pub neutral: Hsla,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::Colorize;

    #[test]
    fn hex_round_trips_through_status_tokens() {
        let json = r#"{
            "ready": "#4c8dff", "working": "#36c98a", "needs_input": "#ff8a3d",
            "failed": "#ff5d5d", "slept": "#7a8493", "neutral": "#374151"
        }"#;
        let s: StatusTokens = serde_json::from_str(json).expect("parse");
        // parse_hex → field → to_hex → parse_hex is stable for a sample token.
        assert_eq!(s.ready.to_hex(), Hsla::parse_hex("#4c8dff").unwrap().to_hex());
    }
}
```

- [ ] **Step 3: Create `crates/lens-ui/src/theme/mod.rs` (skeleton for now)**

```rust
//! §18 theming substrate — semantic token surface for lens-ui.
mod tokens;

pub use tokens::{BaseTokens, StatusTokens};
```

- [ ] **Step 4: Wire the module in `crates/lens-ui/src/lib.rs`**

Add `pub mod theme;` alongside the other `pub mod` lines (after `pub mod slot;`). Leave re-exports for Task 4.

- [ ] **Step 5: Run the test — expect PASS**

Run: `cargo test -p lens-ui theme::tokens -- --nocapture`
Expected: `hex_round_trips_through_status_tokens` PASS. (`use gpui_component::Colorize` in the test brings `to_hex`/`parse_hex` into scope.)

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/Cargo.toml crates/lens-ui/src/lib.rs crates/lens-ui/src/theme/
git commit -m "feat(theme): token structs + hex serde module"
```

---

### Task 2: `LensTheme` + `parse_theme` + the two theme JSON files

**Files:**
- Modify: `crates/lens-ui/src/theme/mod.rs`
- Create: `crates/lens-ui/src/theme/lens-dark.json`
- Create: `crates/lens-ui/src/theme/lens-light.json`

**Interfaces:**
- Consumes: `BaseTokens`, `StatusTokens` (Task 1).
- Produces: `pub struct LensTheme { name: SharedString, mode: ThemeMode, base: BaseTokens, status: StatusTokens }` (derives `Debug, Clone, Serialize, Deserialize`, `impl gpui::Global`); `pub(crate) fn parse_theme(json: &str, expected: ThemeMode) -> anyhow::Result<LensTheme>`; test-only `fn contrast_ratio(a: Hsla, b: Hsla) -> f32`.

- [ ] **Step 1: Create `crates/lens-ui/src/theme/lens-dark.json`**

```json
{
  "name": "Lens Dark",
  "mode": "dark",
  "base": {
    "background": "#07080b",
    "foreground": "#eef2f7",
    "border": "#222936",
    "muted": "#151922",
    "muted_foreground": "#9aa4b3",
    "popover": "#1c2230",
    "popover_foreground": "#eef2f7",
    "accent": "#4c8dff",
    "accent_foreground": "#0b1220",
    "sidebar": "#07080b",
    "sidebar_foreground": "#9aa4b3",
    "sidebar_border": "#222936",
    "title_bar": "#07080b",
    "title_bar_border": "#222936",
    "tab": "#101319",
    "tab_active": "#1c2230",
    "tab_active_foreground": "#eef2f7",
    "tab_foreground": "#9aa4b3",
    "input": "#2c3442",
    "caret": "#4c8dff",
    "ring": "#4c8dff",
    "selection": "#4c8dff",
    "scrollbar": "#101319",
    "scrollbar_thumb": "#2c3442",
    "list": "#101319",
    "list_active": "#1c2230",
    "list_hover": "#151922",
    "progress_bar": "#4c8dff",
    "success": "#36c98a",
    "warning": "#ff8a3d",
    "danger": "#ff5d5d",
    "info": "#4c8dff",
    "overlay": "#000000"
  },
  "status": {
    "ready": "#4c8dff",
    "working": "#36c98a",
    "needs_input": "#ff8a3d",
    "failed": "#ff5d5d",
    "slept": "#7a8493",
    "neutral": "#374151"
  }
}
```

- [ ] **Step 2: Create `crates/lens-ui/src/theme/lens-light.json`**

```json
{
  "name": "Lens Light",
  "mode": "light",
  "base": {
    "background": "#ffffff",
    "foreground": "#1c2230",
    "border": "#d6dbe4",
    "muted": "#f2f4f8",
    "muted_foreground": "#5f6a7a",
    "popover": "#ffffff",
    "popover_foreground": "#1c2230",
    "accent": "#2f6bd8",
    "accent_foreground": "#ffffff",
    "sidebar": "#f7f8fb",
    "sidebar_foreground": "#5f6a7a",
    "sidebar_border": "#d6dbe4",
    "title_bar": "#f7f8fb",
    "title_bar_border": "#d6dbe4",
    "tab": "#eef1f6",
    "tab_active": "#ffffff",
    "tab_active_foreground": "#1c2230",
    "tab_foreground": "#5f6a7a",
    "input": "#c2c9d4",
    "caret": "#2f6bd8",
    "ring": "#2f6bd8",
    "selection": "#bcd3ff",
    "scrollbar": "#eef1f6",
    "scrollbar_thumb": "#c2c9d4",
    "list": "#ffffff",
    "list_active": "#eef1f6",
    "list_hover": "#f2f4f8",
    "progress_bar": "#2f6bd8",
    "success": "#1f9d6b",
    "warning": "#d9701f",
    "danger": "#d43d3d",
    "info": "#2f6bd8",
    "overlay": "#0b1220"
  },
  "status": {
    "ready": "#2f6bd8",
    "working": "#1f9d6b",
    "needs_input": "#d9701f",
    "failed": "#d43d3d",
    "slept": "#6b7280",
    "neutral": "#c2c9d4"
  }
}
```

- [ ] **Step 3: Write the failing tests in `crates/lens-ui/src/theme/mod.rs`**

Replace the file body with the struct + parse fn + tests (implementation of `parse_theme` comes in Step 5; write it now so the tests compile, but keep the JSON files as the source of truth):

```rust
//! §18 theming substrate — semantic token surface for lens-ui.
mod tokens;

pub use tokens::{BaseTokens, StatusTokens};

use anyhow::ensure;
use gpui::SharedString;
use gpui_component::ThemeMode;
use serde::{Deserialize, Serialize};

const DARK_JSON: &str = include_str!("lens-dark.json");
const LIGHT_JSON: &str = include_str!("lens-light.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensTheme {
    pub name: SharedString,
    pub mode: ThemeMode,
    pub base: BaseTokens,
    pub status: StatusTokens,
    // groups 3 (terminal) + 4 (diff): shapes in spec §5, added with their consuming surface.
}
impl gpui::Global for LensTheme {}

/// Pure: parse + validate mode. No I/O, no env — fully unit-testable.
pub(crate) fn parse_theme(json: &str, expected: ThemeMode) -> anyhow::Result<LensTheme> {
    let t: LensTheme = serde_json::from_str(json)?;
    ensure!(
        t.mode == expected,
        "theme mode {:?} != expected {:?} for this file",
        t.mode,
        expected
    );
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Hsla;
    use gpui_component::Colorize;

    /// WCAG relative luminance of a color.
    fn luminance(c: Hsla) -> f32 {
        let rgba: gpui::Rgba = c.into();
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(rgba.r) + 0.7152 * lin(rgba.g) + 0.0722 * lin(rgba.b)
    }

    /// WCAG contrast ratio between two colors (>= 1.0).
    fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn both_embedded_themes_parse_with_matching_mode() {
        let dark = parse_theme(DARK_JSON, ThemeMode::Dark).expect("dark parses");
        let light = parse_theme(LIGHT_JSON, ThemeMode::Light).expect("light parses");
        assert_eq!(dark.name, SharedString::from("Lens Dark"));
        assert_eq!(light.name, SharedString::from("Lens Light"));
    }

    #[test]
    fn mode_mismatch_is_rejected() {
        // A wrong `mode` in a file would flip the global mode and re-select a different
        // file on next reload — guard against it.
        assert!(parse_theme(DARK_JSON, ThemeMode::Light).is_err());
    }

    #[test]
    fn dark_status_matches_board_home_seed() {
        // Seeds from board-home.html; when intentionally retuned, update render + this test together.
        let d = parse_theme(DARK_JSON, ThemeMode::Dark).unwrap();
        assert_eq!(d.status.ready.to_hex(), Hsla::parse_hex("#4c8dff").unwrap().to_hex());
        assert_eq!(d.status.working.to_hex(), Hsla::parse_hex("#36c98a").unwrap().to_hex());
        assert_eq!(d.status.needs_input.to_hex(), Hsla::parse_hex("#ff8a3d").unwrap().to_hex());
        assert_eq!(d.status.failed.to_hex(), Hsla::parse_hex("#ff5d5d").unwrap().to_hex());
        assert_eq!(d.status.slept.to_hex(), Hsla::parse_hex("#7a8493").unwrap().to_hex());
        assert_eq!(d.status.neutral.to_hex(), Hsla::parse_hex("#374151").unwrap().to_hex());
    }

    #[test]
    fn light_expresses_distinctly_from_dark() {
        let dark = parse_theme(DARK_JSON, ThemeMode::Dark).unwrap();
        let light = parse_theme(LIGHT_JSON, ThemeMode::Light).unwrap();
        // cheap "not dark-baked" check: distinct background, and light fg darker than its bg.
        assert_ne!(light.base.background.to_hex(), dark.base.background.to_hex());
        assert!(luminance(light.base.foreground) < luminance(light.base.background));
    }

    #[test]
    fn active_status_colors_clear_3to1_on_card_surface() {
        // The five active status.* (excluding neutral, rendered via muted_foreground) must be
        // legible as small text against the card surface (base.list). Durable guard.
        for json in [DARK_JSON, LIGHT_JSON] {
            let t: LensTheme = serde_json::from_str(json).unwrap();
            let surface = t.base.list;
            for (name, c) in [
                ("ready", t.status.ready),
                ("working", t.status.working),
                ("needs_input", t.status.needs_input),
                ("failed", t.status.failed),
                ("slept", t.status.slept),
            ] {
                let ratio = contrast_ratio(c, surface);
                assert!(ratio >= 3.0, "{} status {name} contrast {ratio:.2} < 3:1", t.name);
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they compile and pass**

Run: `cargo test -p lens-ui theme:: -- --nocapture`
Expected: all `theme::tests::*` PASS. If `active_status_colors_clear_3to1_on_card_surface` fails for a specific placeholder, that is a *value* to retune in the JSON (spec §4 says values are placeholders set to pass) — nudge that color's lightness in the JSON until ≥3:1, not the test threshold.

- [ ] **Step 5: Confirm `parse_theme` behavior is real (not a stub)**

Confirm `parse_theme` is the version written in Step 3 (`serde_json::from_str` + mode `ensure!`), not a placeholder. No separate change needed if Step 3 was written as shown.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/theme/
git commit -m "feat(theme): LensTheme + parse_theme + Lens Dark/Light JSON"
```

---

### Task 3: Loading + mode selection (off-thread-ready, dir-injected)

**Files:**
- Modify: `crates/lens-ui/src/theme/mod.rs`

**Interfaces:**
- Consumes: `parse_theme`, `LensTheme` (Task 2).
- Produces: `pub(crate) fn load(mode: ThemeMode, dir: &Path) -> anyhow::Result<LensTheme>` (external file only, no fallback); `pub(crate) fn load_or_embedded(mode: ThemeMode, dir: Option<&Path>) -> anyhow::Result<LensTheme>` (external-or-embedded); `pub(crate) fn select_mode(cx: &App) -> ThemeMode`.

- [ ] **Step 1: Write the failing tests (append to `mod tests` in `mod.rs`)**

```rust
    #[test]
    fn external_file_overrides_embedded() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lens-theme-test-override-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut modified: LensTheme = serde_json::from_str(DARK_JSON).unwrap();
        modified.base.background = Hsla::parse_hex("#123456").unwrap();
        let json = serde_json::to_string(&modified).unwrap();
        std::fs::File::create(dir.join("lens-dark.json")).unwrap().write_all(json.as_bytes()).unwrap();

        let loaded = super::load_or_embedded(ThemeMode::Dark, Some(&dir)).expect("load ok");
        assert_eq!(loaded.base.background.to_hex(), Hsla::parse_hex("#123456").unwrap().to_hex());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_external_file_falls_back_to_embedded() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lens-theme-test-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::File::create(dir.join("lens-dark.json")).unwrap().write_all(b"{ not json").unwrap();

        // load_or_embedded() falls back to embedded (Ok, no panic); load() surfaces the Err.
        let loaded = super::load_or_embedded(ThemeMode::Dark, Some(&dir)).expect("falls back");
        assert_eq!(loaded.name, SharedString::from("Lens Dark"));
        assert!(super::load(ThemeMode::Dark, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-ui theme::tests::external theme::tests::bad -- --nocapture`
Expected: FAIL — `load` / `load_or_embedded` not defined.

- [ ] **Step 3: Add the loaders + selector to `mod.rs`**

Add imports at the top (`use gpui::App;`, `use std::path::Path;`) and:

```rust
/// Off-thread I/O: read + parse the external file for `mode`. Err on missing/unreadable/malformed.
/// No fallback — the reload path uses this so a bad edit → Err → keep the current theme.
pub(crate) fn load(mode: ThemeMode, dir: &Path) -> anyhow::Result<LensTheme> {
    let file = if mode.is_dark() { "lens-dark.json" } else { "lens-light.json" };
    let path = dir.join(file);
    let s = std::fs::read_to_string(&path)?;
    parse_theme(&s, mode)
}

/// Off-thread I/O: external file wins if present+valid; otherwise the embedded default.
/// Returns Err only if the *embedded* default is bad (a build bug). Used at startup.
pub(crate) fn load_or_embedded(mode: ThemeMode, dir: Option<&Path>) -> anyhow::Result<LensTheme> {
    if let Some(dir) = dir {
        match load(mode, dir) {
            Ok(lens) => return Ok(lens),
            Err(e) => eprintln!(
                "lens-theme: {}/{} — using embedded default: {e}",
                dir.display(),
                if mode.is_dark() { "lens-dark.json" } else { "lens-light.json" }
            ),
        }
    }
    let embedded = if mode.is_dark() { DARK_JSON } else { LIGHT_JSON };
    parse_theme(embedded, mode)
}

/// Resolve mode: LENS_THEME override (warn on unknown value) else the current gpui-component
/// mode (synced from the OS by `gpui_component::init`).
pub(crate) fn select_mode(cx: &App) -> ThemeMode {
    use gpui_component::Theme;
    match std::env::var("LENS_THEME").ok().as_deref() {
        Some("light") => ThemeMode::Light,
        Some("dark") => ThemeMode::Dark,
        Some(other) => {
            eprintln!("lens-theme: ignoring LENS_THEME={other:?}");
            Theme::global(cx).mode
        }
        None => Theme::global(cx).mode,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-ui theme:: -- --nocapture`
Expected: all `theme::tests::*` PASS (including the two new loader tests). `select_mode` is compiled but exercised by the bridge/manual tests later.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-ui/src/theme/mod.rs
git commit -m "feat(theme): external/embedded loaders + mode selection"
```

---

### Task 4: Accessor + bridge adapter + apply (the gpui-component bridge)

**Files:**
- Modify: `crates/lens-ui/src/theme/mod.rs`
- Modify: `crates/lens-ui/src/lib.rs`

**Interfaces:**
- Consumes: `LensTheme`, `load` (earlier tasks).
- Produces: `pub trait ActiveLensTheme { fn lens_theme(&self) -> &LensTheme; }` (impl for `App`); `impl LensTheme { pub fn global(cx: &App) -> &LensTheme }`; `pub(crate) fn to_theme_config(lens: &LensTheme) -> ThemeConfig`; `pub(crate) fn apply(lens: LensTheme, cx: &mut App)`. Re-exported from `lib.rs`: `ActiveLensTheme`, `LensTheme`.

- [ ] **Step 1: Add the accessor + adapter + apply to `mod.rs`**

Add imports (`use std::rc::Rc;`, `use gpui_component::{Colorize, Theme, ThemeConfig};`, `use gpui_component::theme::ThemeConfigColors;`) and:

```rust
pub trait ActiveLensTheme {
    fn lens_theme(&self) -> &LensTheme;
}
impl ActiveLensTheme for App {
    #[inline(always)]
    fn lens_theme(&self) -> &LensTheme {
        LensTheme::global(self)
    }
}
impl LensTheme {
    #[inline(always)]
    pub fn global(cx: &App) -> &LensTheme {
        cx.global::<LensTheme>()
    }
}

/// Build a gpui-component `ThemeConfig` from our base tokens. `apply_config` derives every
/// interaction family (`*_hover`/`*_active`/`*_foreground`) from these; we leave those + fonts/
/// radius/highlight `None` → gpui-component defaults.
pub(crate) fn to_theme_config(lens: &LensTheme) -> ThemeConfig {
    let b = &lens.base;
    let hex = |c: gpui::Hsla| Some(SharedString::from(c.to_hex()));
    ThemeConfig {
        name: lens.name.clone(),
        mode: lens.mode,
        colors: ThemeConfigColors {
            background: hex(b.background),
            foreground: hex(b.foreground),
            border: hex(b.border),
            muted: hex(b.muted),
            muted_foreground: hex(b.muted_foreground),
            popover: hex(b.popover),
            popover_foreground: hex(b.popover_foreground),
            // brand color seeds `primary` (buttons/switch/checkbox read primary, NOT accent);
            // `secondary` (subtle button bg) from muted; gpui-component `accent` (menuitem hover
            // bg) from list_hover. *_hover/*_active/*_foreground left None → derived.
            primary: hex(b.accent),
            primary_foreground: hex(b.accent_foreground),
            secondary: hex(b.muted),
            accent: hex(b.list_hover),
            input: hex(b.input),
            caret: hex(b.caret),
            ring: hex(b.ring),
            selection: hex(b.selection),
            scrollbar: hex(b.scrollbar),
            scrollbar_thumb: hex(b.scrollbar_thumb),
            list: hex(b.list),
            list_active: hex(b.list_active),
            list_hover: hex(b.list_hover),
            progress_bar: hex(b.progress_bar),
            sidebar: hex(b.sidebar),
            sidebar_foreground: hex(b.sidebar_foreground),
            sidebar_border: hex(b.sidebar_border),
            title_bar: hex(b.title_bar),
            title_bar_border: hex(b.title_bar_border),
            tab: hex(b.tab),
            tab_active: hex(b.tab_active),
            tab_active_foreground: hex(b.tab_active_foreground),
            tab_foreground: hex(b.tab_foreground),
            success: hex(b.success),
            warning: hex(b.warning),
            danger: hex(b.danger),
            info: hex(b.info),
            overlay: hex(b.overlay),
            ..Default::default()
        },
        highlight: None,
        ..Default::default()
    }
}

/// Foreground-thread, pure (no I/O): install both globals. gpui-component widgets read
/// `cx.theme()` on paint; our surfaces read `cx.lens_theme()`.
pub(crate) fn apply(lens: LensTheme, cx: &mut App) {
    Theme::global_mut(cx).apply_config(&Rc::new(to_theme_config(&lens)));
    cx.set_global(lens);
}
```

- [ ] **Step 2: Re-export from `crates/lens-ui/src/lib.rs`**

Add to the `pub use` block:

```rust
pub use theme::{ActiveLensTheme, LensTheme};
```

- [ ] **Step 3: Write the failing bridge test (append to `mod tests` in `mod.rs`)**

```rust
    #[gpui::test]
    async fn bridge_pushes_base_palette_and_survives_theme_change(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            let lens = parse_theme(DARK_JSON, ThemeMode::Dark).unwrap();
            let (accent, background) = (lens.base.accent, lens.base.background);
            super::apply(lens, cx);

            let theme = gpui_component::Theme::global(cx);
            assert_eq!(theme.mode, ThemeMode::Dark);
            assert_eq!(theme.background, background);
            assert_eq!(theme.primary, accent);
            // a derived interaction family is non-trivial (hover differs from base primary).
            assert_ne!(theme.primary_hover, theme.primary);

            // After a later Theme::change to the same mode, the palette is STILL ours
            // (config-store defeats the wipe hazard) — primary unchanged.
            gpui_component::Theme::change(ThemeMode::Dark, None, cx);
            assert_eq!(gpui_component::Theme::global(cx).primary, accent);
        });
    }
```

- [ ] **Step 4: Run the bridge test to verify it fails**

Run: `cargo test -p lens-ui theme::tests::bridge -- --nocapture`
Expected: FAIL — `apply` not yet compiled in, or assertion pending. (If `gpui_component::init(cx)` pulls heavy/headless-unfriendly setup and panics under `TestAppContext`, replace it with the theme-only init `gpui_component::theme::init(cx)` — the only prerequisite `apply` needs is that `Theme::global_mut` is installed. Verify which `init` path is headless-safe before settling the test.)

- [ ] **Step 5: Run all theme tests to verify pass**

Run: `cargo test -p lens-ui theme:: -- --nocapture`
Expected: all PASS, including the bridge test.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/theme/mod.rs crates/lens-ui/src/lib.rs
git commit -m "feat(theme): cx.lens_theme() accessor + ThemeConfig bridge + apply"
```

---

### Task 5: `ReloadTheme` action + `install_at_startup`/`reload` + BoardView handler

**Files:**
- Modify: `crates/lens-ui/src/actions.rs`
- Modify: `crates/lens-ui/src/theme/mod.rs`
- Modify: `crates/lens-ui/src/board/mod.rs`

**Interfaces:**
- Consumes: `select_mode`, `load`, `load_or_embedded`, `apply` (earlier tasks).
- Produces: `pub struct ReloadTheme` (gpui action); `pub fn install_at_startup(cx: &mut App)`; `pub fn reload(cx: &mut App, ...)` handled inside `BoardView::on_reload_theme`. Re-exported `ReloadTheme` from `lib.rs` via the existing `actions` module (already `pub mod actions;`).

- [ ] **Step 1: Add the action in `crates/lens-ui/src/actions.rs`**

```rust
use gpui::actions;

actions!(lens_ui, [BackToBoard, ReloadTheme]);
```

- [ ] **Step 2: Add `install_at_startup` + a reload helper to `theme/mod.rs`**

Add helper functions (they read `LENS_THEME_DIR` and drive load→apply). `install_at_startup` is synchronous (startup, pre-window — permitted). Add `use std::path::PathBuf;`:

```rust
/// The external theme dir override, if set.
pub(crate) fn theme_dir() -> Option<PathBuf> {
    std::env::var_os("LENS_THEME_DIR").map(PathBuf::from)
}

/// Startup install (fg thread, pre-window — synchronous read is allowed here). Resolves mode +
/// external dir, loads, applies. On Err (embedded default unparseable — a build bug), print + exit 1.
pub fn install_at_startup(cx: &mut App) {
    let mode = select_mode(cx);
    match load_or_embedded(mode, theme_dir().as_deref()) {
        Ok(lens) => apply(lens, cx),
        Err(e) => {
            eprintln!("lens-app: theme load failed (build bug): {e}");
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 3: Add the reload handler to `crates/lens-ui/src/board/mod.rs`**

Add the action + accessor imports near the top (`use crate::actions::{BackToBoard, ReloadTheme};` — merge with the existing `use crate::actions::BackToBoard;`, and `use crate::theme;`). Add the handler method next to `on_back_to_board`:

```rust
    fn on_reload_theme(&mut self, _: &ReloadTheme, _: &mut Window, cx: &mut Context<Self>) {
        // Window is live → read off the fg thread, apply on it. Bad edit → keep current theme.
        let mode = crate::theme::LensTheme::global(cx).mode;
        let dir = crate::theme::theme_dir();
        let Some(dir) = dir else {
            eprintln!("lens-theme: reload ignored — LENS_THEME_DIR not set");
            return;
        };
        cx.spawn(async move |cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { crate::theme::load(mode, &dir) })
                .await;
            cx.update(|cx| match loaded {
                Ok(lens) => {
                    crate::theme::apply(lens, cx);
                    cx.refresh_windows();
                }
                Err(e) => eprintln!("lens-theme: reload failed, keeping current theme: {e}"),
            })
            .ok();
        })
        .detach();
    }
```

To make `load`/`apply`/`theme_dir`/`LensTheme::global` reachable, ensure they are `pub(crate)`/`pub` as written (Tasks 3–4 already set `load`/`apply`/`theme_dir` to `pub(crate)` and `LensTheme::global` to `pub`).

- [ ] **Step 4: Wire the handler into the BoardView render (`board/mod.rs:~245`)**

Next to the existing `.on_action(cx.listener(Self::on_back_to_board))`, add:

```rust
            .on_action(cx.listener(Self::on_reload_theme))
```

- [ ] **Step 5: Compile-check (no new unit test — reload is verified via the §9 manual loop in Task 6)**

Run: `cargo build -p lens-ui`
Expected: compiles clean. If `cx.spawn(async move |cx| …)` arity differs, match the existing pattern in `crates/lens-ui/src/fleet/poller.rs:17` (same repo, gpui 0.2.2 — `cx.spawn(async move |cx| { … })` with `cx.background_executor().spawn(...).await` and `cx.update(|cx| …)`).

- [ ] **Step 6: Commit**

```bash
git add crates/lens-ui/src/actions.rs crates/lens-ui/src/theme/mod.rs crates/lens-ui/src/board/mod.rs
git commit -m "feat(theme): ReloadTheme action + startup install + reload handler"
```

---

### Task 6: main.rs startup install at both init sites + keybind

**Files:**
- Modify: `crates/lens-app/src/main.rs`

**Interfaces:**
- Consumes: `lens_ui::theme::install_at_startup`, `lens_ui::actions::ReloadTheme`.

- [ ] **Step 1: Import `ReloadTheme` in `main.rs`**

Change the import line 16 from:

```rust
use lens_ui::actions::BackToBoard;
```

to:

```rust
use lens_ui::actions::{BackToBoard, ReloadTheme};
```

- [ ] **Step 2: Bind the reload key in `register_keybindings`**

Change `register_keybindings` (line 109-111):

```rust
fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-.", BackToBoard, None),
        KeyBinding::new("cmd-shift-t", ReloadTheme, None),
    ]);
}
```

- [ ] **Step 3: Install the theme after `gpui_component::init(cx)` at BOTH sites**

Live run — after `gpui_component::init(cx);` (line 78) and before/after `register_keybindings(cx);`:

```rust
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        register_keybindings(cx);
```

Demo run — after `gpui_component::init(cx);` (line 118):

```rust
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        register_keybindings(cx);
```

- [ ] **Step 4: Build and run the demo — dark**

Run: `cargo run -p lens-app -- --demo`
Expected: launches; window shows the six demo cards. (Card colors still come from the old `chrome.rs` hexes until Task 7 — that's expected here; this step verifies startup install doesn't crash and the app runs.)

- [ ] **Step 5: Run the demo — light palette via env**

Run: `LENS_THEME=light cargo run -p lens-app -- --demo`
Expected: launches with the light base palette applied to any gpui-component chrome present. (Full visual proof lands after Task 7.)

- [ ] **Step 6: Commit**

```bash
git add crates/lens-app/src/main.rs
git commit -m "feat(theme): install theme at startup + bind cmd-shift-t reload"
```

---

### Task 7: A2 — hex→token migration (`wave.rs` + `chrome.rs`)

**Files:**
- Modify: `crates/lens-ui/src/card/wave.rs`
- Modify: `crates/lens-ui/src/card/chrome.rs`

**Interfaces:**
- Consumes: `LensTheme`, `ActiveLensTheme` (Task 4), `StatusTokens` (Task 1), `Wave` (existing).
- Produces: `impl Wave { pub fn status_color(self, t: &LensTheme) -> Hsla }`.

- [ ] **Step 1: Write the failing totality test in `crates/lens-ui/src/card/wave.rs`**

Add to `mod tests`:

```rust
    #[test]
    fn status_color_total_over_all_waves() {
        // Adding a Wave variant without a status arm fails to compile (exhaustive match in
        // status_color). This test just asserts every current variant resolves to some color.
        use crate::theme::parse_theme_for_test as _; // placeholder — see Step 3 note
    }
```

(Note: `status_color` is `match self { … }` — exhaustiveness is enforced by the compiler, so the real guard is compilation. The runtime test below asserts resolution against a parsed theme; write it once `status_color` exists, using the embedded dark theme.)

Replace the stub above with the real test after Step 2's helper is available:

```rust
    #[test]
    fn status_color_total_over_all_waves() {
        let t: crate::theme::LensTheme =
            serde_json::from_str(include_str!("../theme/lens-dark.json")).unwrap();
        for w in [
            Wave::Ready, Wave::Working, Wave::NeedsInput,
            Wave::Failed, Wave::Slept, Wave::Neutral,
        ] {
            let _c = w.status_color(&t); // resolves for every variant
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p lens-ui card::wave -- --nocapture`
Expected: FAIL — `status_color` not defined.

- [ ] **Step 3: Add `status_color` to `crates/lens-ui/src/card/wave.rs`**

Add the import `use crate::theme::LensTheme;` and `use gpui::Hsla;` (if not present), then:

```rust
impl Wave {
    /// The saturated status color for this wave (spec §7). Keeps `theme` a leaf — the
    /// Wave→status map lives here in `card`, not in `theme`.
    pub fn status_color(self, t: &LensTheme) -> Hsla {
        match self {
            Wave::Ready => t.status.ready,
            Wave::Working => t.status.working,
            Wave::NeedsInput => t.status.needs_input,
            Wave::Failed => t.status.failed,
            Wave::Slept => t.status.slept,
            Wave::Neutral => t.status.neutral,
        }
    }
}
```

To read `t.status.*`, ensure `LensTheme`'s `status` field and `StatusTokens`' fields are `pub` (Tasks 1–2 already set them `pub`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p lens-ui card::wave -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Migrate `crates/lens-ui/src/card/chrome.rs` hexes to tokens**

Bring the accessor into scope: `use crate::theme::ActiveLensTheme;` (and confirm `cx`/`window` is available at each call site — chrome renders inside the card view with `cx: &mut Context<..>` which derefs to `App`). Apply per the spec §7 table:

- `wave_border_color` (the 6-arm `match` at lines 84-89, currently a *wrong* palette): replace the whole helper's usage with `wave.status_color(cx.lens_theme())` at the call site. Delete the `wave_border_color` fn if it becomes unused.
- Kebab menu bg `gpui::rgb(0x1f2937)` (line 197) → `cx.lens_theme().base.popover`.
- Muted text `gpui::rgb(0x9ca3af)` (lines 223, 258, and the third occurrence) → `cx.lens_theme().base.muted_foreground`.
- Overlay text `gpui::rgb(0xf3f4f6)` (line 278) → `cx.lens_theme().base.foreground`.
- Overlay scrim `gpui::hsla(0.0, 0.0, 0.0, 0.55)` (line 274) → `cx.lens_theme().base.overlay.opacity(0.55)` (bring `use gpui_component::Colorize;` for `.opacity`).
- **Do NOT migrate** the pill fill + `pill_text_color` (lines 85-110 `pill_text_color` and the pill fill) — B1's icon-tile deletes them. Leave them untouched.

Note: `status_color`/`base.*` return `Hsla`; call sites currently pass `gpui::Rgba` (via `.into()`) into `.bg(...)`/`.text_color(...)`. `Hsla` implements `Into<Fill>`/`Into<Hsla>` for those builders — pass the `Hsla` directly (drop the trailing `.into()` where it was converting an `rgb(...)`), and adjust types if a call site required `Rgba` specifically.

- [ ] **Step 6: Run chrome tests + build**

Run: `cargo test -p lens-ui card::chrome -- --nocapture && cargo build -p lens-ui`
Expected: existing chrome unit tests (repo-row text formatting — color-agnostic) still PASS; crate builds clean.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-ui/src/card/wave.rs crates/lens-ui/src/card/chrome.rs
git commit -m "feat(theme): A2 — migrate card chrome hexes to tokens"
```

---

## Final Verification (definition of done — spec §9)

- [ ] **All theme + card tests green**

Run: `cargo test -p lens-ui`
Expected: all PASS, including the mandatory bridge test (`theme::tests::bridge_pushes_base_palette_and_survives_theme_change`).

- [ ] **Demo shows seeded wave colors (dark) and light palette**

Run: `cargo run -p lens-app -- --demo` — six cards in the seeded wave colors (post-A2).
Run: `LENS_THEME=light cargo run -p lens-app -- --demo` — same surfaces in the light palette.

- [ ] **Reload loop works without restart**

Run: `LENS_THEME_DIR=crates/lens-ui/src/theme cargo run -p lens-app -- --demo`, edit a color in `crates/lens-ui/src/theme/lens-dark.json`, press `cmd-shift-t`.
Expected: the change appears without restart (off-thread read → fg apply → `refresh_windows`). Edit the file to invalid JSON and press reload → palette unchanged + a stderr log (keep-current-on-bad-edit).

- [ ] **Gate clean**

Run: `cargo xtask gate`
Expected: no warnings / no dead code. (If `wave_border_color` or `pill_text_color`-adjacent helpers are now unused, remove the genuinely-dead ones — but keep the pill code B1 will delete; if the gate flags pill code as dead, add a scoped `#[allow(dead_code)]` with a `// removed by B1 icon-tile` note.)

- [ ] **Cross-family review of the implementation diff** (project rule): dispatch ≥1 review from a non-authoring model family over the whole branch diff before merge.

---

## Self-Review

**Spec coverage:**
- §3.1 data model (LensTheme/BaseTokens/StatusTokens) → Tasks 1–2. ✓
- §3.2 accessor (`ActiveLensTheme`, `LensTheme::global`, read-in-render constraint) → Task 4 + Global Constraints. ✓
- §3.3 bridge (`to_theme_config` + `apply` via public `apply_config`) → Task 4. ✓
- §3.4 selection/loading/reload (`select_mode`, `load`, off-thread reload, startup install, exit-on-embedded-Err) → Tasks 3, 5, 6. ✓ (with the `load` / `load_or_embedded` split correction).
- §4 theme file format + both JSONs + `hex_hsla` deserialization → Tasks 1–2. ✓
- §5 deferred group shapes → intentionally NOT built (spec says specify-don't-build); `LensTheme` doc-comment notes the slot. ✓
- §6 all ten tests → §6.1-6.5 Task 2; §6.6 Task 1; §6.7-6.8 Task 3; §6.9 Task 7; §6.10 Task 4. ✓
- §7 A2 migration + `Wave::status_color` → Task 7. ✓
- §8 files touched → all covered; **§8's "no new deps" claim corrected** (Task 1 adds serde/serde_json/anyhow). ✓
- §9 verification → Final Verification section. ✓

**Placeholder scan:** Task 7 Step 1 contains a deliberately-replaced stub with an inline instruction to replace it — the real test immediately follows. All other steps carry complete code. No `TODO`/`add error handling`/`similar to Task N`.

**Type consistency:** `status_color(self, &LensTheme) -> Hsla` used identically in Task 7 test + impl. `load`/`load_or_embedded`/`apply`/`parse_theme`/`select_mode`/`install_at_startup`/`theme_dir` signatures match across Tasks 3–6. `to_theme_config` field names all verified present in `ThemeConfigColors` (Global Constraints). `ThemeMode`/`Theme`/`Colorize` import paths verified against gpui-component 0.5.1.
