# Transcript T-1 — ViewBlock Projection Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land a pure, borrow-only projection from a session's canonical `&[Item]` (+ RAM-only `StreamScratch` + the T-0 active-response signal) into `Vec<ViewBlock>` in lens-core — the render spine every transcript slice (T-2..T-7) reads from.

**Architecture:** A new `crates/lens-core/src/reduce/view.rs` module defines the borrowing `ViewBlock<'a>` enum and a three-stage pipeline: Stage 1 = existing `[Item]→[&Item]` filters (unchanged, in `transforms.rs`); Stage 2 = `project()` pairing tool spans + splicing the streaming tail; Stage 3 = `group_work_section()` folding each response's work into a `WorkSection` keyed on the authoritative `response_id`, leaving the live response (== `active_response`) flat. No gpui, no clones in the block tree, no `pending` input.

**Tech Stack:** Rust 2024, lens-core reduce module, `#[cfg(test)]` inline table-driven tests (the existing `reduce/` idiom — hand-authored `Item` lists, no snapshot dep).

## Global Constraints

- **Spec:** `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` — this plan is its faithful decomposition; do not re-open product questions.
- **Pure & borrow-only** over `(items, scratch, active_response)`. No clone in the `ViewBlock` tree; no `pending` input; no gpui; no session-level input beyond those three.
- **Exhaustive `ItemKind` match** in the projector and grouper — **no wildcard `_` arm**. A server-added `ItemKind` must be a compile error. (Existing enum variants: `Message`, `FunctionCall`, `FunctionCallOutput`, `Reasoning`, `NativeTool`, `Compaction`, `SlashCommand`, `TerminalCommand`, `Error`, `ResourceEvent`, `AgentChanged` — see `crates/lens-core/src/domain/item.rs`.)
- **Turn identity + liveness = authoritative `response_id`** (from T-0: `BlockContext.response_id` per item, session `active_response`). NO `ctx.turn`/`scratch.turn` heuristic anywhere.
- **Gate:** `cargo run -p xtask -- gate` must stay green (fmt + workspace clippy `-D warnings` + tests + drift). There is **no** `cargo xtask` alias.
- **Scope fences (do NOT implement here):** `OptimisticUser` / pending (T-7), `ReconnectBreak` (T-2), `SubAgentSpan`/`ChildRef`/real `flatten_sub_agents` (T-5), `WorkSectionMeta` duration/model/tokens/cost/transitions + expand-collapse state (T-6). `merge_text_for_display` is NOT wired into the pipeline (owned-return; §3).
- **`created_at` is NOT used** for grouping or ordering in T-1 (unreliable on disk until T-6; irrelevant to structure).

---

## File Structure

- **Create:** `crates/lens-core/src/reduce/view.rs` — `ViewBlock<'a>` enum + `pair_tool_spans` (Stage 2 helper) + `project` / `project_all` (Stage 2) + `group_work_section` (Stage 3) + all inline tests.
- **Modify:** `crates/lens-core/src/reduce/mod.rs` — add `pub mod view;` (beside `pub mod transforms;` at line 9) and re-export the public surface.
- **Modify (re-export only):** `crates/lens-core/src/lib.rs:17` — extend the `pub use reduce::{...}` so consumers (lens-ui, T-2) can name `ViewBlock`, `project`, `project_all`, `group_work_section`.
- **No changes** to `transforms.rs` (Stage 1 reused as-is), domain types, persistence, or the actor.

### Reference: exact domain shapes this plan borrows (from `domain/item.rs`, `domain/ids.rs`)

