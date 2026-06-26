//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL.
//! Run with: `LENS_OMNIGENT_URL=http://localhost:<port> cargo test -p lens-client --features live-tests --test live_handshake`
#![cfg(feature = "live-tests")]

use lens_client::ids::ConnectionId;
use lens_client::{Auth, Connection};

fn base_url() -> url::Url {
    std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL to the running server (omnigent server status)")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL")
}

#[test]
fn handshake_succeeds_against_pinned_server() {
    let conn = Connection::new(ConnectionId::new("live"), base_url(), Auth::None);
    let client = lens_client::Client::new(conn).expect("handshake should pass the contract gate");
    // /v1/info is reachable and parsed; accounts_enabled is a real bool either way.
    let _ = client.server_info().accounts_enabled;
}
