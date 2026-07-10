//! Actor-owned transport state (never persisted — P3-3 contract).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorTransport {
    Connected,
    Reconnecting,
    /// Recoverable terminal — actor + state resident; awaiting re-auth / user retry.
    Parked {
        reason: ParkReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkReason {
    Unauthorized,
    SessionFailed,
    RetriesExhausted,
}