```rust
// domain/ids.rs
branded_id!(ItemId, CallId, ResponseId, AgentId, BoardId, BoardItemId); // ResponseId: newtype over String
impl ResponseId { pub fn from_wire(s: Option<&str>) -> Option<ResponseId> { /* empty→None */ } }

// domain/item.rs
pub struct BlockContext { pub agent: Option<String>, pub depth: u32, pub response_id: Option<ResponseId> }
pub struct Item { pub id: ItemId, pub seq: Option<u64>, pub ctx: BlockContext, pub created_at: i64, pub kind: ItemKind }
pub enum ItemKind { Message{role,content}, FunctionCall{call_id,name,arguments,status,agent_name},
    FunctionCallOutput{call_id,output,arguments}, Reasoning{full_text,summary_text,encrypted},
    NativeTool{tool_type,data}, Compaction{summary,token_count}, SlashCommand{name,raw},
    TerminalCommand{command}, Error{source,code,message}, ResourceEvent{resource}, AgentChanged{from,to,at} }
pub struct StreamScratch { pub open_message: Option<MessageAcc>, pub open_reasoning: Option<ReasoningAcc>,
    pub unpaired_calls: HashMap<CallId,ItemId>, pub turn: u32, pub current_agent: Option<String> }
pub struct MessageAcc { pub message_id: Option<String>, pub text: String, pub block_index: usize }
pub struct ReasoningAcc { pub full_text: String, pub summary_text: String, pub encrypted: bool }
```

`CallId` compares by value (`branded_id!` derives `PartialEq, Eq, Hash`). `ResponseId` likewise — group membership is `Option<&ResponseId>` equality.

---

## Task 1: `ViewBlock` enum + `pair_tool_spans` (Stage 2 core)

Establishes the borrowing enum and the tool-pairing transform. This is the load-bearing Stage-2 primitive; the streaming splice (Task 2) and grouper (Task 3) build on the block vec it produces.

**Files:**
- Create: `crates/lens-core/src/reduce/view.rs`
- Modify: `crates/lens-core/src/reduce/mod.rs` (add `pub mod view;`)
- Test: inline `#[cfg(test)] mod tests` in `view.rs`

**Interfaces:**
- Consumes: `Item`, `ItemKind`, `ReasoningAcc`, `MessageAcc` (`crate::domain::item`); `ResponseId` (`crate::domain::ids`).
- Produces:
  ```rust
  pub enum ViewBlock<'a> {
      Item(&'a Item),
      ToolSpan { call: &'a Item, output: Option<&'a Item> },
      WorkSection { response_id: &'a ResponseId, blocks: Vec<ViewBlock<'a>> },
      StreamingReasoning(&'a ReasoningAcc),
      StreamingMessage(&'a MessageAcc),
  }
  pub fn pair_tool_spans<'a>(items: &[&'a Item]) -> Vec<ViewBlock<'a>>;
  ```

- [ ] **Step 1: Write the failing tests for `pair_tool_spans`**

Create `crates/lens-core/src/reduce/view.rs` with the enum, a `todo!()` stub body for `pair_tool_spans`, and this test module. Helper constructors are local to the test module.

```rust
//! §3 pure view-projection: canonical `&[Item]` (+ scratch + active_response) → `Vec<ViewBlock>`.
//! Borrow-only; no clones in the block tree; no `pending`; no gpui (T-1 spec).

use crate::domain::ids::ResponseId;
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
    todo!()
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
```

- [ ] **Step 2: Register the module and run tests to verify they fail**

Add to `crates/lens-core/src/reduce/mod.rs` beside `pub mod transforms;` (line 9):

```rust
pub mod view;
```

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: FAIL — `not yet implemented` panic (the `todo!()`), all six `pair_tool_spans` tests failing.

- [ ] **Step 3: Implement `pair_tool_spans`**

Replace the `todo!()` body. Compute the consumed-output set **globally before the walk** so the order of call vs output is irrelevant (output-before-call must NOT leak the output as a passthrough). Pass 1 indexes `call_id → first output item` and the set of `call_id`s present. `consumed` = the id of each first-output whose `call_id` has a matching `FunctionCall` in the window. Pass 2 walks in stream order: `FunctionCall` → `ToolSpan` (pairing its first output, if any); `FunctionCallOutput` → passthrough only if not in `consumed` (orphan, or a duplicate/second output for the call); everything else → passthrough.

```rust
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
```

