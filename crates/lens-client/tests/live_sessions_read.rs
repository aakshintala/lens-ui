//! Live read tests. Require $LENS_OMNIGENT_URL and $LENS_OMNIGENT_SESSION_ID.
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::GetOpts;
use lens_client::{Auth, Connection};

fn client() -> lens_client::Client {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
        .expect("handshake")
}

#[test]
fn get_snapshot_parses() {
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("LENS_OMNIGENT_SESSION_ID"),
    );
    let snap = client()
        .sessions()
        .get(&sid, GetOpts::default())
        .expect("snapshot");
    assert_eq!(snap.id().as_str(), sid.as_str());
    let _ = snap.status();
}

#[test]
fn list_sessions_parses() {
    use lens_client::sessions::SessionFilter;
    let list = client()
        .sessions()
        .list(&SessionFilter::new().limit(5))
        .expect("list");
    // Envelope parsed; data may be empty on a fresh server.
    let _ = list.has_more;
}
