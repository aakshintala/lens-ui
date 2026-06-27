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
//! Everything else passes through unchanged, in order. The only accumulation is
//! reasoning-delta text for the synthetic `ReasoningClosed` — no call/result
//! pairing or reordering beyond the above (typed-client.md §7a). Lives on the
//! Plan 3a reader thread; never blocks the foreground.

use super::event::{ResponseEvent, ServerStreamEvent};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct Normalizer {
    /// Keys of `OutputItemDone` items already emitted: `(kind, call_id, status)`.
    /// A repeat with an identical key is a literal re-fire and is dropped.
    seen_items: HashSet<(&'static str, String, String)>,
    /// `Some` while a reasoning bracket is open (between `ReasoningStarted` and
    /// its synthetic close). Accumulates the reasoning/summary deltas.
    reasoning: Option<ReasoningAccum>,
}

#[derive(Default)]
struct ReasoningAccum {
    full_text: String,
    summary_text: String,
}

impl Normalizer {
    /// Transform one parsed event into zero, one, or two normalized events.
    /// Total — never panics on event content. Task 2 adds suppression; Task 3
    /// adds the reasoning close.
    pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
        use super::event::Item;
        match &ev {
            // ── reasoning bracket ────────────────────────────────────────────
            ServerStreamEvent::Response(ResponseEvent::ReasoningStarted) => {
                self.reasoning = Some(ReasoningAccum::default());
                vec![ev]
            }
            ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta }) => {
                if let Some(acc) = self.reasoning.as_mut() {
                    acc.full_text.push_str(delta);
                }
                vec![ev]
            }
            ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta { delta }) => {
                if let Some(acc) = self.reasoning.as_mut() {
                    acc.summary_text.push_str(delta);
                }
                vec![ev]
            }
            ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { .. })
            | ServerStreamEvent::Response(ResponseEvent::Completed) => {
                if let Some(close) = self.close_reasoning() {
                    return vec![close, ev];
                }
                vec![ev]
            }
            // ── OutputItemDone re-fire suppression (Task 2) ──────────────────
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) => {
                let key = match item {
                    Item::FunctionCall {
                        call_id, status, ..
                    } => Some(("function_call", call_id.clone(), status.clone())),
                    Item::FunctionCallOutput { call_id, .. } => {
                        Some(("function_call_output", call_id.clone(), String::new()))
                    }
                    _ => None,
                };
                if let Some(key) = key
                    && !self.seen_items.insert(key)
                {
                    return Vec::new();
                }
                vec![ev]
            }
            _ => vec![ev],
        }
    }

    /// Take the open reasoning bracket (if any) and build its synthetic close.
    fn close_reasoning(&mut self) -> Option<ServerStreamEvent> {
        let acc = self.reasoning.take()?;
        Some(ServerStreamEvent::Response(
            ResponseEvent::ReasoningClosed {
                full_text: acc.full_text,
                summary_text: acc.summary_text,
            },
        ))
    }

    /// Flush any open synthetic state at stream EOF. Task 3 emits a trailing
    /// `ReasoningClosed` for a reasoning bracket the stream ended without closing.
    pub(crate) fn flush(&mut self) -> Vec<ServerStreamEvent> {
        self.close_reasoning().into_iter().collect()
    }

    /// Clear the `OutputItemDone` dedup set. Called by the reader on
    /// `Reconnected { gap }` when `gap != Some(0)`, so `GET /items` history
    /// replay is not wrongly suppressed (typed-client §7 seam (a)).
    #[allow(dead_code)] // reconnect reader (Plan 3b-2b Task 6)
    pub(crate) fn reset_seen_items(&mut self) {
        self.seen_items.clear();
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

    fn fn_call(call_id: &str, status: &str, item_id: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCall {
                id: item_id.into(),
                call_id: call_id.into(),
                name: "sys_os_shell".into(),
                arguments: "{}".into(),
                status: status.into(),
                agent: None,
            },
        })
    }
    fn fn_out(call_id: &str, item_id: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCallOutput {
                id: item_id.into(),
                call_id: call_id.into(),
                output: "ok".into(),
            },
        })
    }

    #[test]
    fn reset_seen_items_allows_a_previously_seen_item_through() {
        let mut n = Normalizer::default();
        let first = fn_call("toolu_1", "completed", "fc_a");
        assert_eq!(n.push(first.clone()), vec![first.clone()]);
        // Without reset, an identical re-fire is suppressed:
        assert!(n.push(fn_call("toolu_1", "completed", "fc_b")).is_empty());
        // After reset (reconnect with gap != Some(0)), the same item replays:
        n.reset_seen_items();
        let replay = fn_call("toolu_1", "completed", "fc_c");
        assert_eq!(n.push(replay.clone()), vec![replay]);
    }

    #[test]
    fn literal_function_call_refire_is_suppressed() {
        let mut n = Normalizer::default();
        let first = fn_call("toolu_1", "completed", "fc_a");
        assert_eq!(n.push(first.clone()), vec![first]);
        // Identical (kind, call_id, status) — dropped, even with a different item id.
        assert!(n.push(fn_call("toolu_1", "completed", "fc_b")).is_empty());
    }

    #[test]
    fn in_progress_then_completed_is_preserved_as_two_events() {
        // Byte-grounded: the captured happy-path turn fires the same call_id once
        // in_progress, once completed (differing status) — both survive.
        let mut n = Normalizer::default();
        let ip = fn_call("toolu_1", "in_progress", "fc_a");
        let done = fn_call("toolu_1", "completed", "fc_b");
        assert_eq!(n.push(ip.clone()), vec![ip]);
        assert_eq!(n.push(done.clone()), vec![done]);
    }

    #[test]
    fn literal_function_call_output_refire_is_suppressed() {
        let mut n = Normalizer::default();
        let first = fn_out("toolu_1", "fco_a");
        assert_eq!(n.push(first.clone()), vec![first]);
        assert!(n.push(fn_out("toolu_1", "fco_b")).is_empty());
    }

    #[test]
    fn distinct_call_ids_are_independent() {
        let mut n = Normalizer::default();
        assert_eq!(n.push(fn_call("toolu_1", "completed", "a")).len(), 1);
        assert_eq!(n.push(fn_call("toolu_2", "completed", "b")).len(), 1);
    }

    #[test]
    fn non_dedup_items_are_never_suppressed() {
        // A message item has no call_id key — two messages both pass through.
        let mut n = Normalizer::default();
        let msg = ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::Message {
                id: "m1".into(),
                role: "assistant".into(),
                content: vec![],
            },
        });
        assert_eq!(n.push(msg.clone()), vec![msg.clone()]);
        assert_eq!(n.push(msg.clone()), vec![msg]);
    }

    #[test]
    fn happy_path_fixture_preserves_both_function_call_events() {
        // The fixture has the same call_id as in_progress AND completed function_call,
        // plus one function_call_output — all three survive (no literal re-fire).
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let mut n = Normalizer::default();
        let mut out = Vec::new();
        for f in &frames {
            out.extend(n.push(super::super::event::parse_event(f)));
        }
        let fc = out
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                        item: Item::FunctionCall { .. }
                    })
                )
            })
            .count();
        let fco = out
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                        item: Item::FunctionCallOutput { .. }
                    })
                )
            })
            .count();
        assert_eq!(
            fc, 2,
            "in_progress + completed function_call both preserved"
        );
        assert_eq!(fco, 1, "single function_call_output preserved");
    }

    fn rdelta(d: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: d.into() })
    }
    fn sdelta(d: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta { delta: d.into() })
    }
    fn text_delta() -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
            delta: "Hi".into(),
            message_id: None,
            index: None,
            last: None,
        })
    }

    #[test]
    fn reasoning_closes_on_first_output_text_delta_with_accumulated_text() {
        let mut n = Normalizer::default();
        assert_eq!(
            n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted)),
            vec![ServerStreamEvent::Response(ResponseEvent::ReasoningStarted)]
        );
        assert_eq!(n.push(rdelta("be")), vec![rdelta("be")]); // passes through + accumulates
        assert_eq!(n.push(rdelta("cause")), vec![rdelta("cause")]);
        assert_eq!(n.push(sdelta("sum")), vec![sdelta("sum")]);
        // First output_text.delta closes the bracket: [ReasoningClosed, the delta].
        let out = n.push(text_delta());
        assert_eq!(
            out,
            vec![
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                    full_text: "because".into(),
                    summary_text: "sum".into(),
                }),
                text_delta(),
            ]
        );
    }

    #[test]
    fn reasoning_closes_on_completed() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        let completed = ServerStreamEvent::Response(ResponseEvent::Completed);
        let out = n.push(completed.clone());
        assert_eq!(
            out,
            vec![
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                    full_text: String::new(),
                    summary_text: String::new(),
                }),
                completed,
            ]
        );
    }

    #[test]
    fn reasoning_accumulators_reset_on_reopen() {
        let mut n = Normalizer::default();
        // First bracket: accumulate "first", close on Completed.
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        n.push(rdelta("first"));
        let completed = ServerStreamEvent::Response(ResponseEvent::Completed);
        let out = n.push(completed.clone());
        assert_eq!(
            out,
            vec![
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                    full_text: "first".into(),
                    summary_text: String::new(),
                }),
                completed,
            ]
        );
        // Second bracket: accumulators must reset — "second", not "firstsecond".
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        n.push(rdelta("second"));
        let out = n.push(text_delta());
        assert_eq!(
            out,
            vec![
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                    full_text: "second".into(),
                    summary_text: String::new(),
                }),
                text_delta(),
            ]
        );
    }

    #[test]
    fn only_one_close_per_reasoning_bracket() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        assert_eq!(n.push(text_delta()).len(), 2); // close + delta
        assert_eq!(n.push(text_delta()), vec![text_delta()]); // no second close
        assert_eq!(
            n.push(ServerStreamEvent::Response(ResponseEvent::Completed)),
            vec![ServerStreamEvent::Response(ResponseEvent::Completed)]
        );
    }

    #[test]
    fn output_text_delta_without_open_reasoning_is_untouched() {
        let mut n = Normalizer::default();
        assert_eq!(n.push(text_delta()), vec![text_delta()]);
    }

    #[test]
    fn flush_closes_a_dangling_reasoning_bracket() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        n.push(rdelta("x"));
        assert_eq!(
            n.flush(),
            vec![ServerStreamEvent::Response(
                ResponseEvent::ReasoningClosed {
                    full_text: "x".into(),
                    summary_text: String::new(),
                }
            )]
        );
        assert!(n.flush().is_empty()); // idempotent — already closed
    }

    #[test]
    fn happy_path_fixture_synthesizes_one_reasoning_closed() {
        // reasoning.started (line 22) is immediately followed by output_text.delta
        // (line 25): the trigger is byte-grounded. full_text is empty (no delta
        // frames on claude-sdk). Exactly one ReasoningClosed for the turn.
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let mut n = Normalizer::default();
        let mut out = Vec::new();
        for f in &frames {
            out.extend(n.push(super::super::event::parse_event(f)));
        }
        out.extend(n.flush());
        let closes = out
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
                )
            })
            .count();
        assert_eq!(closes, 1);
        // The close lands before the first output_text.delta.
        let close_idx = out
            .iter()
            .position(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
                )
            })
            .unwrap();
        let first_text_idx = out
            .iter()
            .position(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { .. })
                )
            })
            .unwrap();
        assert!(close_idx < first_text_idx);
    }
}
