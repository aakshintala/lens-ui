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
