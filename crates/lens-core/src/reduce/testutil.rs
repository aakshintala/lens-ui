#![allow(dead_code)] // consumed by reducer tests in Tasks 2–11

use lens_client::stream::{ServerStreamEvent, decode_all};

/// Decode a single SSE frame (event + JSON data) into exactly one typed event.
pub(crate) fn parse_one(event: &str, data: &str) -> ServerStreamEvent {
    let sse = format!("event: {event}\ndata: {data}\n\n");
    let mut evs = decode_all(sse.as_bytes());
    assert_eq!(evs.len(), 1, "expected exactly one event for {event}");
    evs.pop().unwrap()
}
pub(crate) fn parse_session(event: &str, data: &str) -> ServerStreamEvent {
    parse_one(event, data)
}
pub(crate) fn parse_response(event: &str, data: &str) -> ServerStreamEvent {
    parse_one(event, data)
}

/// A fresh empty state for `(conn_1, conv_1, ag)`.
pub(crate) fn fresh_state() -> crate::domain::SessionState {
    crate::domain::SessionState::new(
        crate::domain::ConnectionId::new("conn_1"),
        crate::domain::SessionId::new("conv_1"),
        crate::domain::AgentId::new("ag"),
    )
}

/// Build a `SessionSnapshot` fixture from JSON (public `Deserialize`, REVIEW#10).
pub(crate) fn snapshot_fixture(json: serde_json::Value) -> lens_client::sessions::SessionSnapshot {
    serde_json::from_value(json).expect("snapshot fixture must deserialize")
}
