use gpui::{div, prelude::*, App, IntoElement, ParentElement, Styled, Window};

use crate::focused::autolink::{scan_prose_autolinks, AutolinkTarget};
use crate::focused::content_events::emit_navigate_to_file;
use crate::focused::{ContentKey, RowContent};
use crate::md::MarkdownView;
use crate::security::{validate_link_url, LinkVerdict};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserSegment {
    Prose(String),
    InlineCode(String),
    Fenced { lang: Option<String>, body: String },
}

pub fn split_user_segments(text: &str) -> Vec<UserSegment> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with("```") {
            let after_ticks = i + 3;
            let lang_end = text[after_ticks..]
                .find('\n')
                .map(|p| after_ticks + p)
                .unwrap_or(text.len());
            let lang = text
                .get(after_ticks..lang_end)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let body_start = if lang_end < text.len() {
                lang_end + 1
            } else {
                lang_end
            };
            let close = text[body_start..]
                .find("\n```")
                .map(|p| body_start + p + 1)
                .unwrap_or(text.len());
            let body = text[body_start..close].to_string();
            out.push(UserSegment::Fenced { lang, body });
            i = if close < text.len() { close + 4 } else { text.len() };
            continue;
        }
        if text[i..].starts_with('`') {
            let end = text[i + 1..]
                .find('`')
                .map(|p| i + 1 + p)
                .unwrap_or(text.len());
            let code = text[i + 1..end].to_string();
            out.push(UserSegment::InlineCode(code));
            i = if end < text.len() { end + 1 } else { text.len() };
            continue;
        }
        let next = text[i..].find('`').map(|p| i + p).unwrap_or(text.len());
        out.push(UserSegment::Prose(text[i..next].to_string()));
        i = next;
    }
    out
}

#[cfg(test)]
/// Paint/click gate for a scanner hit — every autolink routes through `validate_link_url`.
pub(crate) fn link_verdict_for_autolink(target: &AutolinkTarget) -> LinkVerdict {
    validate_link_url(&autolink_ref(target))
}

fn autolink_element_id(seg_ix: usize, hit_ix: usize) -> (&'static str, u64) {
    ("ual", ((seg_ix as u64) << 32) | hit_ix as u64)
}

fn autolink_ref(target: &AutolinkTarget) -> String {
    match target {
        AutolinkTarget::Url(url) => url.clone(),
        AutolinkTarget::FilePath { path, line } => match line {
            Some(line) => format!("{path}:{line}"),
            None => path.clone(),
        },
    }
}

pub fn render_user_content(
    content: &RowContent,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let RowContent::UserVerbatim { text } = content else {
        return div().into_any_element();
    };
    let mut root = div().flex().flex_col().gap_1();
    for (seg_ix, seg) in split_user_segments(text).into_iter().enumerate() {
        match seg {
            UserSegment::Prose(prose) => {
                root = root.child(render_prose_with_autolinks(&prose, seg_ix, cx));
            }
            UserSegment::InlineCode(code) => {
                root = root.child(div().font_family("monospace").child(code));
            }
            UserSegment::Fenced { lang, body } => {
                let lang_l = lang.as_deref().map(|s| s.to_ascii_lowercase());
                match lang_l.as_deref() {
                    Some("md") | Some("markdown") => {
                        let key = ContentKey::from_label(format!("user-md-{seg_ix}"));
                        root = root.child(
                            MarkdownView::new(key.as_element_id(), body, window, cx)
                                .scrollable(false)
                                .selectable(true)
                                .into_inner(),
                        );
                    }
                    Some(other) => {
                        root = root.child(
                            div()
                                .font_family("monospace")
                                .child(format!("[{other}]\n{body}")),
                        );
                    }
                    None => {
                        root = root.child(div().font_family("monospace").child(body));
                    }
                }
            }
        }
    }
    root.into_any_element()
}

