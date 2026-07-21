//! Embedded asset provider (gpui `AssetSource`). Serves the Lucide glyph SVGs the
//! card tile + spinner render via `svg().path("icons/<name>.svg")`. gpui-component
//! ships no icon SVGs, so the app MUST register this before any card paints.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Every glyph path served by `LensAssets`. Keep in sync with the files under
/// `assets/icons/`; `card::motion::wave_icon_path` returns members of this set.
pub const ICON_PATHS: [&str; 10] = [
    "icons/bell.svg",
    "icons/triangle-alert.svg",
    "icons/loader-circle.svg",
    "icons/alarm-clock.svg",
    "icons/check.svg",
    "icons/moon.svg",
    "icons/coffee.svg",
    "icons/circle-dot.svg",
    "icons/folder.svg",
    "icons/git-branch.svg",
];

/// Compile-time-embedded (path, bytes) table for the bundled Lucide SVGs.
const ICON_BYTES: &[(&str, &[u8])] = &[
    ("icons/bell.svg", include_bytes!("../assets/icons/bell.svg")),
    (
        "icons/triangle-alert.svg",
        include_bytes!("../assets/icons/triangle-alert.svg"),
    ),
    (
        "icons/loader-circle.svg",
        include_bytes!("../assets/icons/loader-circle.svg"),
    ),
    (
        "icons/alarm-clock.svg",
        include_bytes!("../assets/icons/alarm-clock.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/check.svg"),
    ),
    ("icons/moon.svg", include_bytes!("../assets/icons/moon.svg")),
    (
        "icons/coffee.svg",
        include_bytes!("../assets/icons/coffee.svg"),
    ),
    (
        "icons/circle-dot.svg",
        include_bytes!("../assets/icons/circle-dot.svg"),
    ),
    (
        "icons/folder.svg",
        include_bytes!("../assets/icons/folder.svg"),
    ),
    (
        "icons/git-branch.svg",
        include_bytes!("../assets/icons/git-branch.svg"),
    ),
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
            assert!(bytes.windows(4).any(|w| w == b"<svg"), "not an svg: {path}");
        }
    }

    #[test]
    fn list_icons_dir_returns_all_ten() {
        let listed = LensAssets.list("icons/").expect("list ok");
        assert_eq!(listed.len(), 10, "listed: {listed:?}");
    }

    #[test]
    fn unknown_path_is_none_not_err() {
        assert!(LensAssets.load("icons/nope.svg").expect("ok").is_none());
    }
}
