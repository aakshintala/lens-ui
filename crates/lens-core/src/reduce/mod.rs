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
use lens_client::stream::ServerStreamEvent;
use smallvec::SmallVec;

/// Fold one event into `state`; return which parts changed (§4.1). Total over
/// every event arm — never panics on external data (AGENTS.md).
pub fn reduce(state: &mut SessionState, event: &ServerStreamEvent, clock: &dyn Clock) -> Updates {
    // Arms are filled in Tasks 2–9; unhandled events are a no-op for now.
    let _ = (state, event, clock);
    SmallVec::new()
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
    fn reduce_stub_is_a_noop() {
        let mut s = empty_state();
        let clock = ManualClock::new(1_700_000_000_000);
        let ev = ServerStreamEvent::Reconnecting { attempt: 1 };
        let out = reduce(&mut s, &ev, &clock);
        assert!(out.is_empty());
    }
}
