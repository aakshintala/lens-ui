//! T-0 live rider — replay real omnigent 0.5.1 SSE captures through the built
//! stack (lens-client `decode_all` → lens-core `reduce`) and assert authoritative
//! turn-identity invariants end-to-end.
//!
//! Captures: `docs/spikes/captures/2026-07-21-t0-verify/` (live bytes, 2026-07-21).

use lens_client::stream::{ResponseEvent, ServerStreamEvent, decode_all};
use lens_core::{
    AgentId, ConnectionId, ManualClock, ResponseId, SessionId, SessionState, StreamUpdate, reduce,
};

/// Full wire ids read from the capture bytes (not truncated).
const TURN2_RESPONSE_ID: &str = "resp_bcb93365f7aa4a0c9177e142";
const INTERRUPT_TURN_A_ID: &str = "resp_0099878eae564c86aaa21a63";
const INTERRUPT_TURN_B_ID: &str = "resp_37ba30e3a06240e4bc1de44a";

fn fresh_state() -> SessionState {
    SessionState::new(
        ConnectionId::new("conn_1"),
        SessionId::new("conv_599b6d156fd44a8886c200d9d55c7758"),
        AgentId::new("ag_97c6733206ee41b9a864cd5e003dfb28"),
    )
}

fn load_capture(rel: &str) -> Vec<ServerStreamEvent> {
    let path = format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"),);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read capture {path}: {e}"));
    decode_all(&bytes)
}

fn replay(state: &mut SessionState, events: &[ServerStreamEvent], clock: &ManualClock) {
    for ev in events {
        reduce(state, ev, clock);
    }
}

