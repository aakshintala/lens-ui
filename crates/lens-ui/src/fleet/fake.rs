use crossbeam_channel::{Receiver, Sender};
use lens_core::actor::{ActorFeed, ActorOutcome, SessionCommand};
use lens_core::domain::ids::SessionId;
use std::collections::HashMap;

pub const FEED_CAPACITY: usize = 64;

pub struct FakeSessionHandles {
    pub feed_rx: async_channel::Receiver<ActorFeed>,
    pub outcomes_rx: async_channel::Receiver<ActorOutcome>,
    pub commands_tx: Sender<SessionCommand>,
}

struct FakeSession {
    feed_tx: async_channel::Sender<ActorFeed>,
    feed_rx: async_channel::Receiver<ActorFeed>,
    outcomes_tx: async_channel::Sender<ActorOutcome>,
    // Held to keep the per-session outcome/command channels open for the poller/handles.
    #[allow(dead_code)]
    outcomes_rx: async_channel::Receiver<ActorOutcome>,
    #[allow(dead_code)]
    commands_tx: Sender<SessionCommand>,
    commands_rx: Receiver<SessionCommand>,
}

pub struct FakeFleet {
    sessions: HashMap<SessionId, FakeSession>,
}

impl FakeFleet {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn spawn_session(&mut self, id: SessionId) -> FakeSessionHandles {
        let (feed_tx, feed_rx) = async_channel::bounded(FEED_CAPACITY);
        let (outcomes_tx, outcomes_rx) = async_channel::bounded(FEED_CAPACITY);
        let (commands_tx, commands_rx) = crossbeam_channel::bounded(FEED_CAPACITY);
        let handles = FakeSessionHandles {
            feed_rx: feed_rx.clone(),
            outcomes_rx: outcomes_rx.clone(),
            commands_tx: commands_tx.clone(),
        };
        self.sessions.insert(
            id,
            FakeSession {
                feed_tx,
                feed_rx,
                outcomes_tx,
                outcomes_rx,
                commands_tx,
                commands_rx,
            },
        );
        handles
    }

    pub fn feed_tx(&self, id: &SessionId) -> async_channel::Sender<ActorFeed> {
        self.sessions[id].feed_tx.clone()
    }

    pub fn push_feed(&self, id: &SessionId, frame: ActorFeed) {
        self.sessions[id]
            .feed_tx
            .try_send(frame)
            .expect("fake feed push");
    }

    pub fn try_recv_feed(&self, id: &SessionId) -> Option<ActorFeed> {
        self.sessions[id].feed_rx.try_recv().ok()
    }

    pub fn take_commands(&self, id: &SessionId) -> Vec<SessionCommand> {
        let rx = &self.sessions[id].commands_rx;
        let mut out = Vec::new();
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    pub fn push_outcome(&self, id: &SessionId, outcome: ActorOutcome) {
        let _ = self.sessions[id].outcomes_tx.try_send(outcome);
    }
}

impl Default for FakeFleet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::actor::{ActorFeed, SummaryUpdate};
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::usage::Cost;

    fn empty_summary(status: SessionStatusValue, turn: u32) -> SummaryUpdate {
        SummaryUpdate {
            status,
            title: Some("t".into()),
            last_total_tokens: None,
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: None,
            model_override: None,
            agent_name: None,
            cumulative_cost: Cost::default(),
            context_window: None,
            sandbox_status: None,
            git_branch: None,
            workspace: None,
            reasoning_effort: None,
            activity_summary: String::new(),
            last_completed_turn: turn,
            harness: None,
        }
    }

    #[test]
    fn fake_fleet_per_session_channels_are_independent() {
        let mut fleet = FakeFleet::new();
        let a = SessionId::new("a");
        let b = SessionId::new("b");
        let _ha = fleet.spawn_session(a.clone());
        let _hb = fleet.spawn_session(b.clone());
        fleet.push_feed(
            &a,
            ActorFeed::Summary(Box::new(empty_summary(SessionStatusValue::Idle, 1))),
        );
        assert!(
            fleet.feed_tx(&b).is_empty() || fleet.try_recv_feed(&b).is_none(),
            "pushing on A must not deliver on B's channel"
        );
        let frame = fleet.try_recv_feed(&a).expect("A has frame");
        assert!(matches!(frame, ActorFeed::Summary(_)));
    }
}
