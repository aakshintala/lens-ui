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
