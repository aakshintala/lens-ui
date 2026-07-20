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

#[cfg(test)]
mod tests {
    use super::{MAX_REPORTED_TITLE_CHARS, resolve_drain_title, sanitize_reported_title};

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
}
