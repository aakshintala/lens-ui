//! Engine→host presentation events (titles, hyperlinks, clipboard).

pub const PRESENTATION_CHANNEL_CAP: usize = 64;
#[expect(dead_code, reason = "Task 2 title sanitize bounds to this limit")]
pub const MAX_REPORTED_TITLE_CHARS: usize = 512;
#[expect(dead_code, reason = "Task 3 hyperlink validation uses this limit")]
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

#[cfg(test)]
mod tests {
    use super::resolve_drain_title;

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
