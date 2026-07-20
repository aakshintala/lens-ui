//! Engine→host presentation events (titles, hyperlinks, clipboard).

pub const PRESENTATION_CHANNEL_CAP: usize = 64;
pub const MAX_REPORTED_TITLE_CHARS: usize = 512;
pub const MAX_HYPERLINK_URI_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardLocation {
    Standard,
    Selection,
    Primary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardMimePart {
    pub mime: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnginePresentationEvent {
    TitleChanged(String),
    HyperlinkOpen {
        url: String,
    },
    ClipboardWrite {
        location: ClipboardLocation,
        contents: Vec<ClipboardMimePart>,
    },
}

/// Resolve which OSC title wins when draining presentation events.
///
/// The latest-title slot is authoritative when present; channel `TitleChanged`
/// values are wake-only in that case. With no slot title, channel titles apply
/// in FIFO order and the last one wins.
pub(crate) fn resolve_drain_title(
    slot_title: Option<String>,
    channel_titles: &[String],
) -> Option<String> {
    if let Some(title) = slot_title {
        Some(title)
    } else {
        channel_titles.last().cloned()
    }
}

/// Outcome of draining presentation channel events + the latest-title slot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct PresentationDrainResult {
    pub applied_title: Option<String>,
    pub validated_hyperlink_urls: Vec<String>,
}

/// Collect presentation drain effects from the slot + channel batch.
pub(crate) fn collect_presentation_drain(
    slot_title: Option<String>,
    channel_events: impl IntoIterator<Item = EnginePresentationEvent>,
) -> PresentationDrainResult {
    let mut channel_titles = Vec::new();
    let mut validated_hyperlink_urls = Vec::new();
    for ev in channel_events {
        match ev {
            EnginePresentationEvent::TitleChanged(title) => channel_titles.push(title),
            EnginePresentationEvent::HyperlinkOpen { url } => {
                if let Some(url) = validate_open_url(&url) {
                    validated_hyperlink_urls.push(url);
                }
            }
            EnginePresentationEvent::ClipboardWrite { .. } => {}
        }
    }
    PresentationDrainResult {
        applied_title: resolve_drain_title(slot_title, &channel_titles),
        validated_hyperlink_urls,
    }
}

/// The sole inspect-counter site for foreground presentation drain.
pub(crate) fn record_presentation_drain_inspect(
    inspect: &super::inspect::InspectShared,
    result: &PresentationDrainResult,
) {
    for _ in &result.validated_hyperlink_urls {
        inspect.record_hyperlink_open();
    }
    if result.applied_title.is_some() {
        inspect.record_title_applied();
    }
}

/// Sanitize and bound an OSC-reported title for `reported_title` only.
pub fn sanitize_reported_title(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let cu = ch as u32;
        if cu <= 0x1F || cu == 0x7F || (0x80..=0x9F).contains(&cu) {
            continue;
        }
        out.push(ch);
    }
    let trimmed = out.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C'));
    if trimmed.is_empty() {
        return None;
    }
    let bounded: String = trimmed.chars().take(MAX_REPORTED_TITLE_CHARS).collect();
    Some(bounded)
}

