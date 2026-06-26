//! Live lifecycle smoke: create -> patch -> delete a throwaway session end-to-end.
//! Requires $LENS_OMNIGENT_URL and $LENS_OMNIGENT_AGENT_ID (an existing agent id).
//! The session is created BY this test, so every write only touches a resource the
//! test itself owns — nothing pre-existing on the server is mutated.
//! Run: `LENS_OMNIGENT_URL=http://127.0.0.1:<port> LENS_OMNIGENT_AGENT_ID=<ag_…> \
//!   cargo test -p lens-client --features live-tests --test live_sessions_lifecycle`
#![cfg(feature = "live-tests")]

use lens_client::generated::UpdateSessionRequest;
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::{Auth, Connection, CreateSessionRequest};

fn client() -> lens_client::Client {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL");
    lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
        .expect("handshake")
}

#[test]
fn create_patch_delete_round_trips() {
    let agent_id = std::env::var("LENS_OMNIGENT_AGENT_ID").expect("set LENS_OMNIGENT_AGENT_ID");
    let client = client();
    let sessions = client.sessions();

    // create (JSON path) — returns a full snapshot
    let snap = sessions
        .create(&CreateSessionRequest::new(agent_id.clone()))
        .expect("create");
    let sid: SessionId = snap.id().clone();
    assert_eq!(snap.agent_id(), agent_id.as_str());
    assert!(!snap.archived(), "fresh session should not be archived");

    // patch — archive it; UpdateSessionRequest has no Default derive, so build it
    // from JSON (all fields are #[serde(default)]).
    let req: UpdateSessionRequest =
        serde_json::from_value(serde_json::json!({ "archived": true })).unwrap();
    let patched = sessions.patch(&sid, &req).expect("patch");
    assert!(patched.archived(), "patch should report archived=true");

    // delete — confirm the typed ConversationDeleted round-trips
    let deleted = sessions.delete(&sid, false).expect("delete");
    assert_eq!(deleted.id().as_str(), sid.as_str());
    assert!(deleted.deleted(), "delete should report deleted=true");
}
