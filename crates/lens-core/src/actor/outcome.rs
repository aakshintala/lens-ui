//! Unified actor → foreground outcome channel (Task 9 extends this).

use crate::actor::api::CommandOutcome;
use crate::actor::transport::{ActorTransport, ParkReason};

#[derive(Clone, Debug)]
pub enum ActorOutcome {
    Command(CommandOutcome),
    TransportChanged {
        transport: ActorTransport,
        reconcile_in_flight: bool,
    },
    Parked {
        reason: ParkReason,
    },
    StoppedRemoved,
    StoppedTombstone,
}