Bring `CallId` into scope at the top of `view.rs`: change the import to `use crate::domain::ids::{CallId, ResponseId};`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: PASS — all six `pair_tool_spans` tests green, including `output_before_call_still_pairs_at_call_position` (the global `consumed` set makes call/output order irrelevant) and `duplicate_output_for_same_call_id_passes_through` (only the first output is in `consumed`; the second passes through).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/reduce/view.rs crates/lens-core/src/reduce/mod.rs
git commit -m "feat(lens-core): ViewBlock enum + pair_tool_spans (T-1 stage 2 core)"
```

---

## Task 2: `project` / `project_all` — Stage 2 with streaming-tail splice

Wraps `pair_tool_spans` with the live streaming tail from `scratch`, respecting the Stage-1 filter set. This is the full Stage-2 entry point.

**Files:**
- Modify: `crates/lens-core/src/reduce/view.rs`
- Test: inline tests in `view.rs`

**Interfaces:**
- Consumes: `pair_tool_spans` (Task 1); `StreamScratch`, `MessageAcc`, `ReasoningAcc` (`crate::domain::item`); `ResponseId`.
- Produces:
  ```rust
  pub fn project<'a>(
      items: &[&'a Item],
      scratch: &'a StreamScratch,
      _active_response: Option<&'a ResponseId>, // reserved; grouping consumes it in Stage 3
  ) -> Vec<ViewBlock<'a>>;

  pub fn project_all<'a>(
      items: &'a [Item],
      scratch: &'a StreamScratch,
      active_response: Option<&'a ResponseId>,
  ) -> Vec<ViewBlock<'a>>;
  ```
  Splice order after the paired blocks: `StreamingReasoning` then `StreamingMessage`. `project` does NOT filter — the caller supplies the already-filtered `&[&Item]` view; filter consistency for the streaming tail is the caller's contract (see Step 1 note). `project_all` is the no-filter convenience: `project(&items.iter().collect::<Vec<_>>(), scratch, active_response)`.

- [ ] **Step 1: Write the failing tests for the streaming splice**

Append to the `tests` module in `view.rs`. Add a `scratch` builder helper.

```rust
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
    fn splices_reasoning_then_message_after_finalized() {
        let items = vec![msg("m1", Some("resp_a"), Role::Assistant, "done")];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), Some(m_acc()));
        let out = project(&refs, &scratch, Some(&ResponseId::new("resp_a")));
        assert_eq!(out.len(), 3);
        assert_item(&out[0], "m1");
        assert!(matches!(out[1], ViewBlock::StreamingReasoning(_)));
        assert!(matches!(out[2], ViewBlock::StreamingMessage(_)));
    }

    #[test]
    fn no_tail_when_scratch_empty() {
        let items = vec![msg("m1", Some("resp_a"), Role::Assistant, "done")];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let out = project(&refs, &scratch, None);
        assert_eq!(out.len(), 1);
        assert_item(&out[0], "m1");
    }

    #[test]
    fn message_only_tail() {
        let items: Vec<Item> = vec![];
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
        let items: Vec<Item> = vec![];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), Some(m_acc()));
        // hide_reasoning path: project_filtered suppresses the reasoning tail.
        let out = project_filtered(&refs, &scratch, None, false);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], ViewBlock::StreamingMessage(_)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: FAIL — `project`, `project_filtered` not found.

- [ ] **Step 3: Implement `project`, `project_filtered`, `project_all`**

The filter-consistency requirement (§5.2: "the splice respects the Stage-1 filter set") means the streaming-reasoning tail must be suppressed exactly when `hide_reasoning` ran. Since `project` can't see which Stage-1 filters the caller applied, expose the decision as an explicit `splice_reasoning: bool` param on an inner `project_filtered`, and have `project` default it to `true` (no filter):

```rust
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
    _active_response: Option<&'a ResponseId>,
    splice_reasoning: bool,
) -> Vec<ViewBlock<'a>> {
    let mut blocks = pair_tool_spans(items);
    if splice_reasoning
        && let Some(r) = &scratch.open_reasoning
    {
        blocks.push(ViewBlock::StreamingReasoning(r));
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: PASS — all Task 1 + Task 2 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/reduce/view.rs
git commit -m "feat(lens-core): project/project_all Stage 2 with filter-consistent streaming tail (T-1)"
```

---

## Task 3: `group_work_section` — Stage 3 response grouping + liveness

Folds each response's agent-work blocks into one `WorkSection` keyed on the shared `response_id`, leaving the live response (`== active_response`) flat and all others (or all, when idle) folded. This is the load-bearing grouping.

**Files:**
- Modify: `crates/lens-core/src/reduce/view.rs`
- Test: inline tests in `view.rs`