pub fn validate_open_url(raw: &str) -> Option<String> {
    use url::Url;

    if raw.is_empty() || raw.len() > MAX_HYPERLINK_URI_BYTES {
        return None;
    }
    if raw.as_bytes().first().is_some_and(u8::is_ascii_whitespace)
        || raw.as_bytes().last().is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    if raw.chars().any(|c| {
        let u = c as u32;
        u <= 0x1F || u == 0x7F || c.is_whitespace() || c == '\\'
    }) {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    Some(raw.to_owned())
}

fn starts_url_scheme_at(cells: &[char], i: usize) -> bool {
    fn prefix_at(cells: &[char], i: usize, prefix: &str) -> bool {
        prefix
            .chars()
            .enumerate()
            .all(|(j, ch)| cells.get(i + j) == Some(&ch))
    }
    prefix_at(cells, i, "https://") || prefix_at(cells, i, "http://")
}

/// Scan by Unicode scalar / cell index. Build a dense cell vector (one char per
/// cell) and search for `http://` / `https://` spans in cell space — never use
/// raw `str::find` byte offsets as `col`.
pub fn plain_url_covering_cell(row_text: &str, col: usize) -> Option<String> {
    let cells: Vec<char> = row_text.chars().collect();
    if col >= cells.len() {
        return None;
    }
    let mut i = 0;
    while i < cells.len() {
        if starts_url_scheme_at(&cells, i) {
            let end = cells[i..]
                .iter()
                .position(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '(' | '<' | '>'))
                .map(|rel| i + rel)
                .unwrap_or(cells.len());
            if col >= i && col < end {
                let url: String = cells[i..end].iter().collect();
                return validate_open_url(&url);
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        EnginePresentationEvent, MAX_REPORTED_TITLE_CHARS, PresentationDrainResult,
        collect_presentation_drain, plain_url_covering_cell, record_presentation_drain_inspect,
        resolve_drain_title, sanitize_reported_title, validate_open_url,
    };
    use crate::engine::inspect::{InspectEventKind, InspectShared};

    #[test]
    fn sanitize_strips_controls_and_bounds_length() {
        let dirty = format!("ab\u{0007}cd{}", "X".repeat(600));
        let clean = sanitize_reported_title(&dirty).expect("Some");
        assert!(!clean.contains('\u{0007}'));
        assert_eq!(clean.chars().count(), MAX_REPORTED_TITLE_CHARS);
        assert!(clean.starts_with("abcd"));
    }

    #[test]
    fn sanitize_trims_ascii_whitespace_before_truncate() {
        let mut s = String::from("   ");
        s.push_str(&"Y".repeat(510));
        s.push_str("   ");
        let clean = sanitize_reported_title(&s).expect("Some");
        assert_eq!(clean.chars().count(), 510);
        assert!(!clean.starts_with(' '));
        assert!(!clean.ends_with(' '));
    }

    #[test]
    fn sanitize_empty_after_controls_returns_none() {
        assert_eq!(sanitize_reported_title("\u{0007}\u{001b}"), None);
        assert_eq!(sanitize_reported_title("   "), None);
    }

    #[test]
    fn slot_authoritative_over_stale_channel_title_on_drain() {
        let resolved = resolve_drain_title(Some("FinalTitle".into()), &["Stale".into()]);
        assert_eq!(
            resolved.as_deref(),
            Some("FinalTitle"),
            "slot must win over a stale channel TitleChanged (pre-fix applied channel last → Stale)"
        );
    }

    #[test]
    fn channel_fifo_last_wins_when_slot_empty() {
        let resolved = resolve_drain_title(None, &["First".into(), "Last".into()]);
        assert_eq!(resolved.as_deref(), Some("Last"));
    }

    #[test]
    fn validate_open_url_accepts_https_rejects_dangerous() {
        assert_eq!(
            validate_open_url("https://example.com/a"),
            Some("https://example.com/a".into())
        );
        assert_eq!(validate_open_url("javascript:alert(1)"), None);
        assert_eq!(validate_open_url("data:text/html,hi"), None);
        assert_eq!(validate_open_url("file:///etc/passwd"), None);
        assert_eq!(validate_open_url(" https://example.com"), None);
        assert_eq!(validate_open_url("https://example.com "), None);
        assert_eq!(validate_open_url("https://example.com/\r\nINJECT"), None);
        assert_eq!(validate_open_url(r"https://example.com\path"), None);
        assert_eq!(validate_open_url("https://#frag"), None);
        assert_eq!(validate_open_url("https://?x"), None);
        assert_eq!(validate_open_url("http://"), None);
        assert_eq!(validate_open_url("ftp://example.com"), None);
    }

    #[test]
    fn plain_url_covering_cell_uses_cell_index_not_bytes() {
        let row = "見 https://example.com/x";
        assert_eq!(
            plain_url_covering_cell(row, 2).as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(plain_url_covering_cell(row, 0), None);
    }

    #[test]
    fn presentation_inspect_drain_records_title_applied_when_enabled() {
        let inspect = InspectShared::new(40, 8, 32);
        inspect.set_enabled(true);
        let result = collect_presentation_drain(
            Some("Applied".into()),
            std::iter::empty::<EnginePresentationEvent>(),
        );
        record_presentation_drain_inspect(&inspect, &result);
        assert_eq!(inspect.snapshot().titles_applied, 1);
        assert!(
            inspect
                .snapshot()
                .recent
                .iter()
                .any(|e| e.kind == InspectEventKind::TitleApplied)
        );
    }

    #[test]
    fn presentation_inspect_drain_records_hyperlink_open_when_enabled() {
        let inspect = InspectShared::new(40, 8, 32);
        inspect.set_enabled(true);
        let result = collect_presentation_drain(
            None,
            [EnginePresentationEvent::HyperlinkOpen {
                url: "https://example.com/x".into(),
            }],
        );
        record_presentation_drain_inspect(&inspect, &result);
        assert_eq!(inspect.snapshot().hyperlink_opens, 1);
        assert!(
            inspect
                .snapshot()
                .recent
                .iter()
                .any(|e| e.kind == InspectEventKind::HyperlinkOpen)
        );
    }

    #[test]
    fn presentation_inspect_drain_counters_zero_when_disabled() {
        let inspect = InspectShared::new(40, 8, 32);
        let result = PresentationDrainResult {
            applied_title: Some("Applied".into()),
            validated_hyperlink_urls: vec!["https://example.com/x".into()],
        };
        record_presentation_drain_inspect(&inspect, &result);
        let snap = inspect.snapshot();
        assert_eq!(snap.titles_applied, 0);
        assert_eq!(snap.hyperlink_opens, 0);
    }
}
