//! `SnapshotRestored` / `Reconnected` / lifecycle folds (§4.1).

use crate::domain::{AgentId, HostId, ModelUsage, RunnerId, SessionId, SessionState, SkillSummary};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::sessions::{SessionSnapshot, SessionStatus};
use smallvec::smallvec;
use std::sync::Arc;

fn map_snapshot_status(s: SessionStatus) -> crate::domain::SessionStatusValue {
    use crate::domain::SessionStatusValue as V;
    match s {
        SessionStatus::Idle => V::Idle,
        SessionStatus::Running => V::Running,
        SessionStatus::Failed => V::Failed,
    }
}

/// D-P1-15: scalar restore ONLY — no transcript side-effects, no AgentChanged marker.
pub(crate) fn fold_snapshot(state: &mut SessionState, snap: &SessionSnapshot) -> Updates {
    state.status = map_snapshot_status(snap.status());
    state.agent_id = AgentId::new(snap.agent_id().to_string());
    state.agent_name = snap.agent_name().map(str::to_string);
    state.stream.current_agent = state.agent_name.clone();
    state.llm_model = snap.llm_model().map(str::to_string);
    state.model_override = snap.model_override().map(str::to_string);
    state.reasoning_effort = snap.reasoning_effort().map(str::to_string);
    state.context_window = snap.context_window().map(|v| v.max(0) as u64);
    state.last_total_tokens = snap.last_total_tokens().map(|v| v.max(0) as u64);
    state.cumulative_cost.total_cost_usd = snap.total_cost_usd();
    state.title = snap.title().map(str::to_string);
    state.labels = snap.labels().clone();
    state.host_id = snap.host_id().map(|h| HostId::new(h.to_string()));
    state.runner_id = snap.runner_id().map(|r| RunnerId::new(r.to_string()));
    state.workspace = snap.workspace().map(str::to_string);
    state.git_branch = snap.git_branch().map(str::to_string);
    state.parent_session_id = snap
        .parent_session_id()
        .map(|p| SessionId::new(p.to_string()));
    state.permission_level = snap.permission_level().and_then(|p| u8::try_from(p).ok());
    state.archived = snap.archived();
    state.cumulative_cost.cumulative_usage.usage_by_model = snap
        .usage_by_model()
        .iter()
        .map(|(k, mu)| {
            (
                k.clone(),
                ModelUsage {
                    input_tokens: Some(mu.input_tokens().max(0) as u64),
                    output_tokens: Some(mu.output_tokens().max(0) as u64),
                    total_tokens: Some(mu.total_tokens().max(0) as u64),
                    cache_creation_input_tokens: Some(
                        mu.cache_creation_input_tokens().max(0) as u64
                    ),
                    cache_read_input_tokens: Some(mu.cache_read_input_tokens().max(0) as u64),
                    total_cost_usd: Some(mu.total_cost_usd()),
                },
            )
        })
        .collect();
    state.skills = snap
        .skills()
        .iter()
        .map(|sk| SkillSummary {
            name: sk.name().to_string(),
            description: Some(sk.description().to_string()).filter(|d| !d.is_empty()),
        })
        .collect();
    // NOTE: snap.items() is deliberately NOT read here (D-P1-15) — history is replayed as
    // subsequent OutputItemDone events by lens-client (§7 ordering).
    smallvec![StreamUpdate::SnapshotRestored]
}

pub(crate) fn on_reconnected(state: &mut SessionState, gap: Option<u64>) -> Updates {
    let mut u: Updates = smallvec![StreamUpdate::Reconnected];
    if gap != Some(0) {
        // D-P1-16: clear transient scratch; KEEP pending_user (user intent, spec P3b).
        let had = state.stream.open_message.is_some()
            || state.stream.open_reasoning.is_some()
            || !state.stream.unpaired_calls.is_empty();
        state.stream.open_message = None;
        state.stream.open_reasoning = None;
        state.stream.unpaired_calls.clear();
        if had {
            u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
        }
    }
    u
}

#[cfg(test)]
mod tests {
    use crate::clock::ManualClock;
    use crate::domain::controls::PendingUserMessage;
    use crate::domain::item::ItemKind;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState, SessionStatusValue};
    use crate::reduce::testutil::snapshot_fixture as build_snapshot;
    use crate::reduce::{StreamUpdate, reduce};
    use lens_client::stream::{ResponseEvent, ServerStreamEvent};
    use serde_json::json;

    fn st() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }
    fn clock() -> ManualClock {
        ManualClock::new(1_700_000_000_000)
    }
    fn resp_text(delta: &str, message_id: Option<&str>, index: Option<usize>) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
            delta: delta.into(),
            message_id: message_id.map(str::to_string),
            index,
            last: None,
        })
    }
    fn pending(id: &str, content: &str) -> PendingUserMessage {
        PendingUserMessage {
            pending_id: id.into(),
            content: content.into(),
            created_at: 1_700_000_000_000,
        }
    }
    fn test_snapshot() -> lens_client::sessions::SessionSnapshot {
        build_snapshot(json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "llm_model": "opus",
            "items": [{
                "id": "msg_embed_1",
                "type": "message",
                "data": {
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "embedded history"}]
                }
            }]
        }))
    }

    #[test]
    fn reconnected_with_gap_clears_scratch_not_pending_user() {
        let mut s = st();
        reduce(&mut s, &resp_text("partial", None, None), &clock());
        s.pending_user.push(pending("p1", "hey"));
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Reconnected { gap: None },
            &clock(),
        );
        assert!(s.stream.open_message.is_none());
        assert_eq!(s.pending_user.len(), 1);
        assert!(u.contains(&StreamUpdate::Reconnected));
    }

    #[test]
    fn reconnected_gap_zero_keeps_scratch() {
        let mut s = st();
        reduce(&mut s, &resp_text("partial", None, None), &clock());
        reduce(
            &mut s,
            &ServerStreamEvent::Reconnected { gap: Some(0) },
            &clock(),
        );
        assert!(s.stream.open_message.is_some());
    }

    #[test]
    fn snapshot_restored_folds_scalars_only_no_items() {
        let mut s = st();
        let snap = test_snapshot();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::SnapshotRestored(Box::new(snap)),
            &clock(),
        );
        assert_eq!(s.status, SessionStatusValue::Running);
        assert_eq!(s.llm_model.as_deref(), Some("opus"));
        assert_eq!(s.agent_id.as_str(), "ag_9");
        assert!(s.items.is_empty());
        assert!(
            !s.items
                .iter()
                .any(|i| matches!(i.kind, ItemKind::AgentChanged { .. }))
        );
        assert_eq!(&u[..], &[StreamUpdate::SnapshotRestored]);
    }
}
