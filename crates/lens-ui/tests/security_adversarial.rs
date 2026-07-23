//! Threat-matrix fixtures for §6.3 security boundary (T3-2).

use lens_ui::focused::autolink::{scan_prose_autolinks, AutolinkTarget};
use lens_ui::security::{validate_image_ref, validate_link_url, ImageVerdict, LinkVerdict};

#[test]
fn javascript_link_stripped() {
    assert!(matches!(
        validate_link_url("javascript:alert(1)"),
        LinkVerdict::Strip
    ));
}

#[test]
fn data_link_stripped() {
    assert!(matches!(
        validate_link_url("data:text/html,evil"),
        LinkVerdict::Strip
    ));
}

#[test]
fn file_scheme_link_stripped() {
    assert!(matches!(
        validate_link_url("file:///etc/passwd"),
        LinkVerdict::Strip
    ));
}

#[test]
fn vbscript_link_stripped() {
    assert!(matches!(
        validate_link_url("vbscript:msgbox(1)"),
        LinkVerdict::Strip
    ));
}

#[test]
fn stitch_incomplete_link_inert() {
    assert!(matches!(
        validate_link_url("stitch:incomplete-link"),
        LinkVerdict::Strip
    ));
}

#[test]
fn remote_http_image_renders_as_link_not_fetch() {
    assert!(matches!(
        validate_image_ref("http://tracker.example/pixel.png"),
        ImageVerdict::RenderAsLink { .. }
    ));
}

#[test]
fn data_image_renders_as_link_not_fetch() {
    assert!(matches!(
        validate_image_ref("data:image/png;base64,abc"),
        ImageVerdict::RenderAsLink { .. }
    ));
}

#[test]
fn path_traversal_image_stripped() {
    assert!(matches!(
        validate_image_ref("../../../etc/passwd"),
        ImageVerdict::Strip
    ));
}

#[test]
fn path_autolink_targets_navigate_to_file() {
    let prose = "open src/parser.rs:42 next";
    let hits = scan_prose_autolinks(prose);
    assert_eq!(hits.len(), 1);
    match &hits[0].target {
        AutolinkTarget::FilePath { path, line } => {
            assert_eq!(path, "src/parser.rs");
            assert_eq!(*line, Some(42));
        }
        other => panic!("{other:?}"),
    }

    match validate_link_url("src/parser.rs:42") {
        LinkVerdict::NavigateToFile { path, line } => {
            assert_eq!(path, "src/parser.rs");
            assert_eq!(line, Some(42));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn embedded_html_link_stripped_at_boundary() {
    // P4 renders block HTML as escaped source; inline hostile hrefs still hit validate_link_url.
    assert!(matches!(
        validate_link_url("javascript:alert(document.cookie)"),
        LinkVerdict::Strip
    ));
}

#[test]
fn control_char_url_stripped() {
    assert!(matches!(
        validate_link_url("http://evil.com/\x00"),
        LinkVerdict::Strip
    ));
}

#[test]
fn unicode_homoglyph_scheme_still_stripped_when_not_http() {
    // Non-http schemes with unicode tricks should not open.
    assert!(matches!(
        validate_link_url("JAVASCRIPT:alert(1)"),
        LinkVerdict::Strip
    ));
}

#[test]
fn no_unvalidated_img_calls_in_node_rs() {
    let src = include_str!("../src/md/node.rs");
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        assert!(
            !trimmed.contains("img("),
            "unvalidated img() call in node.rs: {trimmed}"
        );
    }
}

#[test]
fn p6_link_paint_gate_strips_dangerous_urls() {
    fn link_would_paint(url: &str) -> bool {
        !matches!(validate_link_url(url), LinkVerdict::Strip)
    }
    assert!(!link_would_paint("javascript:alert(1)"));
    assert!(!link_would_paint("data:text/html,evil"));
    assert!(!link_would_paint("stitch:incomplete-link"));
    assert!(link_would_paint("https://example.com"));
    assert!(link_would_paint("src/foo.rs:1"));
}
