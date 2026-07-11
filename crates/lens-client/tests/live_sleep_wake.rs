//! Live D17 verify — requires a running omnigent server at `$LENS_OMNIGENT_URL`
//! and an IDLE, item-bearing session in `$LENS_OMNIGENT_SESSION_ID` (≥2 items).
//!
//! Run:
//! ```text
//! LENS_OMNIGENT_URL=http://localhost:<port> LENS_OMNIGENT_SESSION_ID=<id> \
//!   cargo test -p lens-client --features live-tests --test live_sleep_wake
//! ```
//!
//! Asserts the D17 durable-refetch contract: after `StopSession` (the sleep
//! mechanism), the session transcript is durably re-fetchable via forward
//! `/items` catch-up (`order=asc`). Also verifies the exclusive `after` cursor
//! (`after=<first_id>` returns the strict suffix excluding the cursor item —
//! the property the actor's forward catch-up relies on).
//!
//! `StopSession` leaves the server session status = `"failed"` (runner torn
//! down); that is expected and irrelevant to transcript durability.
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{ItemList, ItemsPage};
use lens_client::{Auth, Connection, SessionEventInput};

fn item_ids(list: &ItemList) -> Vec<&str> {
    list.items().iter().map(|i| i.id()).collect()
}

#[test]
fn stop_session_transcript_durably_refetchable_via_forward_items() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL");
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );

    let conn = Connection::new(ConnectionId::new("live"), base, Auth::None);
    let client = lens_client::Client::new(conn).expect("handshake");
    let sessions = client.sessions();

    let page_asc = ItemsPage {
        limit: Some(50),
        after: None,
        before: None,
        order: Some("asc".into()),
    };

    // Step 1: capture the ordered transcript before stop.
    let pre_stop = sessions
        .items(&sid, &page_asc)
        .expect("pre-stop forward items");
    let pre_ids = item_ids(&pre_stop);
    assert!(
        pre_ids.len() >= 2,
        "need ≥2 items for D17; got {}",
        pre_ids.len()
    );

    // Step 2: exclusive `after` cursor — strict suffix excluding the cursor item.
    let first_id = pre_ids[0].to_string();
    let suffix_page = ItemsPage {
        after: Some(first_id),
        order: Some("asc".into()),
        limit: Some(50),
        before: None,
    };
    let suffix = sessions
        .items(&sid, &suffix_page)
        .expect("exclusive after-cursor items");
    let suffix_ids = item_ids(&suffix);
    assert_eq!(
        suffix_ids.len(),
        pre_ids.len() - 1,
        "after=<first_id> must be exclusive (N-1 items)"
    );
    assert_eq!(
        suffix_ids,
        pre_ids[1..],
        "after cursor must return the strict suffix excluding the cursor item"
    );

    // Step 3: StopSession (sleep mechanism). Control events report queued=false.
    let ack = sessions
        .send_event(&sid, &SessionEventInput::StopSession)
        .expect("StopSession should return a typed ack");
    let _ = ack.queued;

    // Step 4: post-stop transcript must be identical and durably re-fetchable.
    let post_stop = sessions
        .items(&sid, &page_asc)
        .expect("post-stop forward items");
    let post_ids = item_ids(&post_stop);
    assert_eq!(
        post_ids, pre_ids,
        "post-stop transcript must match pre-stop ordered ids (D17 durable refetch)"
    );
}
