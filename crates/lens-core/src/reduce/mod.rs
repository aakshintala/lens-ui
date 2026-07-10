//! §4.1 canonical reducer — pure, deterministic, no I/O. Folds one
//! `ServerStreamEvent` into `SessionState` and returns semantic `StreamUpdate`s.

mod folds;
mod items;
mod scratch;
mod snapshot;
pub mod transforms;
pub mod update;

#[cfg(test)]
pub(crate) mod testutil;

pub use update::{StreamUpdate, Updates};

use crate::clock::Clock;
use crate::domain::SessionState;
use lens_client::stream::{ResponseEvent, ServerStreamEvent};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

/// Bench-only seam (doc-hidden, not the public contract): append one synthetic
/// assistant message through the same internal `push_item` path the reducer uses
/// — including its linear dedup scan over `state.items` (items.rs). Window-scale
/// benches use this to build a large item tail without synthesizing opaque wire
/// events. Always compiled (trivial, no extra deps) so `cargo bench -p lens-core
/// --no-run` needs no feature flag.
#[doc(hidden)]
pub fn bench_push_message(
    state: &mut SessionState,
    id: crate::domain::ItemId,
    clock: &dyn Clock,
) -> Updates {
    use crate::domain::Role;
    use crate::domain::item::{ContentBlock, ItemKind};
    items::push_item(
        state,
        id,
        ItemKind::Message {
            role: Role::Assistant,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some("bench".into()),
                data: serde_json::Value::Null,
            }],
        },
        None,
        clock,
    )
}