fn wire_response_id_from_output_item_done(ev: &ServerStreamEvent) -> Option<&str> {
    match ev {
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) => {
            item.response_id().filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

fn in_progress_response_id(ev: &ServerStreamEvent) -> Option<&str> {
    match ev {
        ServerStreamEvent::Response(ResponseEvent::InProgress { response_id }) => {
            response_id.as_deref().filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

#[test]
fn turn2_stamps_items_and_tracks_active_response_mid_turn() {
    let events = load_capture("docs/spikes/captures/2026-07-21-t0-verify/turn2.stream.sse");
    let clock = ManualClock::new(1_700_000_000_000);

    // Mid-turn: after `response.in_progress`, before terminal `response.completed`.
    let mut mid = fresh_state();
    let mut saw_in_progress = false;
    for ev in &events {
        reduce(&mut mid, ev, &clock);
        if let Some(id) = in_progress_response_id(ev) {
            assert_eq!(id, TURN2_RESPONSE_ID);
            assert_eq!(
                mid.active_response.as_ref().map(ResponseId::as_str),
                Some(TURN2_RESPONSE_ID),
                "active_response must mirror response.in_progress mid-turn"
            );
            saw_in_progress = true;
        }
        if matches!(ev, ServerStreamEvent::Response(ResponseEvent::Completed)) {
            break;
        }
    }
    assert!(saw_in_progress, "capture must contain response.in_progress");

    // Full replay: items carry their own wire response_id.
    let wire_id = events
        .iter()
        .find_map(wire_response_id_from_output_item_done)
        .expect("capture must contain output_item.done with response_id");
    assert_eq!(wire_id, TURN2_RESPONSE_ID);

    let mut state = fresh_state();
    replay(&mut state, &events, &clock);
    assert!(
        state.items.iter().any(|item| {
            item.ctx
                .response_id
                .as_ref()
                .is_some_and(|r| r.as_str() == wire_id)
        }),
        "at least one transcript item must be stamped with the wire response_id"
    );
    assert_eq!(
        state.active_response, None,
        "terminal response.completed clears active"
    );
}

#[test]
fn interrupt_then_retry_mints_distinct_response_ids() {
    let events =
        load_capture("docs/spikes/captures/2026-07-21-t0-verify/interrupt-then-retry.stream.sse");
    let clock = ManualClock::new(1_700_000_000_000);
    let mut state = fresh_state();

    assert_ne!(INTERRUPT_TURN_A_ID, INTERRUPT_TURN_B_ID);

    let mut active_after_a_in_progress = false;
    let mut cleared_on_cancelled = false;
    let mut active_after_b_in_progress = false;
    let mut seen_ids: Vec<String> = Vec::new();

    for ev in &events {
        reduce(&mut state, ev, &clock);

        if let Some(id) = in_progress_response_id(ev) {
            seen_ids.push(id.to_owned());
            if id == INTERRUPT_TURN_A_ID {
                assert_eq!(
                    state.active_response.as_ref().map(ResponseId::as_str),
                    Some(INTERRUPT_TURN_A_ID)
                );
                active_after_a_in_progress = true;
            }
            if id == INTERRUPT_TURN_B_ID {
                assert_eq!(
                    state.active_response.as_ref().map(ResponseId::as_str),
                    Some(INTERRUPT_TURN_B_ID)
                );
                active_after_b_in_progress = true;
            }
        }

        if matches!(ev, ServerStreamEvent::Response(ResponseEvent::Cancelled)) {
            assert_eq!(
                state.active_response, None,
                "response.cancelled must clear active_response"
            );
            cleared_on_cancelled = true;
        }
    }

    assert!(active_after_a_in_progress, "turn A in_progress seen");
    assert!(cleared_on_cancelled, "response.cancelled seen");
    assert!(active_after_b_in_progress, "turn B in_progress seen");

    // Distinct ids across the two turns; no collision in observed in_progress sequence.
    let mut deduped = seen_ids.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        2,
        "expected exactly two distinct in_progress ids"
    );
    assert!(deduped.contains(&INTERRUPT_TURN_A_ID.to_owned()));
    assert!(deduped.contains(&INTERRUPT_TURN_B_ID.to_owned()));

    // Items stamped per-turn with their own wire ids.
    let turn_b_items: Vec<_> = state
        .items
        .iter()
        .filter(|i| {
            i.ctx
                .response_id
                .as_ref()
                .is_some_and(|r| r.as_str() == INTERRUPT_TURN_B_ID)
        })
        .collect();
    assert!(
        !turn_b_items.is_empty(),
        "turn B must produce at least one item stamped with its response_id"
    );
    // Turn A was interrupted before any output_item.done — no resp_-stamped items for A.
    assert!(
        state.items.iter().all(|i| {
            i.ctx
                .response_id
                .as_ref()
                .is_none_or(|r| r.as_str() != INTERRUPT_TURN_A_ID)
        }),
        "turn A response_id must not appear on transcript items after interrupt"
    );

    assert_eq!(
        state.active_response, None,
        "terminal completed clears active"
    );
    assert!(
        state.items.iter().any(|i| i
            .ctx
            .response_id
            .as_ref()
            .is_some_and(|r| r.as_str() == INTERRUPT_TURN_B_ID)),
        "kiwi reply must carry turn B response_id"
    );

    // Delta path: cancelled emitted ActiveResponseChanged(None); B in_progress emitted Some.
    let mut deltas: Vec<Option<String>> = Vec::new();
    let mut probe = fresh_state();
    for ev in &events {
        let updates = reduce(&mut probe, ev, &clock);
        for u in updates {
            if let StreamUpdate::ActiveResponseChanged(r) = u {
                deltas.push(r.map(|id| id.as_str().to_owned()));
            }
        }
    }
    assert!(
        deltas
            .iter()
            .any(|d| d.as_deref() == Some(INTERRUPT_TURN_A_ID)),
        "ActiveResponseChanged(Some(A)) emitted"
    );
    assert!(
        deltas.iter().any(|d| d.is_none()),
        "ActiveResponseChanged(None) emitted on cancel/complete"
    );
    assert!(
        deltas
            .iter()
            .any(|d| d.as_deref() == Some(INTERRUPT_TURN_B_ID)),
        "ActiveResponseChanged(Some(B)) emitted"
    );
}
