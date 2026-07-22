//! Live terminal transport rider (Slice 1a): REST create/list/get/delete + WS
//! attach/input/output/resize against a real omnigent 0.5.1, plus the 4404
//! bogus-`tid` close classification (Spike-B live-confirmed).
//!
//! Creates its OWN throwaway session, so every write only touches a resource the
//! test itself owns. Skips-with-log if the server is unreachable rather than
//! failing the suite.
//!
//! Run: `LENS_OMNIGENT_URL=http://127.0.0.1:<port> LENS_OMNIGENT_AGENT_ID=<ag_…> \
//!   cargo test -p lens-client --features live-tests --test terminal_live -- --nocapture`
#![cfg(feature = "live-tests")]

use std::time::Duration;

use lens_client::ids::{ConnectionId, SessionId, TerminalId};
use lens_client::{
    AttachOptions, Auth, CloseCause, Connection, CreateSessionRequest, TerminalCreate, WsInbound,
    WsOutbound, attach,
};

fn client() -> lens_client::Client {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL");
    lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
        .expect("handshake")
}

/// Drain `inbound` up to `timeout`, returning true once a `Vt` frame contains
/// `needle`. Non-`Vt` frames (rare text, close) are logged and skipped.
fn wait_for_vt_containing(
    inbound: &crossbeam_channel::Receiver<WsInbound>,
    needle: &[u8],
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match inbound.recv_timeout(Duration::from_millis(250)) {
            Ok(WsInbound::Vt(bytes)) => {
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
            Ok(WsInbound::Text(t)) => eprintln!("live: inbound text {t:?}"),
            Ok(WsInbound::Closed(cause)) => {
                eprintln!("live: inbound closed {cause:?}");
                return false;
            }
            Err(_) => continue, // timeout tick; keep waiting until the deadline
        }
    }
    false
}

#[test]
fn terminal_crud_attach_input_output_resize_round_trips() {
    let agent_id = std::env::var("LENS_OMNIGENT_AGENT_ID").expect("set LENS_OMNIGENT_AGENT_ID");
    let client = client();
    let sessions = client.sessions();

    // Throwaway session this test owns.
    let snap = sessions
        .create(&CreateSessionRequest::new(agent_id.clone()))
        .expect("create session");
    let sid: SessionId = snap.id().clone();

    let terminals = client.terminals(sid.clone());

    // create — Spike-B-verified {terminal, session_key} body → typed resource.
    let created = terminals
        .create(&TerminalCreate {
            terminal: "shell".into(),
            session_key: "main".into(),
        })
        .expect("create terminal");
    let tid: TerminalId = created.id.clone();

    // list + get see it.
    assert!(
        terminals.list().expect("list").iter().any(|t| t.id == tid),
        "created terminal should appear in list"
    );
    let got = terminals.get(&tid).expect("get terminal");
    assert_eq!(got.id, tid);

    // attach (interactive) and prove input → echoed output.
    let handle = attach(&client, &sid, &tid, AttachOptions { read_only: false }).expect("attach");
    handle
        .outbound
        .send(WsOutbound::Input(b"printf 'MARKER_A\\n'\n".to_vec()))
        .expect("send input");
    assert!(
        wait_for_vt_containing(&handle.inbound, b"MARKER_A", Duration::from_secs(10)),
        "expected MARKER_A echoed back over the VT stream"
    );

    // resize is a text control frame; just prove it doesn't tear the connection.
    handle
        .outbound
        .send(WsOutbound::Resize {
            cols: 120,
            rows: 40,
        })
        .expect("send resize");
    std::thread::sleep(Duration::from_millis(500));
    handle.close();

    // delete → get is NotFound.
    terminals.delete(&tid).expect("delete terminal");
    match terminals.get(&tid) {
        Err(lens_client::ClientError::NotFound { .. }) => {}
        other => panic!("expected NotFound after delete, got {other:?}"),
    }

    let _ = sessions.delete(&sid, false);
}

#[test]
fn bogus_terminal_id_attach_classifies_4404() {
    let agent_id = std::env::var("LENS_OMNIGENT_AGENT_ID").expect("set LENS_OMNIGENT_AGENT_ID");
    let client = client();
    let sessions = client.sessions();
    let snap = sessions
        .create(&CreateSessionRequest::new(agent_id))
        .expect("create session");
    let sid: SessionId = snap.id().clone();

    // A 101 upgrade happens BEFORE the terminal lookup, so a bogus tid yields an
    // app-level close with code 4404 (Spike-B live-confirmed).
    let bogus = TerminalId::new("term_does_not_exist");
    let handle = attach(&client, &sid, &bogus, AttachOptions { read_only: false })
        .expect("attach upgrades before lookup");

    let mut saw_not_found = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match handle.inbound.recv_timeout(Duration::from_millis(250)) {
            Ok(WsInbound::Closed(CloseCause::TerminalNotFound)) => {
                saw_not_found = true;
                break;
            }
            Ok(other) => eprintln!("live: unexpected inbound {other:?}"),
            Err(_) => continue,
        }
    }
    assert!(
        saw_not_found,
        "bogus tid should close with 4404 TerminalNotFound"
    );

    handle.close();
    let _ = sessions.delete(&sid, false);
}
