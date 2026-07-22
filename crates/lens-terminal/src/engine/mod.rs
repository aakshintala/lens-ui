pub mod command;
pub mod forwarder;
pub mod frame;
pub mod handle;
pub mod inspect;
pub mod key_map;
pub mod presentation;
pub mod vt;
pub mod worker;

#[cfg(test)]
mod reconnect_seed;

pub use frame::CursorPos;
pub use handle::{EngineHandle, FeedError};
pub use inspect::{EngineInspect, PER_CELL_BYTES};
#[expect(
    unused_imports,
    reason = "re-exported presentation surface for Slice 2d+"
)]
pub use presentation::{
    ClipboardLocation, ClipboardMimePart, EnginePresentationEvent, MAX_HYPERLINK_URI_BYTES,
    MAX_REPORTED_TITLE_CHARS, PRESENTATION_CHANNEL_CAP, sanitize_reported_title, validate_open_url,
};
pub use vt::{EngineConfig, EngineError, VtEngine};
pub use worker::{EgressFrame, EgressKind};
