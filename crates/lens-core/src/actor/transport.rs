//! Actor-owned transport state (never persisted — P3-3 contract).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorTransport {
    Connected,
    Reconnecting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkReason {
    Unauthorized,
    SessionFailed,
    RetriesExhausted,
    Forbidden,
    NotFound,
}
