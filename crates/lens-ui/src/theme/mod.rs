//! §18 theming substrate — semantic token surface for lens-ui.
mod tokens;

pub use tokens::{BaseTokens, StatusTokens};

use anyhow::ensure;
use gpui::App;
use gpui::SharedString;
use gpui_component::ThemeMode;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    fn external_file_overrides_embedded() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lens-theme-test-override-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut modified: LensTheme = serde_json::from_str(DARK_JSON).unwrap();
        modified.base.background = Hsla::parse_hex("#123456").unwrap();
        let json = serde_json::to_string(&modified).unwrap();
        std::fs::File::create(dir.join("lens-dark.json")).unwrap().write_all(json.as_bytes()).unwrap();

        let loaded = super::load_or_embedded(ThemeMode::Dark, Some(&dir)).expect("load ok");
        // External file wins: the loaded background matches the value written to disk (compared as
        // Hsla against a re-parse of the SAME json — gpui-component's hex<->Hsla is lossy per cycle,
        // so never compare to_hex values that crossed the hex boundary a different number of times),
        // and differs from the embedded default.
        let from_file: LensTheme = serde_json::from_str(&json).unwrap();
        let embedded: LensTheme = serde_json::from_str(DARK_JSON).unwrap();
        assert_eq!(loaded.base.background, from_file.base.background);
        assert_ne!(loaded.base.background, embedded.base.background);
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
