//! Live SSE event stream: pure frame parser (`sse`), typed event taxonomy
//! (`event`), and the blocking reader-thread bridge (`reader`).
pub mod event;
pub(crate) mod normalize;
pub mod reader;
pub(crate) mod sse;

pub use event::{
    DEFERRED_EVENT_TYPES, DisconnectReason, Item, MODELED_EVENT_TYPES, MessageContentBlock,
    PresenceViewer, ResponseEvent, ServerStreamEvent, SessionEvent, SessionStatusValue,
};
pub use reader::EventStream;
