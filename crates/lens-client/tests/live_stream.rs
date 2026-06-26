//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL and an
//! idle, runner-backed session id in $LENS_OMNIGENT_SESSION_ID (claude-sdk).
//! Subscribe-first: opens the stream, posts a message, asserts typed events flow.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_stream -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::stream::{ResponseEvent, ServerStreamEvent};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn live_stream_yields_typed_events() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );
    let client =
        lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
            .unwrap();

    // Subscribe FIRST (no-replay), then drive a turn.
    let stream = client.sessions().stream(&sid).expect("open stream");
    client
        .sessions()
        .send_event(
            &sid,
            &SessionEventInput::Message {
                content: vec![
                    serde_json::json!({"type":"input_text","text":"Say hello in one word."}),
                ],
                model_override: None,
                tools: None,
            },
        )
        .expect("post message");

    // Drain until a terminal response event or a timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut saw_completed = false;
    let mut saw_unknown: Vec<String> = Vec::new();
    while std::time::Instant::now() < deadline {
        match stream.recv() {
            Some(ServerStreamEvent::Response(ResponseEvent::Completed)) => {
                saw_completed = true;
                break;
            }
            Some(ServerStreamEvent::Unknown { event_type }) => saw_unknown.push(event_type),
            Some(_) => {}
            None => break,
        }
    }
    assert!(saw_completed, "never observed response.completed");
    // Surface (do not hard-fail) any unmodeled live events — feeds Plan 3c drift.
    if !saw_unknown.is_empty() {
        eprintln!("UNMODELED live events (model these / Plan 3c): {saw_unknown:?}");
    }
}