**Interfaces:**
- Consumes: `ViewBlock` (Task 1), `project` (Task 2), `ResponseId`.
- Produces:
  ```rust
  pub fn group_work_section<'a>(
      blocks: Vec<ViewBlock<'a>>,
      active_response: Option<&'a ResponseId>,
  ) -> Vec<ViewBlock<'a>>;
  ```

**Grouping rules (spec §5.3), exhaustive over `ItemKind` — no wildcard:**

Each top-level `ViewBlock` gets a `response_id: Option<&ResponseId>` derived from its underlying item:
- `ViewBlock::Item(i)` / `ViewBlock::ToolSpan { call: i, .. }` → derive from `i`.
- **Agent-work items** (belong in a section, keyed by their `ctx.response_id`): `Reasoning`, `FunctionCall`/`FunctionCallOutput` (via `ToolSpan`), `NativeTool`, `AgentChanged`.
- **Sibling items** (never grouped; `response_id = None` for grouping purposes even if the item carries one): `Message` (user AND assistant — §4 "final assistant text stays visible"), `ResourceEvent`, `Compaction`, `Error`, `SlashCommand`, `TerminalCommand`.
- `StreamingReasoning` / `StreamingMessage` → **never grouped** (§5.2); always flat, always at the tail.

A run of consecutive agent-work blocks sharing the same `Some(response_id)` folds into one `WorkSection { response_id, blocks }` at the position of the run — UNLESS that `response_id == active_response`, in which case the run stays flat (the live turn). A sibling (or a `None`-keyed block, or a streaming tail block) breaks the run.

- [ ] **Step 1: Write the failing tests for grouping**

Append to the `tests` module. Add a native-tool + reasoning helper.

```rust
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

    fn assert_section<'a>(vb: &'a ViewBlock<'a>, resp: &str) -> &'a [ViewBlock<'a>] {
        match vb {
            ViewBlock::WorkSection { response_id, blocks } => {
                assert_eq!(response_id.as_str(), resp);
                blocks.as_slice()
            }
            other => panic!("expected WorkSection({resp}), got {other:?}"),
        }
    }

    #[test]
    fn settled_response_folds_into_section() {
        // reasoning + tool-span share resp_a; no active_response ⇒ fold.
        let items = vec![
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
    fn live_response_stays_flat() {
        let items = vec![
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, Some(&ResponseId::new("resp_a")));
        let out = group_work_section(projected, Some(&ResponseId::new("resp_a")));
        // resp_a is live ⇒ flat: reasoning + tool-span, no WorkSection.
        assert_eq!(out.len(), 2);
        assert!(!matches!(out[0], ViewBlock::WorkSection { .. }));
    }

    #[test]
    fn user_message_and_resource_are_siblings_before_section() {
        // user msg (sibling) then resp_a work; user msg stays flat before the section.
        let items = vec![
            msg("u1", None, Role::User, "do a thing"),
            reasoning("r1", Some("resp_a")),
            msg("a1", Some("resp_a"), Role::Assistant, "final text"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, None);
        let out = group_work_section(projected, None);
        // u1 sibling, then WorkSection(resp_a){reasoning}, then a1 assistant message sibling.
        assert_eq!(out.len(), 3);
        assert_item(&out[0], "u1");
        let inner = assert_section(&out[1], "resp_a");
        assert_eq!(inner.len(), 1);
        assert_item(&out[2], "a1");
    }

    #[test]
    fn multi_response_sequence_folds_each_separately() {
        let items = vec![
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
        // Two responses; resp_b is active ⇒ resp_a folds, resp_b flat.
        let items = vec![
            reasoning("r1", Some("resp_a")),
            reasoning("r2", Some("resp_b")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(None, None);
        let projected = project(&refs, &scratch, Some(&ResponseId::new("resp_b")));
        let out = group_work_section(projected, Some(&ResponseId::new("resp_b")));
        assert_eq!(out.len(), 2);
        assert_section(&out[0], "resp_a");
        assert!(matches!(out[1], ViewBlock::StreamingReasoning(_) | ViewBlock::Item(_)));
    }

    #[test]
    fn streaming_tail_never_grouped() {
        let items = vec![reasoning("r1", Some("resp_a"))];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), None);
        let projected = project(&refs, &scratch, None);
        let out = group_work_section(projected, None);
        // resp_a folds; the StreamingReasoning tail stays flat after it.
        assert_eq!(out.len(), 2);
        assert_section(&out[0], "resp_a");
        assert!(matches!(out[1], ViewBlock::StreamingReasoning(_)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: FAIL — `group_work_section` not found.

- [ ] **Step 3: Implement `group_work_section`**

Two helpers: `grouping_key` maps a top-level `ViewBlock` to `Option<&ResponseId>` (None for siblings + streaming, Some for agent-work), and the main loop coalesces consecutive equal `Some` keys — unless that key is the active response.

```rust
/// The response_id a block groups under, or None if it is a sibling / streaming tail (never grouped).
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
        ViewBlock::WorkSection { .. }
        | ViewBlock::StreamingReasoning(_)
        | ViewBlock::StreamingMessage(_) => None,
    }
}

