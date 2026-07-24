//! Unified actor → foreground feed (§3.1). One FIFO preserves send-order across
//! Summary/Detailed interleaves (catch-up TranscriptAdvanced, Promote Rebased).

use crate::actor::outcome::{ActorOutcome, TerminalResourceSignal};
use crate::actor::summary::SummaryUpdate;
use crate::reduce::{StreamUpdate, Updates};

#[derive(Clone, Debug, PartialEq)]
pub enum ActorFeed {
    /// Boxed: `SummaryUpdate` is a wide card-chrome struct, so boxing keeps the
    /// enum (and the FIFO buffer) lean vs the small `Detailed` variant
    /// (`StreamUpdate` already boxes/Arc-wraps its large payloads).
    Summary(Box<SummaryUpdate>),
    Detailed(StreamUpdate),
}

/// Control-only `StreamUpdate`s route to `ActorOutcome`, not `ActorFeed`.
pub fn is_control_only_update(u: &StreamUpdate) -> bool {
    matches!(
        u,
        StreamUpdate::Superseded { .. }
            | StreamUpdate::TerminalResourceCreated { .. }
            | StreamUpdate::TerminalResourceDeleted { .. }
    )
}

/// Map session control deltas to foreground `ActorOutcome`s (Slice 5).
pub fn control_outcome_from_update(u: &StreamUpdate) -> Option<ActorOutcome> {
    match u {
        StreamUpdate::Superseded {
            target_conversation_id,
            reason,
        } => Some(ActorOutcome::Superseded {
            target_conversation_id: target_conversation_id.clone(),
            reason: reason.clone(),
        }),
        StreamUpdate::TerminalResourceCreated {
            terminal_id,
            terminal_name,
            session_key,
            session_id,
        } => Some(ActorOutcome::TerminalResource(
            TerminalResourceSignal::Created {
                terminal_id: terminal_id.clone(),
                terminal_name: terminal_name.clone(),
                session_key: session_key.clone(),
                session_id: session_id.clone(),
            },
        )),
        StreamUpdate::TerminalResourceDeleted { terminal_id } => Some(
            ActorOutcome::TerminalResource(TerminalResourceSignal::Deleted {
                terminal_id: terminal_id.clone(),
            }),
        ),
        _ => None,
    }
}

/// Strip control-only updates before the unified feed FIFO.
pub fn feed_updates(batch: Updates) -> Updates {
    batch
        .into_iter()
        .filter(|u| !is_control_only_update(u))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, ConnectionId, HostId, SessionId, TerminalId};
    use crate::domain::scalars::SessionStatusValue;
    use crate::domain::session::SessionState;
    use crate::domain::usage::Cost;

    #[test]
    fn feed_variants_wrap_existing_bridge_types() {
        let s = SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        );
        let summary = ActorFeed::Summary(Box::new(SummaryUpdate::from_state(&s)));
        let detailed = ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: 0,
        });
        assert!(matches!(summary, ActorFeed::Summary(_)));
        assert!(matches!(
            detailed,
            ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })
        ));
        if let ActorFeed::Summary(u) = summary {
            assert_eq!(u.last_completed_turn, 0);
            assert!(u.activity_summary.is_empty());
            assert_eq!(u.cumulative_cost, Cost::default());
            assert_eq!(u.status, SessionStatusValue::Idle);
            assert!(u.host_id.is_none());
            let _ = HostId::new("unused"); // documents HostId remains on SummaryUpdate
        }
    }

    #[test]
    fn superseded_update_maps_to_control_outcome() {
        let u = StreamUpdate::Superseded {
            target_conversation_id: "conv_b".into(),
            reason: "clear".into(),
        };
        assert!(matches!(
            control_outcome_from_update(&u),
            Some(ActorOutcome::Superseded {
                target_conversation_id,
                reason,
            }) if target_conversation_id == "conv_b" && reason == "clear"
        ));
        assert!(is_control_only_update(&u));
    }

    #[test]
    fn terminal_resource_created_maps_to_control_outcome() {
        let u = StreamUpdate::TerminalResourceCreated {
            terminal_id: TerminalId::new("terminal_tui_main"),
            terminal_name: "tui".into(),
            session_key: "main".into(),
            session_id: SessionId::new("conv_a"),
        };
        assert!(matches!(
            control_outcome_from_update(&u),
            Some(ActorOutcome::TerminalResource(TerminalResourceSignal::Created {
                terminal_id,
                terminal_name,
                session_key,
                session_id,
            })) if terminal_id.as_str() == "terminal_tui_main"
                && terminal_name == "tui"
                && session_key == "main"
                && session_id.as_str() == "conv_a"
        ));
        assert!(is_control_only_update(&u));
    }

    #[test]
    fn terminal_resource_deleted_maps_to_control_outcome() {
        let u = StreamUpdate::TerminalResourceDeleted {
            terminal_id: TerminalId::new("terminal_tui_main"),
        };
        assert!(matches!(
            control_outcome_from_update(&u),
            Some(ActorOutcome::TerminalResource(TerminalResourceSignal::Deleted {
                terminal_id,
            })) if terminal_id.as_str() == "terminal_tui_main"
        ));
        assert!(is_control_only_update(&u));
    }

    #[test]
    fn feed_updates_strips_control_only_deltas() {
        use crate::reduce::Updates;
        use smallvec::smallvec;

        let batch: Updates = smallvec![
            StreamUpdate::ResourcesChanged,
            StreamUpdate::Superseded {
                target_conversation_id: "conv_b".into(),
                reason: "clear".into(),
            },
            StreamUpdate::StatusChanged(SessionStatusValue::Running),
        ];
        let stripped = feed_updates(batch);
        assert_eq!(stripped.len(), 2);
        assert!(matches!(stripped[0], StreamUpdate::ResourcesChanged));
        assert!(matches!(
            stripped[1],
            StreamUpdate::StatusChanged(SessionStatusValue::Running)
        ));
    }
}
