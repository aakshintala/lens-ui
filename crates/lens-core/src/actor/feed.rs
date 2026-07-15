//! Unified actor → foreground feed (§3.1). One FIFO preserves send-order across
//! Summary/Detailed interleaves (catch-up TranscriptAdvanced, Promote Rebased).

use crate::actor::summary::SummaryUpdate;
use crate::reduce::StreamUpdate;

#[derive(Clone, Debug, PartialEq)]
pub enum ActorFeed {
    Summary(SummaryUpdate),
    Detailed(StreamUpdate),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, ConnectionId, HostId, SessionId};
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
        let summary = ActorFeed::Summary(SummaryUpdate::from_state(&s));
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
}