/// Stage 3: fold each settled response's consecutive agent-work run into a `WorkSection`.
/// The response `== active_response` stays flat (live turn); all others fold.
pub fn group_work_section<'a>(
    blocks: Vec<ViewBlock<'a>>,
    active_response: Option<&'a ResponseId>,
) -> Vec<ViewBlock<'a>> {
    let mut out: Vec<ViewBlock<'a>> = Vec::with_capacity(blocks.len());
    let mut run: Vec<ViewBlock<'a>> = Vec::new();
    let mut run_key: Option<&'a ResponseId> = None;

    fn flush<'a>(
        out: &mut Vec<ViewBlock<'a>>,
        run: &mut Vec<ViewBlock<'a>>,
        run_key: &mut Option<&'a ResponseId>,
        active: Option<&'a ResponseId>,
    ) {
        if let Some(key) = run_key.take() {
            if Some(key) == active {
                out.append(run); // live turn: stay flat
            } else {
                out.push(ViewBlock::WorkSection {
                    response_id: key,
                    blocks: std::mem::take(run),
                });
            }
        }
        run.clear();
    }

    for vb in blocks {
        match grouping_key(&vb) {
            Some(key) if run_key == Some(key) => run.push(vb),
            Some(key) => {
                flush(&mut out, &mut run, &mut run_key, active_response);
                run_key = Some(key);
                run.push(vb);
            }
            None => {
                flush(&mut out, &mut run, &mut run_key, active_response);
                out.push(vb);
            }
        }
    }
    flush(&mut out, &mut run, &mut run_key, active_response);
    out
}
```

> **Note:** `run_key == Some(key)` compares `Option<&ResponseId>` by value equality (`ResponseId: PartialEq`). The `flush` helper takes `&mut run_key` and leaves it `None` after a flush (`take`), so the "start new run" arm always sets it fresh.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: PASS — all Task 1–3 tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/reduce/view.rs
git commit -m "feat(lens-core): group_work_section Stage 3 response grouping + liveness gate (T-1)"
```

---

## Task 4: Full-pipeline integration fixtures + exactly-once invariant + public re-exports

Proves the three stages compose correctly on realistic item lists, asserts the exactly-once coverage invariant, and exposes the public surface from the crate root.

**Files:**
- Modify: `crates/lens-core/src/reduce/view.rs` (integration tests)
- Modify: `crates/lens-core/src/reduce/mod.rs` (re-export)
- Modify: `crates/lens-core/src/lib.rs:17` (crate-root re-export)
- Test: inline tests in `view.rs`

**Interfaces:**
- Consumes: `project`, `project_all`, `group_work_section` (Tasks 2–3), Stage-1 `hide_reasoning` (`crate::reduce::transforms`).
- Produces: crate-root exports `ViewBlock`, `project`, `project_all`, `project_filtered`, `group_work_section`, `pair_tool_spans`.

- [ ] **Step 1: Write the failing integration + invariant tests**

Append to the `tests` module.

