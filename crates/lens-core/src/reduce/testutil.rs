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

/// Representative golden-corpus captures for the P1 determinism gate (Task 11).
pub(crate) const CORPUS_FILES: &[&str] = &[
    "docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse",
    "docs/spikes/captures/2026-06-26-sse/interrupt.stream.sse",
    "docs/spikes/captures/2026-06-26-sse/reasoning_effort_high.stream.sse",
    "docs/spikes/captures/2026-06-26-live-recapture/agent-switched.sse",
    "docs/spikes/captures/2026-06-26-live-recapture/claude-native-todos.sse",
    "docs/spikes/captures/2026-06-26-live-recapture/polly-child-session.sse",
    "docs/spikes/captures/2026-06-26-live-recapture/compaction.sse",
    "docs/spikes/captures/2026-06-26-live-recapture/cursor-sdk-reasoning.sse",
];

/// Decode a repo-relative corpus path into typed events via the `decode_all` seam.
pub(crate) fn decode_corpus(rel_from_repo_root: &str) -> Vec<ServerStreamEvent> {
    let path = format!(
        "{}/{}",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),
        rel_from_repo_root
    );
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read corpus {path}: {e}"));
    decode_all(&bytes)
}
