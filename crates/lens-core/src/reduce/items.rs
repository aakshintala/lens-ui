//! Wire `stream::Item` → domain `ItemKind`; dedup-by-id; `BlockContext` stamping.

use crate::clock::Clock;
use crate::domain::item::ReasoningAcc;
use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind, StreamScratch};
use crate::domain::{AgentId, CallId, ErrorSource, ItemId, Role, SessionState};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::Item as WireItem;
use serde_json::Value;
use smallvec::smallvec;

pub(crate) fn current_ctx(scratch: &StreamScratch) -> BlockContext {
    BlockContext {
        agent: scratch.current_agent.clone(),
        depth: 0, // P1-DECISION D-P1-14: sub-agent depth deferred to §9
        turn: scratch.turn,
    }
}

fn role_of(role: &str) -> Role {
    match role {
        "user" => Role::User,
        _ => Role::Assistant,
    }
}

fn parse_args(raw: &str) -> Value {
    // D-P1-6: wire `arguments` is a raw JSON string; the state model owns parsing.
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// REVIEW#3: returns `None` for wire items that produce NO transcript item (resources —
/// D-P1-4). The `OutputItemDone` routing turns `None` into a `ResourcesChanged` marker.
pub(crate) fn map_item(wire: &WireItem) -> Option<(ItemId, ItemKind)> {
    let id = ItemId::new(wire.id().to_string());
    let kind = match wire {
        WireItem::Message { role, content, .. } => ItemKind::Message {
            role: role_of(role),
            content: content
                .iter()
                .map(|b| ContentBlock {
                    kind: b.block_type().to_string(),
                    text: b.text().map(str::to_string),
                    data: Value::Null,
                })
                .collect(),
        },
        WireItem::FunctionCall {
            call_id,
            name,
            arguments,
            status,
            agent,
            ..
        } => ItemKind::FunctionCall {
            call_id: CallId::new(call_id.clone()),
            name: name.clone(),
            arguments: parse_args(arguments),
            status: status.clone(),
            agent_name: agent.clone().filter(|_| status == "completed"), // D-P1-6
        },
        WireItem::FunctionCallOutput {
            call_id, output, ..
        } => ItemKind::FunctionCallOutput {
            call_id: CallId::new(call_id.clone()),
            output: output.clone(),
            arguments: Value::Null, // D-P1-7: paired at render, not back-filled
        },
        WireItem::Error {
            source,
            code,
            message,
            ..
        } => ItemKind::Error {
            source: source
                .as_deref()
                .map(map_error_source)
                .unwrap_or(ErrorSource::Unknown),
            code: code.clone().unwrap_or_default(),
            message: message.clone().unwrap_or_default(),
        },
        // D-P1-4: resources are NOT materialized as items in P1 (no SessionResourceObject
        // available from the wire) → None ⇒ ResourcesChanged marker.
        WireItem::ResourceEvent { .. } => return None,
        // D-P1-3 catch-all: native tools / unmodeled wire items keep a transcript slot;
        // full payload deferred until lens-client widens `stream::Item`.
        WireItem::Other { item_type, .. } => ItemKind::NativeTool {
            tool_type: item_type.clone(),
            data: Value::Null,
        },
    };
    Some((id, kind))
}

fn map_error_source(s: &str) -> ErrorSource {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(ErrorSource::Unknown)
}

/// Deterministic, collision-free local id for reducer-synthesized items (REVIEW#2).
fn local_id(kind: &str, state: &SessionState) -> ItemId {
    let mut n = state.items.len();
    loop {
        let candidate = ItemId::new(format!("{kind}_local_{n}"));
        if !state.items.iter().any(|it| it.id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

pub(crate) fn finalize_message(state: &mut SessionState, clock: &dyn Clock) -> Updates {
    let Some(acc) = state.stream.open_message.take() else {
        return smallvec![];
    };
    let id = acc
        .message_id
        .clone()
        .map(ItemId::new)
        .unwrap_or_else(|| local_id("msg", state));
    let kind = ItemKind::Message {
        role: Role::Assistant,
        content: vec![ContentBlock {
            kind: "output_text".into(),
            text: Some(acc.text),
            data: Value::Null,
        }],
    };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn finalize_reasoning(state: &mut SessionState, clock: &dyn Clock) -> Updates {
    let Some(acc): Option<ReasoningAcc> = state.stream.open_reasoning.take() else {
        return smallvec![];
    };
    let id = local_id("reasoning", state);
    let kind = ItemKind::Reasoning {
        full_text: acc.full_text,
        summary_text: acc.summary_text,
        encrypted: acc.encrypted,
    };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn push_compaction(
    state: &mut SessionState,
    total_tokens: Option<i64>,
    clock: &dyn Clock,
) -> Updates {
    let id = local_id("compaction", state);
    let kind = ItemKind::Compaction {
        summary: String::new(),
        token_count: total_tokens.map(|t| t.max(0) as u64),
    };
    push_item(state, id, kind, None, clock)
}

pub(crate) fn push_agent_changed(
    state: &mut SessionState,
    from: AgentId,
    to: AgentId,
    clock: &dyn Clock,
) -> Updates {
    let at = clock.now_millis();
    let id = local_id("agent_changed", state);
    push_item(
        state,
        id,
        ItemKind::AgentChanged { from, to, at },
        None,
        clock,
    )
}

/// Dedup-by-id insert (D-P1-13). Present ⇒ update in place; absent ⇒ append.
pub(crate) fn push_item(
    state: &mut SessionState,
    id: ItemId,
    kind: ItemKind,
    seq: Option<u64>,
    clock: &dyn Clock,
) -> Updates {
    let ctx = current_ctx(&state.stream);
    if let Some(idx) = state.items.iter().position(|it| it.id == id) {
        let existing = &mut state.items[idx];
        existing.kind = kind;
        existing.seq = seq.or(existing.seq);
        smallvec![StreamUpdate::ItemUpdated { index: idx }]
    } else {
        state.items.push(Item {
            id,
            seq,
            ctx,
            created_at: clock.now_millis(),
            kind,
        });
        smallvec![StreamUpdate::ItemAppended {
            index: state.items.len() - 1,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState};
    use crate::reduce::testutil::parse_response;
    use crate::reduce::{StreamUpdate, reduce};

    fn st() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }
    fn clock() -> ManualClock {
        ManualClock::new(1_700_000_000_000)
    }

    #[test]
    fn function_call_parses_arguments_and_sanitizes_agent() {
        let mut s = st();
        let ev = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{\"path\":\"a.rs\"}","call_id":"toolu_1","agent":"coder"}}"#,
        );
        let u = reduce(&mut s, &ev, &clock());
        assert_eq!(s.items.len(), 1);
        match &s.items[0].kind {
            ItemKind::FunctionCall {
                call_id,
                arguments,
                agent_name,
                status,
                ..
            } => {
                assert_eq!(call_id.as_str(), "toolu_1");
                assert_eq!(arguments["path"], "a.rs"); // parsed to Value
                assert_eq!(agent_name.as_deref(), Some("coder")); // completed ⇒ name kept
                assert_eq!(status, "completed");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(&u[..], &[StreamUpdate::ItemAppended { index: 0 }]);
        assert_eq!(s.items[0].created_at, 1_700_000_000_000); // clock-stamped
    }

    #[test]
    fn in_progress_function_call_drops_resp_id_agent() {
        let mut s = st();
        let ev = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_2","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"c","agent":"resp_abc"}}"#,
        );
        reduce(&mut s, &ev, &clock());
        match &s.items[0].kind {
            ItemKind::FunctionCall { agent_name, .. } => assert_eq!(*agent_name, None), // D-P1-6
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn duplicate_id_updates_in_place() {
        let mut s = st();
        let first = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"c"}}"#,
        );
        reduce(&mut s, &first, &clock());
        let second = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{}","call_id":"c"}}"#,
        );
        let u = reduce(&mut s, &second, &clock());
        assert_eq!(s.items.len(), 1); // no double-insert (D-P1-13)
        assert_eq!(&u[..], &[StreamUpdate::ItemUpdated { index: 0 }]);
    }

    use lens_client::stream::{ResponseEvent, ServerStreamEvent};

    fn resp_text(delta: &str, message_id: Option<&str>, index: Option<usize>) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
            delta: delta.into(),
            message_id: message_id.map(str::to_string),
            index,
            last: None,
        })
    }

    #[test]
    fn completed_bumps_turn_and_finalizes_unpersisted_message() {
        let mut s = st();
        reduce(&mut s, &resp_text("hi", None, None), &clock());
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clock(),
        );
        assert_eq!(s.stream.turn, 1);
        assert_eq!(s.items.len(), 1);
        assert!(matches!(s.items[0].kind, ItemKind::Message { .. }));
        assert_eq!(s.items[0].ctx.turn, 0); // REVIEW#1: stamped with the PRE-bump turn
        assert!(s.stream.open_message.is_none());
    }

    #[test]
    fn synthetic_ids_are_unique_across_same_clock_finalizes() {
        let mut s = st();
        let clk = clock();
        reduce(&mut s, &resp_text("first", None, None), &clk);
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clk,
        );
        reduce(&mut s, &resp_text("second", None, None), &clk);
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clk,
        );
        assert_eq!(
            s.items.len(),
            2,
            "same-clock synthetic ids collided → dedup ate one"
        );
        assert_ne!(s.items[0].id, s.items[1].id);
    }

    #[test]
    fn output_item_done_unrelated_keyed_message_preserves_open_preview() {
        let mut s = st();
        reduce(
            &mut s,
            &resp_text("streaming…", Some("msg_A"), None),
            &clock(),
        );
        let done_other = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"msg_B","type":"message","role":"assistant","content":[{"type":"output_text","text":"other"}]}}"#,
        );
        reduce(&mut s, &done_other, &clock());
        assert!(
            s.stream.open_message.is_some(),
            "unrelated msg_B must not clear the msg_A preview"
        );
    }

    #[test]
    fn completed_does_not_double_insert_when_output_item_done_won() {
        let mut s = st();
        reduce(&mut s, &resp_text("hi", None, None), &clock());
        let done = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}"#,
        );
        reduce(&mut s, &done, &clock());
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clock(),
        );
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn reasoning_closed_finalizes_item_from_scratch() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
            &clock(),
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
                delta: "why".into(),
            }),
            &clock(),
        );
        let closed = ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
            full_text: "why".into(),
            summary_text: "".into(),
        });
        reduce(&mut s, &closed, &clock());
        assert!(s.stream.open_reasoning.is_none());
        assert!(matches!(
            &s.items[0].kind,
            ItemKind::Reasoning { full_text, .. } if full_text == "why"
        ));
    }

    #[test]
    fn compaction_completed_pushes_item() {
        let mut s = st();
        let ev = ServerStreamEvent::Response(ResponseEvent::CompactionCompleted {
            total_tokens: Some(8421),
        });
        reduce(&mut s, &ev, &clock());
        assert!(matches!(
            s.items[0].kind,
            ItemKind::Compaction {
                token_count: Some(8421),
                ..
            }
        ));
    }

    #[test]
    fn agent_changed_pushes_transcript_marker_with_synthesized_from() {
        use lens_client::stream::SessionEvent;
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::AgentChanged {
                agent_id: "ag_2".into(),
                agent_name: "debby".into(),
            }),
            &clock(),
        );
        let marker = s.items.iter().find_map(|it| match &it.kind {
            ItemKind::AgentChanged { from, to, .. } => {
                Some((from.as_str().to_string(), to.as_str().to_string()))
            }
            _ => None,
        });
        assert_eq!(marker, Some(("ag".into(), "ag_2".into())));
        assert!(u.contains(&StreamUpdate::AgentChanged));
    }

    #[test]
    fn local_id_probes_past_existing_collision() {
        let mut s = st();
        let clk = clock();
        // Seed a real item whose id would collide with the first synthesized reasoning id.
        let seeded = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"reasoning_local_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"real"}]}}"#,
        );
        reduce(&mut s, &seeded, &clk);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].id.as_str(), "reasoning_local_1");

        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
            &clk,
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
                delta: "synth".into(),
            }),
            &clk,
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                full_text: "synth".into(),
                summary_text: "".into(),
            }),
            &clk,
        );
        assert_eq!(
            s.items.len(),
            2,
            "synthesized reasoning must append, not overwrite"
        );
        assert_eq!(s.items[0].id.as_str(), "reasoning_local_1");
        assert_eq!(s.items[1].id.as_str(), "reasoning_local_2");
    }

    #[test]
    fn output_item_done_clears_preview_emits_scratch_changed() {
        let mut s = st();
        reduce(&mut s, &resp_text("hi", None, None), &clock());
        let done = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}"#,
        );
        let u = reduce(&mut s, &done, &clock());
        assert!(u.contains(&StreamUpdate::ScratchChanged));
        assert!(s.stream.open_message.is_none());
    }

    #[test]
    fn completed_clears_preview_emits_scratch_changed() {
        let mut s = st();
        reduce(&mut s, &resp_text("hi", None, None), &clock());
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::Completed),
            &clock(),
        );
        assert!(u.contains(&StreamUpdate::ScratchChanged));
        assert!(s.stream.open_message.is_none());
    }

    #[test]
    fn reasoning_closed_emits_scratch_changed() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
            &clock(),
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
                delta: "why".into(),
            }),
            &clock(),
        );
        let closed = ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
            full_text: "why".into(),
            summary_text: "".into(),
        });
        let u = reduce(&mut s, &closed, &clock());
        assert!(u.contains(&StreamUpdate::ScratchChanged));
    }

    #[test]
    fn unmodeled_item_maps_to_native_tool_catchall() {
        let mut s = st();
        let ev = parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"x_9","type":"native_tool","kind":"web_search_call"}}"#,
        );
        reduce(&mut s, &ev, &clock());
        match &s.items[0].kind {
            ItemKind::NativeTool { tool_type, .. } => assert_eq!(tool_type, "native_tool"), // D-P1-3
            other => panic!("{other:?}"),
        }
    }
}