```rust
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
                ViewBlock::StreamingReasoning(_) | ViewBlock::StreamingMessage(_) => {}
            }
        }
    }

    #[test]
    fn full_pipeline_agent_changed_inside_section_live_tail() {
        // user msg (sibling) → resp_a settled work (reasoning + agent_changed + tool) →
        // resp_b live (flat) → streaming tail.
        let ac = item(
            "ac1",
            Some("resp_a"),
            ItemKind::AgentChanged {
                from: AgentId::new("coder"),
                to: AgentId::new("researcher"),
                at: 0,
            },
        );
        let items = vec![
            msg("u1", None, Role::User, "go"),
            reasoning("r1", Some("resp_a")),
            ac,
            call("c1", Some("resp_a"), "call_1", "completed"),
            output("o1", Some("resp_a"), "call_1"),
            reasoning("r2", Some("resp_b")),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = scratch_with(Some(r_acc()), None);
        let projected = project(&refs, &scratch, Some(&ResponseId::new("resp_b")));
        let out = group_work_section(projected, Some(&ResponseId::new("resp_b")));
        // u1 sibling | WorkSection(resp_a){reasoning, agent_changed, tool-span} | r2 flat | streaming
        assert_eq!(out.len(), 4);
        assert_item(&out[0], "u1");
        let inner = assert_section(&out[1], "resp_a");
        assert_eq!(inner.len(), 3);
        assert_item(&inner[0], "r1");
        assert_item(&inner[1], "ac1");
        assert_span(&inner[2], "c1", Some("o1"));
        assert_item(&out[2], "r2"); // resp_b live ⇒ flat
        assert!(matches!(out[3], ViewBlock::StreamingReasoning(_)));
    }

    #[test]
    fn disk_only_paint_folds_everything() {
        let items = vec![
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
        let items = vec![
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
        let mut expected: Vec<String> =
            items.iter().map(|i| i.id.as_str().to_string()).collect();
        expected.sort();
        assert_eq!(covered, expected, "every input Item must appear exactly once");
    }

    #[test]
    fn hide_reasoning_filter_removes_items_from_coverage() {
        // Stage-1 hide_reasoning drops reasoning; those ids are legitimately absent.
        use crate::reduce::transforms::hide_reasoning;
        let items = vec![
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
        assert!(!out.iter().any(|b| matches!(b, ViewBlock::StreamingReasoning(_))));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-core reduce::view 2>&1 | tail -20`
Expected: FAIL — `crate::reduce::transforms::hide_reasoning` reachable, but new integration tests fail only if a composition bug exists; if all Task 1–3 code is correct they may PASS immediately. If they pass, that is acceptable (integration coverage over already-correct units) — proceed. If any fail, fix the Stage-2/3 composition before continuing.

- [ ] **Step 3: Add the public re-exports**

In `crates/lens-core/src/reduce/mod.rs`, after the existing `pub use` lines (around line 17), add:

```rust
pub use view::{
    ViewBlock, group_work_section, pair_tool_spans, project, project_all, project_filtered,
};
```

In `crates/lens-core/src/lib.rs`, extend line 17's re-export:

```rust
pub use reduce::{
    StreamUpdate, Updates, ViewBlock, group_work_section, pair_tool_spans, project, project_all,
    project_filtered, reduce,
};
```

- [ ] **Step 4: Run the full gate to verify everything passes**

