//! Live SSE event stream: pure frame parser (`sse`), typed event taxonomy
//! (`event`), and the blocking reader-thread bridge (`reader`).
pub mod event;
pub(crate) mod normalize;
pub mod reader;
pub(crate) mod sse;

/// Bench/live-test-only wrappers over `pub(crate)` pipeline stages. Not part of
/// the stable API — compiled only under `bench` or `live-tests` (both internal),
/// neither implies the other; doc-hidden so it never shows as public surface.
#[cfg(any(feature = "bench", feature = "live-tests"))]
#[doc(hidden)]
pub mod bench_api {
    use super::event::ServerStreamEvent;

    /// SSE wire frame (`event:` / `data:` block).
    pub struct SseFrame(pub(crate) super::sse::SseFrame);

    impl SseFrame {
        /// Per-frame seq peek the reader does before parse (mirror its hot path).
        pub fn sequence_number(&self) -> Option<u64> {
            self.0.sequence_number()
        }
    }

    /// Incremental SSE framer fed by the reader thread.
    #[derive(Default)]
    pub struct SseParser(pub(crate) super::sse::SseParser);

    impl SseParser {
        pub fn push(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
            self.0.push(bytes).into_iter().map(SseFrame).collect()
        }

        pub fn finish(&mut self) -> Vec<SseFrame> {
            self.0.finish().into_iter().map(SseFrame).collect()
        }
    }

    /// Deserialize one SSE frame into a typed stream event.
    pub fn parse_event(frame: &SseFrame) -> ServerStreamEvent {
        super::event::parse_event(&frame.0)
    }

    /// Stateful normalization between parse and the consumer.
    #[derive(Default)]
    pub struct Normalizer(pub(crate) super::normalize::Normalizer);

    impl Normalizer {
        pub fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
            self.0.push(ev)
        }

        pub fn flush(&mut self) -> Vec<ServerStreamEvent> {
            self.0.flush()
        }
    }
}

pub use event::{
    ChildSession, ChildTaskStatus, DEFERRED_EVENT_TYPES, DisconnectReason, ElicitationParams, Item,
    MODELED_EVENT_TYPES, MessageContentBlock, PresenceViewer, ResponseEvent, ServerStreamEvent,
    SessionEvent, SessionStatusValue,
};
pub use reader::EventStream;
