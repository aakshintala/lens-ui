//! Live taxonomy-diff (Plan 3c / §9.4 runtime half): drive one turn and assert
//! every observed wire `Unknown` event type is at least *declared* by the pinned
//! contract (∈ ACCOUNTED_EVENT_TYPES). A wire type absent from the contract is
//! drift the openapi doesn't express — fail loud.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_taxonomy -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::stream::{ACCOUNTED_EVENT_TYPES, ResponseEvent, ServerStreamEvent};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn live_wire_types_are_declared_by_contract() {
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

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut undeclared: Vec<String> = Vec::new();
    let mut saw_completed = false;
    while std::time::Instant::now() < deadline {
        match stream.try_recv() {
            Some(ServerStreamEvent::Response(ResponseEvent::Completed)) => {
                saw_completed = true;
                break;
            }
            Some(ServerStreamEvent::Unknown { event_type }) => {
                if !ACCOUNTED_EVENT_TYPES.contains(&event_type.as_str()) {
                    undeclared.push(event_type);
                }
            }
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    assert!(saw_completed, "never observed response.completed");
    assert!(
        undeclared.is_empty(),
        "wire event types absent from the pinned contract (re-vendor + model): {undeclared:?}"
    );
}
