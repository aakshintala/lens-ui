//! Wire `stream::Item` → domain `ItemKind`; dedup-by-id; `BlockContext` stamping.

use crate::clock::Clock;
use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind, StreamScratch};
use crate::domain::{CallId, ErrorSource, ItemId, Role, SessionState};
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
