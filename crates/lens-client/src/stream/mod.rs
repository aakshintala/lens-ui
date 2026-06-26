//! Live SSE event stream: pure frame parser (`sse`), typed event taxonomy
//! (`event`), and the blocking reader-thread bridge (`reader`).
pub mod event;
pub(crate) mod sse;
