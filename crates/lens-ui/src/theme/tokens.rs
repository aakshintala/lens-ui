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
    #[serde(with = "hex_hsla")]
    pub background: Hsla,
    #[serde(with = "hex_hsla")]
    pub foreground: Hsla,
    #[serde(with = "hex_hsla")]
    pub border: Hsla,
    #[serde(with = "hex_hsla")]
    pub muted: Hsla,
    #[serde(with = "hex_hsla")]
    pub muted_foreground: Hsla,
    #[serde(with = "hex_hsla")]
    pub popover: Hsla,
    #[serde(with = "hex_hsla")]
    pub popover_foreground: Hsla,
    #[serde(with = "hex_hsla")]
    pub accent: Hsla,
    #[serde(with = "hex_hsla")]
    pub accent_foreground: Hsla,
    // chrome
    #[serde(with = "hex_hsla")]
    pub sidebar: Hsla,
    #[serde(with = "hex_hsla")]
    pub sidebar_foreground: Hsla,
    #[serde(with = "hex_hsla")]
    pub sidebar_border: Hsla,
    #[serde(with = "hex_hsla")]
    pub title_bar: Hsla,
    #[serde(with = "hex_hsla")]
    pub title_bar_border: Hsla,
    #[serde(with = "hex_hsla")]
    pub tab: Hsla,
    #[serde(with = "hex_hsla")]
    pub tab_active: Hsla,
    #[serde(with = "hex_hsla")]
    pub tab_active_foreground: Hsla,
    #[serde(with = "hex_hsla")]
    pub tab_foreground: Hsla,
    // controls
    #[serde(with = "hex_hsla")]
    pub input: Hsla, // gpui-component `input` is the input *border* color
    #[serde(with = "hex_hsla")]
    pub caret: Hsla,
    #[serde(with = "hex_hsla")]
    pub ring: Hsla,
    #[serde(with = "hex_hsla")]
    pub selection: Hsla,
    #[serde(with = "hex_hsla")]
    pub scrollbar: Hsla,
    #[serde(with = "hex_hsla")]
    pub scrollbar_thumb: Hsla,
    #[serde(with = "hex_hsla")]
    pub list: Hsla,
    #[serde(with = "hex_hsla")]
    pub list_active: Hsla,
    #[serde(with = "hex_hsla")]
    pub list_hover: Hsla,
    #[serde(with = "hex_hsla")]
    pub progress_bar: Hsla,
    // generic component-state
    #[serde(with = "hex_hsla")]
    pub success: Hsla,
    #[serde(with = "hex_hsla")]
    pub warning: Hsla,
    #[serde(with = "hex_hsla")]
    pub danger: Hsla,
    #[serde(with = "hex_hsla")]
    pub info: Hsla,
    // overlay scrim
    #[serde(with = "hex_hsla")]
    pub overlay: Hsla,
}

/// One saturated color per wave state. Consumers use it directly or a derived tint via
/// `Colorize::opacity/mix` (the mixes are code, not tokens — D2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StatusTokens {
    #[serde(with = "hex_hsla")]
    pub ready: Hsla,
    #[serde(with = "hex_hsla")]
    pub working: Hsla,
    #[serde(with = "hex_hsla")]
    pub needs_input: Hsla,
    #[serde(with = "hex_hsla")]
    pub failed: Hsla,
    #[serde(with = "hex_hsla")]
    pub slept: Hsla,
    #[serde(with = "hex_hsla")]
    pub neutral: Hsla,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::Colorize;

    #[test]
    fn hex_round_trips_through_status_tokens() {
        let json = r##"{
            "ready": "#4c8dff", "working": "#36c98a", "needs_input": "#ff8a3d",
            "failed": "#ff5d5d", "slept": "#7a8493", "neutral": "#374151"
        }"##;
        let s: StatusTokens = serde_json::from_str(json).expect("parse");
        // parse_hex → field → to_hex → parse_hex is stable for a sample token.
        assert_eq!(
            s.ready.to_hex(),
            Hsla::parse_hex("#4c8dff").unwrap().to_hex()
        );
    }
}