fn render_prose_with_autolinks(prose: &str, seg_ix: usize, _cx: &mut App) -> gpui::AnyElement {
    let hits = scan_prose_autolinks(prose);
    if hits.is_empty() {
        return div()
            .whitespace_normal()
            .child(prose.to_string())
            .into_any_element();
    }
    let mut row = div().flex().flex_row().flex_wrap();
    let mut cursor = 0usize;
    for (hit_ix, hit) in hits.into_iter().enumerate() {
        if hit.range.start > cursor {
            row = row.child(prose[cursor..hit.range.start].to_string());
        }
        let label = prose[hit.range.clone()].to_string();
        let ref_str = autolink_ref(&hit.target);
        match validate_link_url(&ref_str) {
            LinkVerdict::Strip => {
                row = row.child(label);
            }
            LinkVerdict::AllowOpenUrl => {
                let url = ref_str;
                row = row.child(
                    div()
                        .id(autolink_element_id(seg_ix, hit_ix))
                        .cursor_pointer()
                        .text_decoration_1()
                        .child(label)
                        .on_click(move |_, _, cx| {
                            if let LinkVerdict::AllowOpenUrl = validate_link_url(&url) {
                                cx.open_url(&url);
                            }
                        }),
                );
            }
            LinkVerdict::NavigateToFile { path, line } => {
                row = row.child(
                    div()
                        .id(autolink_element_id(seg_ix, hit_ix))
                        .cursor_pointer()
                        .text_decoration_1()
                        .child(label)
                        .on_click(move |_, _, cx| {
                            let nav_ref = match line {
                                Some(line) => format!("{path}:{line}"),
                                None => path.clone(),
                            };
                            if let LinkVerdict::NavigateToFile { path, line } =
                                validate_link_url(&nav_ref)
                            {
                                emit_navigate_to_file(path, line, cx);
                            }
                        }),
                );
            }
        }
        cursor = hit.range.end;
    }
    if cursor < prose.len() {
        row = row.child(prose[cursor..].to_string());
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focused::autolink::scan_prose_autolinks;

    #[test]
    fn splits_inline_code() {
        let segs = split_user_segments("hello `x` world");
        assert_eq!(
            segs,
            vec![
                UserSegment::Prose("hello ".into()),
                UserSegment::InlineCode("x".into()),
                UserSegment::Prose(" world".into()),
            ]
        );
    }

    #[test]
    fn splits_fenced_rust() {
        let segs = split_user_segments("```rust\nfn main() {}\n```");
        assert_eq!(
            segs,
            vec![UserSegment::Fenced {
                lang: Some("rust".into()),
                body: "fn main() {}\n".into(),
            }]
        );
    }

    #[test]
    fn splits_fenced_markdown() {
        let segs = split_user_segments("```md\n# H\n```");
        assert_eq!(
            segs,
            vec![UserSegment::Fenced {
                lang: Some("md".into()),
                body: "# H\n".into(),
            }]
        );
    }

    #[test]
    fn splits_untagged_fence() {
        let segs = split_user_segments("```\nplain\n```");
        assert_eq!(
            segs,
            vec![UserSegment::Fenced {
                lang: None,
                body: "plain\n".into(),
            }]
        );
    }

    #[test]
    fn autolink_prose_not_in_inline_code() {
        let prose = "see src/a.rs:1";
        assert_eq!(scan_prose_autolinks(prose).len(), 1);
        let segs = split_user_segments("`src/a.rs:1`");
        assert!(matches!(&segs[0], UserSegment::InlineCode(_)));
        if let UserSegment::InlineCode(s) = &segs[0] {
            assert_eq!(s, "src/a.rs:1");
            // Suppression is segment-gated: inline code never reaches `render_prose_with_autolinks`.
            assert!(!matches!(&segs[0], UserSegment::Prose(_)));
        }
    }

    #[test]
    fn markdown_fence_literal_not_full_user_markdown() {
        let segs = split_user_segments("```md\n[x](javascript:alert(1))\n```");
        assert_eq!(segs.len(), 1);
        assert!(matches!(
            segs[0],
            UserSegment::Fenced {
                lang: Some(_),
                ..
            }
        ));
        if let UserSegment::Fenced { body, .. } = &segs[0] {
            assert!(body.contains("javascript:"));
        }
    }

    #[test]
    fn hostile_autolink_targets_strip() {
        for prose in ["see ../.ssh/id_rsa", "open ftp://evil"] {
            let hits = scan_prose_autolinks(prose);
            assert!(
                !hits.is_empty(),
                "scanner should detect autolink candidate in {prose:?}"
            );
            for hit in &hits {
                assert!(
                    matches!(link_verdict_for_autolink(&hit.target), LinkVerdict::Strip),
                    "hostile target {:?} must strip",
                    hit.target
                );
            }
        }
        assert!(
            scan_prose_autolinks("[x](javascript:alert(1))").is_empty(),
            "markdown syntax is literal prose, not parsed as links"
        );
        assert!(matches!(
            validate_link_url("javascript:alert(1)"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn safe_autolink_targets_validate() {
        let url_hit = scan_prose_autolinks("visit https://example.com")
            .into_iter()
            .next()
            .expect("url hit");
        assert!(matches!(
            link_verdict_for_autolink(&url_hit.target),
            LinkVerdict::AllowOpenUrl
        ));

        let path_hit = scan_prose_autolinks("see src/x.rs:10")
            .into_iter()
            .next()
            .expect("path hit");
        match link_verdict_for_autolink(&path_hit.target) {
            LinkVerdict::NavigateToFile { path, line } => {
                assert_eq!(path, "src/x.rs");
                assert_eq!(line, Some(10));
            }
            other => panic!("{other:?}"),
        }
    }
}
