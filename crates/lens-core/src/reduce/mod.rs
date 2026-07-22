//! §4.1 canonical reducer — pure, deterministic, no I/O. Folds one
//! `ServerStreamEvent` into `SessionState` and returns semantic `StreamUpdate`s.

mod folds;
mod items;
mod reconcile;
mod scratch;
mod snapshot;
pub mod transforms;
pub mod update;
pub mod view;

#[cfg(test)]
pub(crate) mod testutil;

pub(crate) use reconcile::user_text;
pub use reconcile::{LostSend, reconcile_held_landed};
pub use update::{StreamUpdate, Updates};
pub use view::{
    ViewBlock, group_work_section, pair_tool_spans, project, project_all, project_filtered,
};

/// Wire `stream::Item` → domain `(ItemId, ItemKind)` for catch-up persist (D19).
pub(crate) fn map_wire_item(
    wire: &lens_client::stream::Item,
) -> Option<(crate::domain::ItemId, crate::domain::item::ItemKind)> {
    items::map_item(wire)
}

use crate::clock::Clock;
use crate::domain::SessionState;
use crate::domain::item::ReasoningAcc;
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
    let response_id = state.active_response.clone();
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
        response_id,
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
            } => {
                let new_acc_id = if state.stream.open_message.is_none() {
                    Some(state.mint_acc_id())
                } else {
                    None
                };
                scratch::accumulate_text(
                    &mut state.stream,
                    delta,
                    message_id.as_deref(),
                    *index,
                    new_acc_id,
                )
            }
            ResponseEvent::ReasoningStarted => {
                if state.stream.open_reasoning.is_none() {
                    let acc_id = state.mint_acc_id();
                    state.stream.open_reasoning = Some(ReasoningAcc {
                        acc_id,
                        ..Default::default()
                    });
                }
                smallvec![StreamUpdate::ScratchChanged(Arc::new(state.stream.clone()))]
            }
            ResponseEvent::ReasoningTextDelta { delta } => {
                let new_acc_id = if state.stream.open_reasoning.is_none() {
                    Some(state.mint_acc_id())
                } else {
                    None
                };
                scratch::accumulate_reasoning(
                    &mut state.stream,
                    scratch::ReasoningKind::Full,
                    delta,
                    new_acc_id,
                )
            }
            ResponseEvent::ReasoningSummaryTextDelta { delta } => {
                let new_acc_id = if state.stream.open_reasoning.is_none() {
                    Some(state.mint_acc_id())
                } else {
                    None
                };
                scratch::accumulate_reasoning(
                    &mut state.stream,
                    scratch::ReasoningKind::Summary,
                    delta,
                    new_acc_id,
                )
            }
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
                    let response_id = crate::domain::ids::ResponseId::from_wire(item.response_id());
                    let mut u = items::push_item(state, id, kind, None, response_id, clock);
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
                state.active_response = None;
                u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
                u.push(StreamUpdate::StatusChanged(state.status));
                u.push(StreamUpdate::ActiveResponseChanged(None));
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
        ServerStreamEvent::Disconnected { reason } => {
            smallvec![StreamUpdate::Disconnected(*reason)]
        }
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

    mod active_response {
        use super::*;
        use crate::domain::ids::ResponseId;
        use crate::reduce::testutil::parse_response;
        use lens_client::stream::{ResponseEvent, ServerStreamEvent};
        use smallvec::SmallVec;

        fn response_in_progress(response_id: &str) -> ServerStreamEvent {
            parse_response(
                "response.in_progress",
                &format!(r#"{{"response":{{"id":"{response_id}"}}}}"#),
            )
        }

        fn output_item_done(response_id: &str) -> ServerStreamEvent {
            parse_response(
                "response.output_item.done",
                &format!(
                    r#"{{"item":{{"id":"msg_1","type":"message","role":"assistant","response_id":"{response_id}","content":[{{"type":"output_text","text":"hi"}}]}}}}"#
                ),
            )
        }

        fn reduce_batch(
            state: &mut SessionState,
            events: &[ServerStreamEvent],
            clock: &dyn Clock,
        ) -> SmallVec<[StreamUpdate; 16]> {
            let mut all = SmallVec::new();
            for ev in events {
                all.extend(reduce(state, ev, clock).into_iter());
            }
            all
        }

        #[test]
        fn in_progress_sets_active_and_emits() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            let updates = reduce(&mut s, &response_in_progress("resp_37ba30e3"), &clock);
            assert_eq!(
                s.active_response.as_ref().map(ResponseId::as_str),
                Some("resp_37ba30e3")
            );
            assert!(updates.iter().any(|u| {
                matches!(u, StreamUpdate::ActiveResponseChanged(Some(r)) if r.as_str() == "resp_37ba30e3")
            }));
        }

        #[test]
        fn terminal_response_clears_active_and_emits_none() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            reduce(&mut s, &response_in_progress("resp_37ba30e3"), &clock);
            for terminal in [
                ServerStreamEvent::Response(ResponseEvent::Completed),
                ServerStreamEvent::Response(ResponseEvent::Failed),
                ServerStreamEvent::Response(ResponseEvent::Incomplete),
                ServerStreamEvent::Response(ResponseEvent::Cancelled),
            ] {
                let mut mid = empty_state();
                reduce(&mut mid, &response_in_progress("resp_37ba30e3"), &clock);
                let updates = reduce(&mut mid, &terminal, &clock);
                assert_eq!(mid.active_response, None, "terminal {terminal:?}");
                assert!(
                    updates
                        .iter()
                        .any(|u| matches!(u, StreamUpdate::ActiveResponseChanged(None))),
                    "terminal {terminal:?}"
                );
            }
        }

        #[test]
        fn output_item_done_stamps_own_response_id() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            reduce(&mut s, &response_in_progress("resp_bcb93365"), &clock);
            reduce(&mut s, &output_item_done("resp_bcb93365"), &clock);
            assert_eq!(
                s.items
                    .last()
                    .unwrap()
                    .ctx
                    .response_id
                    .as_ref()
                    .map(ResponseId::as_str),
                Some("resp_bcb93365")
            );
        }

        #[test]
        fn wire_item_without_response_id_does_not_borrow_active() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            reduce(&mut s, &response_in_progress("resp_A"), &clock);
            let done_without_id = parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}"#,
            );
            reduce(&mut s, &done_without_id, &clock);
            assert_eq!(
                s.items
                    .last()
                    .unwrap()
                    .ctx
                    .response_id
                    .as_ref()
                    .map(ResponseId::as_str),
                None,
                "wire item without response_id must not borrow active_response"
            );
        }

        #[test]
        fn synthesized_item_falls_back_to_active_response() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            reduce(&mut s, &response_in_progress("resp_abc"), &clock);
            reduce(
                &mut s,
                &ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
                    delta: "hi".into(),
                    message_id: None,
                    index: None,
                    last: None,
                }),
                &clock,
            );
            reduce(
                &mut s,
                &ServerStreamEvent::Response(ResponseEvent::Completed),
                &clock,
            );
            assert_eq!(
                s.items
                    .last()
                    .unwrap()
                    .ctx
                    .response_id
                    .as_ref()
                    .map(ResponseId::as_str),
                Some("resp_abc")
            );
        }

        #[test]
        fn greedy_batch_active_item_none_settles_committed_item() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            let updates = reduce_batch(
                &mut s,
                &[
                    response_in_progress("resp_A"),
                    output_item_done("resp_A"),
                    ServerStreamEvent::Response(ResponseEvent::Completed),
                ],
                &clock,
            );
            assert_eq!(
                s.items
                    .last()
                    .unwrap()
                    .ctx
                    .response_id
                    .as_ref()
                    .map(ResponseId::as_str),
                Some("resp_A")
            );
            assert_eq!(s.active_response, None);
            assert!(matches!(
                updates.last(),
                Some(StreamUpdate::ActiveResponseChanged(None))
            ));
        }

        #[test]
        fn in_progress_stays_within_smallvec_inline_budget() {
            let mut s = empty_state();
            let clock = ManualClock::new(1_700_000_000_000);
            let updates = reduce(&mut s, &response_in_progress("resp_x"), &clock);
            assert!(
                updates.len() <= 2,
                "response.in_progress emitted {} updates; SmallVec inline cap is 2",
                updates.len()
            );
        }
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
