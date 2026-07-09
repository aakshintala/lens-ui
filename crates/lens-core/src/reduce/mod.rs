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
                smallvec![StreamUpdate::ScratchChanged]
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
                    if let crate::domain::ItemKind::Message { .. } = &kind {
                        let clears = state.stream.open_message.as_ref().is_some_and(|acc| {
                            acc.message_id.is_none()
                                || acc.message_id.as_deref() == Some(id.as_str())
                        });
                        if clears {
                            state.stream.open_message = None;
                        }
                    }
                    items::push_item(state, id, kind, None, clock)
                }
            },
            ResponseEvent::Completed => {
                let mut u = items::finalize_message(state, clock);
                state.stream.turn += 1;
                u.push(StreamUpdate::StatusChanged);
                u
            }
            ResponseEvent::ReasoningClosed { .. } => items::finalize_reasoning(state, clock),
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
}
