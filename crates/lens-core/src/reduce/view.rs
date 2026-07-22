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
    /// One contiguous agent-work run, keyed by `(response_id, run_index)` (T-6 attaches meta/expansion).
    WorkSection {
        response_id: &'a ResponseId,
        run_index: u32,
        blocks: Vec<ViewBlock<'a>>,
    },
    /// Live in-flight reasoning tail (scratch.open_reasoning).
    StreamingReasoning {
        response_id: Option<&'a ResponseId>,
        acc: &'a ReasoningAcc,
    },
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

    let mut paired: HashSet<&CallId> = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match &it.kind {
            ItemKind::FunctionCall { call_id, .. } => {
                // First call to claim this call_id gets the output; later same-id calls get None.
                let output = if paired.insert(call_id) {
                    first_output.get(call_id).copied()
                } else {
                    None
                };
                out.push(ViewBlock::ToolSpan { call: it, output });
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

/// Stage 2: pair tool spans over the (already Stage-1-filtered) item view, then splice the
/// live streaming tail (reasoning, then message). `active_response` is reserved for Stage 3.
pub fn project<'a>(
    items: &[&'a Item],
    scratch: &'a StreamScratch,
    active_response: Option<&'a ResponseId>,
) -> Vec<ViewBlock<'a>> {
    project_filtered(items, scratch, active_response, true)
}

/// `project` with explicit control over whether the reasoning tail is spliced — the caller
/// sets `splice_reasoning = false` when it applied `hide_reasoning` in Stage 1 (§5.2).
pub fn project_filtered<'a>(
    items: &[&'a Item],
    scratch: &'a StreamScratch,
    active_response: Option<&'a ResponseId>,
    splice_reasoning: bool,
) -> Vec<ViewBlock<'a>> {
    let mut blocks = pair_tool_spans(items);
    if splice_reasoning && let Some(r) = &scratch.open_reasoning {
        blocks.push(ViewBlock::StreamingReasoning {
            response_id: active_response,
            acc: r,
        });
    }
    if let Some(m) = &scratch.open_message {
        blocks.push(ViewBlock::StreamingMessage(m));
    }
    blocks
}

/// No-filter convenience: project the full canonical slice.
pub fn project_all<'a>(
    items: &'a [Item],
    scratch: &'a StreamScratch,
    active_response: Option<&'a ResponseId>,
) -> Vec<ViewBlock<'a>> {
    let refs: Vec<&Item> = items.iter().collect();
    project(&refs, scratch, active_response)
}

/// The response_id a block groups under, or None if it is a sibling (never grouped).
/// `StreamingReasoning` folds under its live section; `StreamingMessage` stays a top-level sibling.
/// Exhaustive over ItemKind — a new kind is a compile error here.
fn grouping_key<'a>(vb: &ViewBlock<'a>) -> Option<&'a ResponseId> {
    fn item_key(i: &Item) -> Option<&ResponseId> {
        match &i.kind {
            // Agent-work: group by the item's authoritative response_id.
            ItemKind::Reasoning { .. }
            | ItemKind::FunctionCall { .. }
            | ItemKind::FunctionCallOutput { .. }
            | ItemKind::NativeTool { .. }
            | ItemKind::AgentChanged { .. } => i.ctx.response_id.as_ref(),
            // Siblings: never grouped even if they carry a response_id.
            ItemKind::Message { .. }
            | ItemKind::ResourceEvent { .. }
            | ItemKind::Compaction { .. }
            | ItemKind::Error { .. }
            | ItemKind::SlashCommand { .. }
            | ItemKind::TerminalCommand { .. } => None,
        }
    }
    match vb {
        ViewBlock::Item(i) => item_key(i),
        ViewBlock::ToolSpan { call, .. } => item_key(call),
        ViewBlock::StreamingReasoning { response_id, .. } => *response_id,
        ViewBlock::WorkSection { .. } | ViewBlock::StreamingMessage(_) => None,
    }
}

