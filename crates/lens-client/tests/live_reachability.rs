//! Live endpoint-reachability sweep (Plan 3c / typed-client §9): every consumed
//! read endpoint is pinged once through its typed method. Reachable = Ok or a
//! typed domain error; a transport error or a deserialize failure is a contract
//! problem and fails the test.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_reachability -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{GetOpts, ItemsPage, SessionFilter};
use lens_client::{Auth, Connection};

/// A typed Ok or a modeled domain error both prove reachability; only transport
/// / decode failures are fatal here.
fn reachable<T>(label: &str, r: lens_client::Result<T>) {
    match r {
        Ok(_) => eprintln!("  ok   {label}"),
        Err(e) if e.is_transport() => panic!("UNREACHABLE {label}: transport error: {e}"),
        Err(e) if e.is_decode() => panic!("CONTRACT DRIFT {label}: decode error: {e}"),
        Err(e) => eprintln!("  ok   {label} (typed domain error: {e})"),
    }
}

#[test]
fn consumed_read_endpoints_are_reachable() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );
    // Client::new exercises /health, /api/version, /v1/info on the ready ladder.
    let client =
        lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
            .unwrap();

    // Registries + info (impl Client; no session needed).
    reachable("/v1/me", client.me());
    reachable("/v1/agents", client.list_agents());
    reachable("/v1/hosts", client.list_hosts());
    reachable("/v1/policies", client.list_policies());

    // Session-scoped reads.
    let s = client.sessions();
    reachable("/v1/sessions", s.list(&SessionFilter::default()));
    reachable("/v1/sessions/{id}", s.get(&sid, GetOpts::default()));
    reachable(
        "/v1/sessions/{id}/items",
        s.items(&sid, &ItemsPage::default()),
    );
    reachable(
        "/v1/sessions/{id}/child_sessions",
        s.child_sessions(&sid, None, None),
    );
    reachable("/v1/sessions/{id}/resources", s.resources(&sid));
}