Run: `cargo run -p xtask -- gate 2>&1 | tail -15`
Expected: PASS — `gate: all checks passed` (fmt clean, clippy `-D warnings` clean incl. no dead-code/unused-import warnings on the re-exports, all tests green, no drift).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/reduce/view.rs crates/lens-core/src/reduce/mod.rs crates/lens-core/src/lib.rs
git commit -m "feat(lens-core): T-1 pipeline integration tests + exactly-once invariant + public re-exports"
```

---

## Task 5: Cross-family review + spec annotation + docs

Per project rule: every non-trivial change gets ≥1 review from a model family other than the author's. Then annotate the source design (§3.1 says "I annotate §3 when this lands") and update STATUS.

**Files:**
- Modify: `docs/design/conversation-transcript.md` (§3 annotations — the six §3.1 deviations)
- Modify: `docs/STATUS.md` (transcript fan-out: T-1 done)
- Modify: `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` (status → EXECUTED)

- [ ] **Step 1: Request cross-family review of the diff**

Run codex (gpt-5.6 review path, per [[codex-as-reviewer]]) on the T-1 diff — **redirect stdin from /dev/null** so it does not hang:

```bash
# T-1 base = HEAD at plan start (2ab0976); scope the diff to the T-1 files only.
git diff 2ab0976..HEAD -- crates/lens-core/src/reduce/view.rs crates/lens-core/src/reduce/mod.rs crates/lens-core/src/lib.rs > /tmp/t1-diff.txt
codex exec -s read-only "Review this Rust diff for the T-1 ViewBlock projection against docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md. Focus: (1) exhaustive-match/no-wildcard invariant, (2) pair_tool_spans output-before-call + duplicate-output correctness, (3) group_work_section run-coalescing + live-turn-flat correctness, (4) filter-consistency of the streaming reasoning tail, (5) any clone/borrow leaks. Report concrete bugs with file:line." < /dev/null
```

Adjudicate findings against the spec. Fix real bugs (add a failing test first, then fix). Record any false positives with the spec section that refutes them.

- [ ] **Step 2: Annotate the source design (§3.1 obligation)**

In `docs/design/conversation-transcript.md` §3, add a note recording the six landed deviations (WorkSection `{response_id, blocks}` not `{open, meta}`; CompactionMarker/AgentChangedMarker as `Item` passthroughs; OptimisticUser removed; SubAgentSpan → T-5; ReconnectBreak → T-2). Reference the T-1 spec §3.1.

- [ ] **Step 3: Update STATUS + spec status**

Mark T-1 done in `docs/STATUS.md` transcript fan-out; set the T-1 spec header `Status:` to `EXECUTED`.

- [ ] **Step 4: Verify gate still green after any review fixes**

Run: `cargo run -p xtask -- gate 2>&1 | tail -5`
Expected: `gate: all checks passed`.

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs: T-1 ViewBlock projection executed + reviewed; annotate transcript design §3"
```

---

## Self-Review (author checklist — completed at write time)

**Spec coverage:**
- §3 `ViewBlock` enum (5 variants, borrows) → Task 1. ✓
- §3.1 deviations (WorkSection shape, markers as passthroughs, dropped variants) → enforced by the enum in Task 1 + grouping in Task 3; recorded in Task 5. ✓
- §4 staged pipeline (Stage 1 reused, Stage 2 `project`/`project_all`, Stage 3 grouper) → Tasks 2–3; Stage 1 explicitly unchanged (Global Constraints). ✓
- §5.1 `pair_tool_spans` (paired / non-completed None / orphan / output-before-call / interleaved / duplicate) → Task 1 tests, all six cases. ✓
- §5.2 streaming-tail splice (reasoning-then-message, filter consistency, never grouped) → Task 2 (splice + `project_filtered` flag) + Task 3 (`streaming_tail_never_grouped`). ✓
- §5.3 `group_work_section` (membership by response_id, siblings, live flat, idle folds all, multi-response) → Task 3 tests. ✓
- §5.4 `WorkSectionMeta` deferred → enforced (WorkSection carries only `{response_id, blocks}`); Global Constraints fence. ✓
- §7 testing (per-transform table-driven + full-pipeline fixtures + exactly-once invariant) → Tasks 1–4. ✓
- §9 success criteria (lands in lens-core, exhaustive match, response_id grouping/gating no heuristic, gate green, pure/borrow-only) → all tasks + Task 4 gate. ✓

**Deferred-scope fences honored:** no OptimisticUser/pending input, no ReconnectBreak, no SubAgentSpan, no WorkSectionMeta, no gpui, `merge_text_for_display` not wired. ✓

**Type consistency:** `ViewBlock<'a>` variant shapes, `project`/`project_filtered`/`project_all`/`pair_tool_spans`/`group_work_section` signatures, and `grouping_key` are consistent across Tasks 1–4. `ResponseId::new`/`as_str`, `CallId::new`, `ItemId::new`/`as_str` match `branded_id!` (verified against `domain/ids.rs`). `StreamScratch`/`MessageAcc`/`ReasoningAcc` field names match `domain/item.rs`. ✓

**Trickiest spot (handled in-plan):** the output-before-call and duplicate-output edges in `pair_tool_spans`. Both are made correct by computing the `consumed` set globally before the walk (Task 1 Step 3) rather than accumulating it during the walk — so call/output order and second-outputs are handled without a follow-up fix. The two tests that pin this are `output_before_call_still_pairs_at_call_position` and `duplicate_output_for_same_call_id_passes_through`.