/// Stage 3: fold each CONTIGUOUS agent-work run into a `WorkSection`. A sibling (message,
/// ResourceEvent, …) closes the current run and renders in place, so an interleaved turn
/// stays chronological (multiple sections + the siblings between them). Each section carries
/// `run_index` = the number of prior runs of the SAME `response_id`, giving finalize-stable
/// `(response_id, run_index)` keys. `_active` is unused — the collapse decision is the renderer's
/// (derived per `response_id`, T-2 §6/§12).
pub fn group_work_section<'a>(
    blocks: Vec<ViewBlock<'a>>,
    _active: Option<&'a ResponseId>,
) -> Vec<ViewBlock<'a>> {
    use std::collections::HashMap;
    let mut out: Vec<ViewBlock<'a>> = Vec::with_capacity(blocks.len());
    let mut run: Vec<ViewBlock<'a>> = Vec::new();
    let mut run_key: Option<&'a ResponseId> = None;
    let mut run_counts: HashMap<&'a ResponseId, u32> = HashMap::new();

    fn flush<'a>(
        out: &mut Vec<ViewBlock<'a>>,
        run: &mut Vec<ViewBlock<'a>>,
        run_key: &mut Option<&'a ResponseId>,
        run_counts: &mut HashMap<&'a ResponseId, u32>,
    ) {
        if let Some(key) = run_key.take() {
            let idx = run_counts.entry(key).or_insert(0);
            out.push(ViewBlock::WorkSection {
                response_id: key,
                run_index: *idx,
                blocks: std::mem::take(run),
            });
            *idx += 1;
        }
        run.clear();
    }

    for vb in blocks {
        match grouping_key(&vb) {
            Some(key) if run_key == Some(key) => run.push(vb),
            Some(key) => {
                flush(&mut out, &mut run, &mut run_key, &mut run_counts);
                run_key = Some(key);
                run.push(vb);
            }
            None => {
                flush(&mut out, &mut run, &mut run_key, &mut run_counts);
                out.push(vb);
            }
        }
    }
    flush(&mut out, &mut run, &mut run_key, &mut run_counts);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, CallId, ItemId};
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

    fn scratch_with(reasoning: Option<ReasoningAcc>, message: Option<MessageAcc>) -> StreamScratch {
        StreamScratch {
            open_message: message,
            open_reasoning: reasoning,
            ..Default::default()
        }
    }

    fn r_acc() -> ReasoningAcc {
        ReasoningAcc {
            full_text: "thinking".into(),
            summary_text: String::new(),
            encrypted: false,
        }
    }

    fn m_acc() -> MessageAcc {
        MessageAcc {
            message_id: Some("msg_live".into()),
            text: "partial".into(),
            block_index: 0,
        }
    }

    #[test]
    fn pairs_call_with_following_output() {
        let items = [
            call("c1", None, "call_1", "completed"),
            output("o1", None, "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_span(&out[0], "c1", Some("o1"));
    }

    #[test]
    fn splices_reasoning_then_message_after_finalized() {
        let items = [msg("m1", Some("resp_a"), Role::Assistant, "done")];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), Some(m_acc()));
        let resp_a = ResponseId::new("resp_a");
        let out = project(&refs, &scratch, Some(&resp_a));
        assert_eq!(out.len(), 3);
        assert_item(&out[0], "m1");
        assert!(matches!(out[1], ViewBlock::StreamingReasoning { .. }));
        assert!(matches!(out[2], ViewBlock::StreamingMessage(_)));
    }

    #[test]
    fn no_tail_when_scratch_empty() {
        let items = [msg("m1", Some("resp_a"), Role::Assistant, "done")];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let out = project(&refs, &scratch, None);
        assert_eq!(out.len(), 1);
        assert_item(&out[0], "m1");
    }

    #[test]
    fn message_only_tail() {
        let items: [Item; 0] = [];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, Some(m_acc()));
        let out = project(&refs, &scratch, None);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], ViewBlock::StreamingMessage(_)));
    }

    #[test]
    fn filter_consistency_hide_reasoning_suppresses_streaming_reasoning() {
        // When the caller applies hide_reasoning (Stage 1), it must also suppress the
        // reasoning tail. project() honors this by taking a `splice_reasoning` decision
        // from the caller — modeled here by the caller pre-filtering + passing the flag.
        let items: [Item; 0] = [];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), Some(m_acc()));
        // hide_reasoning path: project_filtered suppresses the reasoning tail.
        let out = project_filtered(&refs, &scratch, None, false);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], ViewBlock::StreamingMessage(_)));
    }

    #[test]
    fn non_completed_call_without_output_is_span_with_none() {
        let items = [call("c1", None, "call_1", "in_progress")];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_span(&out[0], "c1", None);
    }

    #[test]
    fn orphan_output_passes_through() {
        let items = [output("o1", None, "call_missing")];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 1);
        assert_item(&out[0], "o1");
    }

    #[test]
    fn output_before_call_still_pairs_at_call_position() {
        // Output precedes its call in stream order; span takes the call's position.
        let items = [
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
        let items = [
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
        let items = [
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

    #[test]
    fn reused_call_id_second_call_gets_none_output() {
        // Two FunctionCall items share call_1; the single output pairs to the FIRST call only.
        // The second call gets output:None — the output must not be double-counted.
        let items = [
            call("c1", None, "call_1", "completed"),
            call("c2", None, "call_1", "completed"),
            output("o1", None, "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let out = pair_tool_spans(&refs);
        assert_eq!(out.len(), 2);
        assert_span(&out[0], "c1", Some("o1"));
        assert_span(&out[1], "c2", None);
    }

    fn reasoning(id: &str, resp: Option<&str>) -> Item {
        item(
            id,
            resp,
            ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: String::new(),
                encrypted: false,
            },
        )
    }

    fn resource_event(id: &str) -> Item {
        use lens_client::generated::{SessionResourceObject, Type};
        item(
            id,
            None,
            ItemKind::ResourceEvent {
                resource: SessionResourceObject {
                    environment: None,
                    id: "default".into(),
                    metadata: serde_json::Map::new(),
                    name: "workspace".into(),
                    object: "session.resource".into(),
                    session_id: "conv_1".into(),
                    type_: Type::Environment,
                },
            },
        )
    }

    fn assert_section_ri<'a>(vb: &'a ViewBlock<'a>, resp: &str) -> (&'a [ViewBlock<'a>], u32) {
        match vb {
            ViewBlock::WorkSection {
                response_id,
                run_index,
                blocks,
            } => {
                assert_eq!(response_id.as_str(), resp);
                (blocks.as_slice(), *run_index)
            }
            other => panic!("expected WorkSection({resp}), got {other:?}"),
        }
    }

    fn assert_section<'a>(vb: &'a ViewBlock<'a>, resp: &str) -> &'a [ViewBlock<'a>] {
        let (blocks, _) = assert_section_ri(vb, resp);
        blocks
    }

    #[test]
    fn settled_response_folds_into_section() {
        // reasoning + tool-span share resp_a; no active_response ⇒ fold.
        let items = [
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, None);
        let out = group_work_section(projected, None);
        assert_eq!(out.len(), 1);
        let inner = assert_section(&out[0], "resp_a");
        assert_eq!(inner.len(), 2); // reasoning + tool-span
    }

    #[test]
    fn live_response_also_folds_into_section() {
        let items = [
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let resp_a = ResponseId::new("resp_a");
        // Even when resp_a is active, grouping folds it — expanded-vs-collapsed is the renderer's job now.
        let out = group_work_section(project(&refs, &scratch, Some(&resp_a)), Some(&resp_a));
        assert_eq!(out.len(), 1);
        let inner = assert_section(&out[0], "resp_a");
        assert_eq!(inner.len(), 2); // reasoning + tool-span
    }

    #[test]
    fn user_message_and_resource_are_siblings_before_section() {
        // user msg (sibling) → resp_a work (folds) → assistant msg + ResourceEvent siblings (flat).
        let items = [
            msg("u1", None, Role::User, "do a thing"),
            reasoning("r1", Some("resp_a")),
            msg("a1", Some("resp_a"), Role::Assistant, "final text"),
            resource_event("res1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, None);
        let out = group_work_section(projected, None);
        // u1 sibling | WorkSection(resp_a){reasoning} | a1 sibling | res1 sibling
        assert_eq!(out.len(), 4);
        assert_item(&out[0], "u1");
        let inner = assert_section(&out[1], "resp_a");
        assert_eq!(inner.len(), 1);
        assert_item(&out[2], "a1");
        assert_item(&out[3], "res1");
    }

    #[test]
    fn multi_response_sequence_folds_each_separately() {
        let items = [
            reasoning("r1", Some("resp_a")),
            reasoning("r2", Some("resp_b")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, None);
        let out = group_work_section(projected, None);
        assert_eq!(out.len(), 2);
        assert_section(&out[0], "resp_a");
        assert_section(&out[1], "resp_b");
    }

    #[test]
    fn idle_folds_all_but_active_folds_all_others() {
        // Two responses; both fold — active_response no longer suppresses grouping.
        let items = [
            reasoning("r1", Some("resp_a")),
            reasoning("r2", Some("resp_b")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let resp_b = ResponseId::new("resp_b");
        let projected = project(&refs, &scratch, Some(&resp_b));
        let out = group_work_section(projected, Some(&resp_b));
        assert_eq!(out.len(), 2);
        assert_section(&out[0], "resp_a");
        assert_section(&out[1], "resp_b");
    }

    #[test]
    fn streaming_tail_never_grouped() {
        let items = [reasoning("r1", Some("resp_a"))];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), None);
        let resp_a = ResponseId::new("resp_a");
        let projected = project(&refs, &scratch, Some(&resp_a));
        let out = group_work_section(projected, Some(&resp_a));
        // resp_a folds; the StreamingReasoning tail folds into the section.
        assert_eq!(out.len(), 1);
        let inner = assert_section(&out[0], "resp_a");
        assert_eq!(inner.len(), 2);
        assert_item(&inner[0], "r1");
        assert!(matches!(inner[1], ViewBlock::StreamingReasoning { .. }));
    }

    // Count how many input Items are represented across the ViewBlock tree (recursively).
    fn covered_item_ids<'a>(blocks: &[ViewBlock<'a>], acc: &mut Vec<String>) {
        for b in blocks {
            match b {
                ViewBlock::Item(i) => acc.push(i.id.as_str().to_string()),
                ViewBlock::ToolSpan { call, output } => {
                    acc.push(call.id.as_str().to_string());
                    if let Some(o) = output {
                        acc.push(o.id.as_str().to_string());
                    }
                }
                ViewBlock::WorkSection { blocks, .. } => covered_item_ids(blocks, acc),
                ViewBlock::StreamingReasoning { .. } | ViewBlock::StreamingMessage(_) => {}
            }
        }
    }

    #[test]
    fn full_pipeline_agent_changed_inside_section_live_tail() {
        // user msg (sibling) → resp_a settled work (reasoning + agent_changed + tool) →
        // resp_b live section (reasoning + streaming tail).
        let ac = item(
            "ac1",
            Some("resp_a"),
            ItemKind::AgentChanged {
                from: AgentId::new("coder"),
                to: AgentId::new("researcher"),
                at: 0,
            },
        );
        let items = [
            msg("u1", None, Role::User, "go"),
            reasoning("r1", Some("resp_a")),
            ac,
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
            reasoning("r2", Some("resp_b")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), None);
        let resp_b = ResponseId::new("resp_b");
        let projected = project(&refs, &scratch, Some(&resp_b));
        let out = group_work_section(projected, Some(&resp_b));
        // u1 sibling | WorkSection(resp_a){...} | WorkSection(resp_b){r2, StreamingReasoning}
        assert_eq!(out.len(), 3);
        assert_item(&out[0], "u1");
        let inner = assert_section(&out[1], "resp_a");
        assert_eq!(inner.len(), 3);
        assert_item(&inner[0], "r1");
        assert_item(&inner[1], "ac1");
        assert_span(&inner[2], "c1", Some("o1"));
        let inner_b = assert_section(&out[2], "resp_b");
        assert_eq!(inner_b.len(), 2);
        assert_item(&inner_b[0], "r2");
        assert!(matches!(inner_b[1], ViewBlock::StreamingReasoning { .. }));
    }

    #[test]
    fn disk_only_paint_folds_everything() {
        let items = [
            msg("u1", None, Role::User, "go"),
            reasoning("r1", Some("resp_a")),
            msg("a1", Some("resp_a"), Role::Assistant, "answer"),
        ];
        let scratch = scratch_with(None, None);
        let out = group_work_section(project_all(&items, &scratch, None), None);
        assert_eq!(out.len(), 3);
        assert_item(&out[0], "u1");
        assert_section(&out[1], "resp_a");
        assert_item(&out[2], "a1");
    }

    #[test]
    fn every_item_appears_exactly_once() {
        let items = [
            msg("u1", None, Role::User, "go"),
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
            msg("a1", Some("resp_a"), Role::Assistant, "answer"),
            reasoning("r2", Some("resp_b")),
        ];
        let scratch = scratch_with(None, None);
        let out = group_work_section(project_all(&items, &scratch, None), None);
        let mut covered = Vec::new();
        covered_item_ids(&out, &mut covered);
        covered.sort();
        let mut expected: Vec<String> = items.iter().map(|i| i.id.as_str().to_string()).collect();
        expected.sort();
        assert_eq!(
            covered, expected,
            "every input Item must appear exactly once"
        );
    }

    #[test]
    fn hide_reasoning_filter_removes_items_from_coverage() {
        // Stage-1 hide_reasoning drops reasoning; those ids are legitimately absent.
        use crate::reduce::transforms::hide_reasoning;
        let items = [
            reasoning("r1", Some("resp_a")),
            msg("a1", Some("resp_a"), Role::Assistant, "answer"),
        ];
        let filtered = hide_reasoning(&items); // Vec<&Item>
        let scratch = scratch_with(Some(r_acc()), None);
        // hide_reasoning applied ⇒ splice_reasoning = false (§5.2 filter consistency).
        let projected = project_filtered(&filtered, &scratch, None, false);
        let out = group_work_section(projected, None);
        let mut covered = Vec::new();
        covered_item_ids(&out, &mut covered);
        assert_eq!(covered, vec!["a1".to_string()]);
        assert!(
            !out.iter()
                .any(|b| matches!(b, ViewBlock::StreamingReasoning { .. }))
        );
    }

    #[test]
    fn interleaved_message_keeps_two_runs_in_order_with_run_index() {
        // reasoning(resp_a), assistant msg(resp_a) [sibling], reasoning(resp_a) again.
        let items = [
            reasoning("r1", Some("resp_a")),
            msg("a1", Some("resp_a"), Role::Assistant, "narration"),
            reasoning("r2", Some("resp_a")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let out = group_work_section(project(&refs, &scratch, None), None);
        // section(resp_a,#0){r1} | a1 sibling IN PLACE | section(resp_a,#1){r2} — order preserved.
        assert_eq!(out.len(), 3);
        let (i0, ri0) = assert_section_ri(&out[0], "resp_a");
        assert_eq!(ri0, 0);
        assert_item(&i0[0], "r1");
        assert_item(&out[1], "a1");
        let (i1, ri1) = assert_section_ri(&out[2], "resp_a");
        assert_eq!(ri1, 1);
        assert_item(&i1[0], "r2");
    }

    #[test]
    fn streaming_reasoning_carries_active_response_id() {
        let items: [Item; 0] = [];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), None);
        let resp_a = ResponseId::new("resp_a");
        let out = project(&refs, &scratch, Some(&resp_a));
        match &out[0] {
            ViewBlock::StreamingReasoning { response_id, .. } => {
                assert_eq!(response_id.map(|r| r.as_str()), Some("resp_a"));
            }
            other => panic!("expected StreamingReasoning, got {other:?}"),
        }
    }
}
