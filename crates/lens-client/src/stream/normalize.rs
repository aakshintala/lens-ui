//! §7a normalization: the pure, stateful transform between the SSE parser and
//! the consumer. Two guarantees, nothing more (typed-client.md §7a):
//!   1. `OutputItemDone` re-fire suppression — a second event with the same
//!      `(kind, call_id, status)` is dropped (claude-sdk double-fires). The
//!      captured `in_progress`→`completed` progression is preserved (status
//!      differs), so this is NOT a collapse to one event per call_id.
//!   2. Synthetic `ReasoningClosed` — the stream has no reasoning-end frame;
//!      the bracket closes on the first `OutputTextDelta`/`Completed` after a
//!      `ReasoningStarted`.
//!
//! Everything else passes through unchanged, in order. No text accumulation,
//! call/result pairing, or reordering beyond the above — that is the state
//! model's job. Lives on the Plan 3a reader thread; never blocks the foreground.

use super::event::ServerStreamEvent;
use std::collections::HashSet;

#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct Normalizer {
    /// Keys of `OutputItemDone` items already emitted: `(kind, call_id, status)`.
    /// A repeat with an identical key is a literal re-fire and is dropped.
    seen_items: HashSet<(&'static str, String, String)>,
    /// `Some` while a reasoning bracket is open (between `ReasoningStarted` and
    /// its synthetic close). Accumulates the reasoning/summary deltas.
    reasoning: Option<ReasoningAccum>,
}

#[derive(Default)]
#[allow(dead_code)]
struct ReasoningAccum {
    full_text: String,
    summary_text: String,
}

#[allow(dead_code)]
impl Normalizer {
    /// Transform one parsed event into zero, one, or two normalized events.
    /// Total — never panics on event content. Task 2 adds suppression; Task 3
    /// adds the reasoning close. For now, identity.
    pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
        vec![ev]
    }

    /// Flush any open synthetic state at stream EOF. Task 3 emits a trailing
    /// `ReasoningClosed` for a reasoning bracket the stream ended without closing.
    pub(crate) fn flush(&mut self) -> Vec<ServerStreamEvent> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::event::{Item, ResponseEvent, SessionEvent, SessionStatusValue};
    use super::*;

    fn status(s: SessionStatusValue) -> ServerStreamEvent {
        ServerStreamEvent::Session(SessionEvent::Status {
            status: s,
            response_id: None,
        })
    }

    #[test]
    fn unrelated_events_pass_through_unchanged_and_in_order() {
        let mut n = Normalizer::default();
        let a = status(SessionStatusValue::Running);
        let b = ServerStreamEvent::Response(ResponseEvent::InProgress);
        let c = ServerStreamEvent::Unknown {
            event_type: "x.y".into(),
        };
        let mut out = Vec::new();
        out.extend(n.push(a.clone()));
        out.extend(n.push(b.clone()));
        out.extend(n.push(c.clone()));
        assert_eq!(out, vec![a, b, c]);
    }

    #[test]
    fn a_lone_output_item_passes_through() {
        let mut n = Normalizer::default();
        let ev = ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCallOutput {
                id: "fco_1".into(),
                call_id: "toolu_1".into(),
                output: "ok".into(),
            },
        });
        assert_eq!(n.push(ev.clone()), vec![ev]);
    }

    #[test]
    fn flush_on_empty_state_yields_nothing() {
        let mut n = Normalizer::default();
        assert!(n.flush().is_empty());
    }
}