/// Fold one event into `state`; return which parts changed (§4.1). Total over
/// every event arm — never panics on external data (AGENTS.md).
pub fn reduce(state: &mut SessionState, event: &ServerStreamEvent, clock: &dyn Clock) -> Updates {
    if let ServerStreamEvent::Session(ev) = event
        && let Some(updates) = folds::fold_session_field(state, ev, clock)
    {
        return updates;
    }
    if let ServerStreamEvent::Response(ev) = event {
        if let Some(updates) = folds::fold_response_marker(state, ev) {
            return updates;
        }
        return match ev {
            ResponseEvent::OutputTextDelta {
                delta,
                message_id,
                index,
                ..
            } => scratch::accumulate_text(&mut state.stream, delta, message_id.as_deref(), *index),
            ResponseEvent::ReasoningStarted => {
                state
                    .stream
                    .open_reasoning
                    .get_or_insert_with(Default::default);
                smallvec![StreamUpdate::ScratchChanged(Arc::new(state.stream.clone()))]
            }
            ResponseEvent::ReasoningTextDelta { delta } => scratch::accumulate_reasoning(
                &mut state.stream,
                scratch::ReasoningKind::Full,
                delta,
            ),
            ResponseEvent::ReasoningSummaryTextDelta { delta } => scratch::accumulate_reasoning(
                &mut state.stream,
                scratch::ReasoningKind::Summary,
                delta,
            ),
            ResponseEvent::OutputItemDone { item } => match items::map_item(item) {
                // D-P1-4 / REVIEW#3: resource items produce no transcript item.
                None => smallvec![StreamUpdate::ResourcesChanged],
                Some((id, kind)) => {
                    let prev_agent = state.stream.current_agent.clone();
                    // REVIEW#7 / D-P1-14: a completed FunctionCall's sanitized agent_name becomes the
                    // current attribution agent for this and subsequent items.
                    if let crate::domain::ItemKind::FunctionCall {
                        agent_name: Some(a),
                        ..
                    } = &kind
                    {
                        state.stream.current_agent = Some(a.clone());
                    }
                    // REVIEW#5 / D-P1-12: the canonical Message supersedes the streaming preview ONLY
                    // when it is the SAME message — match by message_id (None ⇒ untracked single
                    // preview for this turn, safe to clear). An unrelated keyed preview is preserved.
                    let mut cleared = false;
                    if let crate::domain::ItemKind::Message { .. } = &kind {
                        cleared = state.stream.open_message.as_ref().is_some_and(|acc| {
                            acc.message_id.is_none()
                                || acc.message_id.as_deref() == Some(id.as_str())
                        });
                        if cleared {
                            state.stream.open_message = None;
                        }
                    }
                    let mut u = items::push_item(state, id, kind, None, clock);
                    if cleared || state.stream.current_agent != prev_agent {
                        u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
                    }
                    u
                }
            },
            ResponseEvent::Completed => {
                let mut u = items::finalize_message(state, clock);
                let mut ru = items::finalize_reasoning(state, clock);
                u.append(&mut ru);
                state.stream.turn = state.stream.turn.saturating_add(1);
                u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
                u.push(StreamUpdate::StatusChanged(state.status));
                u
            }
            ResponseEvent::ReasoningClosed { .. } => {
                let had = state.stream.open_reasoning.is_some();
                let mut u = items::finalize_reasoning(state, clock);
                if had {
                    u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
                }
                u
            }
            ResponseEvent::CompactionCompleted { total_tokens } => {
                items::push_compaction(state, *total_tokens, clock)
            }
            _ => SmallVec::new(),
        };
    }
    match event {
        ServerStreamEvent::Reconnecting { attempt } => {
            smallvec![StreamUpdate::Reconnecting { attempt: *attempt }]
        }
        ServerStreamEvent::Reconnected { gap } => snapshot::on_reconnected(state, *gap),
        ServerStreamEvent::SnapshotRestored(snap) => snapshot::fold_snapshot(state, snap),
        ServerStreamEvent::Disconnected { .. } => smallvec![StreamUpdate::Disconnected],
        ServerStreamEvent::Unknown { .. } => smallvec![],
        _ => SmallVec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState};

    fn empty_state() -> SessionState {
        SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("ag_1"),
        )
    }

    #[test]
    fn reconnecting_emits_lifecycle_marker() {
        let mut s = empty_state();
        let clock = ManualClock::new(1_700_000_000_000);
        let ev = ServerStreamEvent::Reconnecting { attempt: 1 };
        let out = reduce(&mut s, &ev, &clock);
        assert_eq!(&out[..], &[StreamUpdate::Reconnecting { attempt: 1 }]);
    }

    #[test]
    fn completed_always_emits_scratch_after_turn_bump() {
        use lens_client::stream::ResponseEvent;
        let mut s = empty_state();
        let clock = ManualClock::new(1_700_000_000_000);
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clock,
        );
        assert_eq!(s.stream.turn, 1);
        let scratch = u.iter().find_map(|update| match update {
            StreamUpdate::ScratchChanged(scratch) => Some(Arc::clone(scratch)),
            _ => None,
        });
        let scratch = scratch.expect("Completed must emit ScratchChanged for turn bump");
        assert_eq!(scratch.turn, s.stream.turn);
    }

    #[test]
    fn function_call_attribution_emits_scratch_when_agent_changes() {
        use crate::reduce::testutil::parse_response;
        let mut s = empty_state();
        let clock = ManualClock::new(1_700_000_000_000);
        let ev = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{}","call_id":"toolu_1","agent":"coder"}}"#,
        );
        let u = reduce(&mut s, &ev, &clock);
        assert_eq!(s.stream.current_agent.as_deref(), Some("coder"));
        let scratch = u.iter().find_map(|update| match update {
            StreamUpdate::ScratchChanged(scratch) => Some(Arc::clone(scratch)),
            _ => None,
        });
        let scratch = scratch.expect("FunctionCall agent attribution must emit ScratchChanged");
        assert_eq!(scratch.current_agent, s.stream.current_agent);
    }

    mod corpus {
        use super::*;
        use crate::domain::ItemKind;
        use crate::reduce::testutil::{CORPUS_FILES, decode_corpus, fresh_state};

        #[test]
        fn corpus_replay_is_deterministic() {
            for path in CORPUS_FILES {
                let events = decode_corpus(path);
                let mut a = fresh_state();
                let mut b = fresh_state();
                let clock = ManualClock::new(1_700_000_000_000);
                for ev in &events {
                    reduce(&mut a, ev, &clock);
                }
                for ev in &events {
                    reduce(&mut b, ev, &clock);
                }
                assert_eq!(a, b, "non-deterministic replay for {path}");
            }
        }

        #[test]
        fn happy_path_produces_expected_transcript_shape() {
            let events = decode_corpus("docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse");
            let mut s = fresh_state();
            let clock = ManualClock::new(1_700_000_000_000);
            for ev in &events {
                reduce(&mut s, ev, &clock);
            }
            assert!(
                s.items
                    .iter()
                    .any(|i| matches!(i.kind, ItemKind::FunctionCall { .. }))
            );
            assert!(
                s.items
                    .iter()
                    .any(|i| matches!(i.kind, ItemKind::FunctionCallOutput { .. }))
            );
            assert!(
                s.items
                    .iter()
                    .any(|i| matches!(i.kind, ItemKind::Message { .. }))
            );
            let mut ids: Vec<_> = s.items.iter().map(|i| i.id.as_str().to_string()).collect();
            let n = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), n, "duplicate item ids leaked");
        }

        #[test]
        fn deferred_wire_type_is_a_noop() {
            let mut s = fresh_state();
            let clock = ManualClock::new(1_700_000_000_000);
            let u = reduce(
                &mut s,
                &ServerStreamEvent::Unknown {
                    event_type: "session.collaboration_mode".into(),
                },
                &clock,
            );
            assert!(u.is_empty());
            assert_eq!(s.collaboration_mode, None);
        }
    }
}
