use crate::clock::UiClock;
use lens_core::actor::ActorFeed;
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::SessionStatusValue;

#[derive(Clone, Debug)]
pub struct SessionCard {
    pub session_id: SessionId,
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub activity_summary: String,
    pub last_completed_turn: u32,
    pub seen_turn: u32,
    pub last_completed_at: Option<i64>,
    pub connection_overlay: ConnectionOverlay,
    /// Test/instrumentation: increments on each `cx.notify` from poller folds.
    pub notify_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionOverlay {
    #[default]
    Connected,
    Reconnecting,
    Disconnected,
}

impl SessionCard {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            status: SessionStatusValue::Idle,
            title: None,
            activity_summary: String::new(),
            last_completed_turn: 0,
            seen_turn: 0,
            last_completed_at: None,
            connection_overlay: ConnectionOverlay::Connected,
            notify_count: 0,
        }
    }

    pub fn fold_feed(&mut self, frame: ActorFeed, _clock: &dyn UiClock) {
        match frame {
            ActorFeed::Summary(u) => {
                self.status = u.status;
                self.title = u.title.clone();
                self.activity_summary = u.activity_summary.clone();
                self.last_completed_turn = u.last_completed_turn;
            }
            ActorFeed::Detailed(_) => {}
        }
    }
}
