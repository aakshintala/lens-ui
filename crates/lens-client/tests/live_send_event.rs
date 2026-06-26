//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL and an
//! existing session id in $LENS_OMNIGENT_SESSION_ID (a safe target; the test
//! sends an `Interrupt`, which is harmless on an idle session).
//! Run with: `LENS_OMNIGENT_URL=http://localhost:<port> LENS_OMNIGENT_SESSION_ID=<id> \
//!   cargo test -p lens-client --features live-tests --test live_send_event`
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn interrupt_event_round_trips() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL");
    let sid = std::env::var("LENS_OMNIGENT_SESSION_ID")
        .expect("set LENS_OMNIGENT_SESSION_ID to a live session id");

    let conn = Connection::new(ConnectionId::new("live"), base, Auth::None);
    let client = lens_client::Client::new(conn).expect("handshake");
    let ack = client
        .sessions()
        .send_event(&SessionId::new(sid), &SessionEventInput::Interrupt)
        .expect("send_event should return a typed ack");
    // A control event reports queued=false; the point is the ack parsed.
    let _ = ack.queued;
}
