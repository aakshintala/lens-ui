//! §3 pure view-projection: canonical `&[Item]` (+ scratch + active_response) → `Vec<ViewBlock>`.
//! Borrow-only; no clones in the block tree; no `pending`; no gpui (T-1 spec).

use crate::domain::ids::{CallId, ResponseId};
use crate::domain::item::{Item, ItemKind, MessageAcc, ReasoningAcc, StreamScratch};

/// A projected render unit. Borrows the whole `&[Item]` slice / `&StreamScratch` for the frame.
#[derive(Debug)]
pub enum ViewBlock<'a> {
    /// Passthrough (incl. Compaction/AgentChanged/Error/ResourceEvent markers — render matches ItemKind).
    Item(&'a Item),
    /// FunctionCall paired with its FunctionCallOutput by call_id. Takes the call's stream position.
    ToolSpan {
        call: &'a Item,
        output: Option<&'a Item>,
    },
    /// One response's folded work, keyed by the shared authoritative response_id (T-6 attaches meta/expansion).
    WorkSection {
        response_id: &'a ResponseId,
        blocks: Vec<ViewBlock<'a>>,
    },
    /// Live in-flight reasoning tail (scratch.open_reasoning).
    StreamingReasoning(&'a ReasoningAcc),
    /// Live in-flight message tail (scratch.open_message).
    StreamingMessage(&'a MessageAcc),
}

/// Stage 2 helper: pair `FunctionCall` with its `FunctionCallOutput` by `call_id`.
/// The `ToolSpan` takes the call's position; the consumed output is removed from the flat stream.
/// Orphan outputs and duplicate/re-used outputs pass through as `Item` (never dropped, never merged).
pub fn pair_tool_spans<'a>(items: &[&'a Item]) -> Vec<ViewBlock<'a>> {
    use std::collections::{HashMap, HashSet};

    let mut calls: HashSet<&CallId> = HashSet::new();
    let mut first_output: HashMap<&CallId, &'a Item> = HashMap::new();
    for it in items {
        match &it.kind {
            ItemKind::FunctionCall { call_id, .. } => {
                calls.insert(call_id);
            }
            ItemKind::FunctionCallOutput { call_id, .. } => {
                first_output.entry(call_id).or_insert(it);
            }
            _ => {}
        }
    }
    // Output items that a call will consume: the FIRST output of a call_id that has a
    // matching FunctionCall in the window. Keyed by item id (identity). Order-independent.
    let mut consumed: HashSet<&str> = HashSet::new();
    for (call_id, out_item) in &first_output {
        if calls.contains(*call_id) {
            consumed.insert(out_item.id.as_str());
        }
    }

    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match &it.kind {
            ItemKind::FunctionCall { call_id, .. } => {
                out.push(ViewBlock::ToolSpan {
                    call: it,
                    output: first_output.get(call_id).copied(),
                });
            }
            ItemKind::FunctionCallOutput { .. } => {
                // Orphan (no call) or duplicate/second output → passthrough; consumed → skip.
                if !consumed.contains(it.id.as_str()) {
                    out.push(ViewBlock::Item(it));
                }
            }
            _ => out.push(ViewBlock::Item(it)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{CallId, ItemId};
    use crate::domain::item::{BlockContext, ContentBlock};
    use crate::domain::scalars::Role;
    use serde_json::Value;

    fn ctx_with(resp: Option<&str>) -> BlockContext {
        BlockContext {
            agent: None,
            depth: 0,
            response_id: resp.map(ResponseId::new),
        }
    }

    fn item(id: &str, resp: Option<&str>, kind: ItemKind) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(resp),
            created_at: 0,
            kind,
        }
    }

    fn call(id: &str, resp: Option<&str>, call_id: &str, status: &str) -> Item {
        item(
            id,
            resp,
            ItemKind::FunctionCall {
                call_id: CallId::new(call_id),
                name: "read".into(),
                arguments: Value::Null,
                status: status.into(),
                agent_name: None,
            },
        )
    }

    fn output(id: &str, resp: Option<&str>, call_id: &str) -> Item {
        item(
            id,
            resp,
            ItemKind::FunctionCallOutput {
                call_id: CallId::new(call_id),
                output: "ok".into(),
                arguments: Value::Null,
            },
        )
    }

    fn msg(id: &str, resp: Option<&str>, role: Role, text: &str) -> Item {
        item(
            id,
            resp,
            ItemKind::Message {
                role,
                content: vec![ContentBlock {
                    kind: "output_text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        )
    }

    // Assert helper: a ViewBlock is a ToolSpan over the given call/output item ids.
    fn assert_span(vb: &ViewBlock, call_id: &str, output_id: Option<&str>) {
        match vb {
            ViewBlock::ToolSpan { call, output } => {
                assert_eq!(call.id.as_str(), call_id);
                assert_eq!(output.map(|o| o.id.as_str()), output_id);
            }
            other => panic!("expected ToolSpan, got {other:?}"),
        }
    }

    fn assert_item(vb: &ViewBlock, id: &str) {
        match vb {
            ViewBlock::Item(i) => assert_eq!(i.id.as_str(), id),
            other => panic!("expected Item({id}), got {other:?}"),
        }
    }

    #[test]
    fn pairs_call_with_following_output() {
        let items = vec![call("c1", None, "call_1", "completed"), output("o1", None, "call_1")];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_span(&out[0], "c1", Some("o1"));
    }

    #[test]
    fn non_completed_call_without_output_is_span_with_none() {
        let items = vec![call("c1", None, "call_1", "in_progress")];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_span(&out[0], "c1", None);
    }

    #[test]
    fn orphan_output_passes_through() {
        let items = vec![output("o1", None, "call_missing")];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_item(&out[0], "o1");
    }

    #[test]
    fn output_before_call_still_pairs_at_call_position() {
        // Output precedes its call in stream order; span takes the call's position.
        let items = vec![
            output("o1", None, "call_1"),
            msg("m1", None, Role::User, "hi"),
            call("c1", None, "call_1", "completed"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 2);
        assert_item(&out[0], "m1");
        assert_span(&out[1], "c1", Some("o1"));
    }

    #[test]
    fn parallel_interleaved_calls_pair_by_call_id() {
        let items = vec![
            call("c1", None, "call_1", "completed"),
            call("c2", None, "call_2", "completed"),
            output("o2", None, "call_2"),
            output("o1", None, "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 2);
        assert_span(&out[0], "c1", Some("o1"));
        assert_span(&out[1], "c2", Some("o2"));
    }

    #[test]
    fn duplicate_output_for_same_call_id_passes_through() {
        // First output wins; a second output for the same call_id is a passthrough Item, not merged.
        let items = vec![
            call("c1", None, "call_1", "completed"),
            output("o1", None, "call_1"),
            output("o1b", None, "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 2);
        assert_span(&out[0], "c1", Some("o1"));
        assert_item(&out[1], "o1b");
    }
}
