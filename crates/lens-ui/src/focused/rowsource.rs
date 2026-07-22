//! Id-keyed retained row store — owned `RowPresentation` per projected block (T-2 §6).
//! Lifted from `spikes/transcript-virtual/src/rowsource.rs`; projection stays borrow-only
//! in lens-core, materialization happens here for the `'static` `list()` closure.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{App, AppContext, Entity, EntityId, ListState};
use lens_core::domain::ids::{AccId, ItemId, ResponseId};
use lens_core::domain::item::{Item, ItemKind, MessageAcc, ReasoningAcc};
use lens_core::domain::scalars::Role;
use lens_core::reduce::ViewBlock;

/// Stable row identity — keyed store, not list index.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RowId {
    /// Finalize-stable section key: `(response_id, run_index)`.
    Section(ResponseId, u32),
    /// Work child inside a section (reasoning id, tool call item id, …).
    Work(ItemId),
    /// Top-level sibling (message, resource event, …).
    Sibling(ItemId),
    /// Live streaming tail keyed by accumulator id.
    StreamTail(AccId),
    /// Reconnect-break marker (Task 14); also used for synthetic section-rail rows.
    Marker(u64),
}

/// Minimal owned presentation the stub renderer needs — not the whole `Item`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowPresentation {
    pub kind: RowKind,
    pub text: String,
    /// Derived collapse flag (Task 12); materialize seeds `false` (expanded).
    pub collapsed: bool,
    /// Optional height hint for the stub renderer (pixels).
    pub height_hint: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowKind {
    #[default]
    SectionChip,
    SectionRail,
    WorkChild,
    Message,
    UserMessage,
    ResourceEvent,
    StreamingReasoning,
    StreamingMessage,
    ReconnectBreak,
}

/// Per-row retained state. Backends render a handle into this store.
#[derive(Clone)]
pub struct RowState {
    pub id: RowId,
    pub presentation: RowPresentation,
}

/// Result of an id-keyed upsert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpsertEffect {
    Inserted,
    UpdatedInPlace { entity_id_stable: bool },
}

/// Retained `Entity`-per-row store keyed by `RowId`. Order is separate from identity
/// so virtualization recycle never remounts row state.
pub struct RowStore {
    pub(crate) order: Vec<RowId>,
    entities: HashMap<RowId, Entity<RowState>>,
}

impl RowStore {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            entities: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn order(&self) -> &[RowId] {
        &self.order
    }

    pub fn id_at(&self, index: usize) -> Option<&RowId> {
        self.order.get(index)
    }

    pub fn entity(&self, id: &RowId) -> Option<&Entity<RowState>> {
        self.entities.get(id)
    }

    pub fn entity_mut(&mut self, id: &RowId) -> Option<&mut Entity<RowState>> {
        self.entities.get_mut(id)
    }

    pub fn kind_at(&self, index: usize, cx: &App) -> Option<RowKind> {
        let id = self.order.get(index)?;
        let entity = self.entities.get(id)?;
        Some(entity.read(cx).presentation.kind)
    }

    pub fn entity_id(&self, id: &RowId, _cx: &App) -> Option<EntityId> {
        self.entities.get(id).map(|entity| entity.entity_id())
    }

    /// Id-keyed upsert: reuse the existing `Entity` when the `RowId` is present.
    pub fn upsert(&mut self, id: RowId, pres: RowPresentation, cx: &mut App) -> UpsertEffect {
        if let Some(entity) = self.entities.get_mut(&id) {
            entity.update(cx, |state, _| state.presentation = pres);
            UpsertEffect::UpdatedInPlace {
                entity_id_stable: true,
            }
        } else {
            let entity = cx.new(|_| RowState {
                id: id.clone(),
                presentation: pres,
            });
            self.entities.insert(id, entity);
            UpsertEffect::Inserted
        }
    }

    /// Live order/count change — `splice`, never `reset` (reserved for mount / new-session).
    pub fn splice_into(&self, list: &ListState, effect_range: Range<usize>, count: usize) {
        list.splice(effect_range, count);
    }

    /// Height-invalidate a content-mutated row (spike `invalidate_row_height`).
    pub fn invalidate_row_height(&self, list: &ListState, index: usize) {
        self.splice_into(list, index..index + 1, 1);
    }

    /// Owned copy of each projected block into retained entities + flat order.
    pub fn materialize(blocks: &[ViewBlock<'_>], into: &mut RowStore, cx: &mut App) {
        into.order.clear();
        for block in blocks {
            materialize_block(block, into, cx);
        }
    }

    fn push_order(&mut self, id: RowId) {
        self.order.push(id);
    }
}

impl Default for RowStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Synthetic `Marker` seq for a section rail row — high bit keeps Task-14 reconnect seqs separate.
fn section_rail_seq(response_id: &ResponseId, run_index: u32) -> u64 {
    0x8000_0000_0000_0000 | ((run_index as u64) << 32) ^ fnv1a64(response_id.as_str().as_bytes())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn materialize_block(block: &ViewBlock<'_>, into: &mut RowStore, cx: &mut App) {
    match block {
        ViewBlock::Item(item) => materialize_sibling_item(item, into, cx),
        ViewBlock::ToolSpan { call, output } => materialize_tool_span(call, *output, into, cx),
        ViewBlock::WorkSection {
            response_id,
            run_index,
            blocks,
        } => materialize_work_section(response_id, *run_index, blocks, into, cx),
        ViewBlock::StreamingReasoning { acc, .. } => {
            materialize_streaming_reasoning(acc, into, cx);
        }
        ViewBlock::StreamingMessage(acc) => materialize_streaming_message(acc, into, cx),
    }
}

fn materialize_work_section(
    response_id: &ResponseId,
    run_index: u32,
    blocks: &[ViewBlock<'_>],
    into: &mut RowStore,
    cx: &mut App,
) {
    let section_id = RowId::Section(response_id.clone(), run_index);
    let chip = RowPresentation {
        kind: RowKind::SectionChip,
        text: format!("section {} run {run_index}", response_id.as_str()),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(section_id.clone(), chip, cx);
    into.push_order(section_id);

    let rail_id = RowId::Marker(section_rail_seq(response_id, run_index));
    let rail = RowPresentation {
        kind: RowKind::SectionRail,
        text: format!("rail {} run {run_index}", response_id.as_str()),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(rail_id.clone(), rail, cx);
    into.push_order(rail_id);

    for child in blocks {
        materialize_section_child(child, into, cx);
    }
}

fn materialize_section_child(block: &ViewBlock<'_>, into: &mut RowStore, cx: &mut App) {
    match block {
        ViewBlock::Item(item) => materialize_work_item(item, into, cx),
        ViewBlock::ToolSpan { call, output } => {
            materialize_tool_span(call, *output, into, cx);
        }
        ViewBlock::StreamingReasoning { acc, .. } => {
            materialize_streaming_reasoning(acc, into, cx);
        }
        ViewBlock::WorkSection { .. } => {
            // Nested sections are not produced by `group_work_section`; ignore if seen.
        }
        ViewBlock::StreamingMessage(acc) => materialize_streaming_message(acc, into, cx),
    }
}

fn materialize_sibling_item(item: &Item, into: &mut RowStore, cx: &mut App) {
    let id = RowId::Sibling(item.id.clone());
    let pres = RowPresentation {
        kind: sibling_row_kind(item),
        text: item_text_stub(item),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id.clone(), pres, cx);
    into.push_order(id);
}

fn materialize_work_item(item: &Item, into: &mut RowStore, cx: &mut App) {
    let id = RowId::Work(item.id.clone());
    let pres = RowPresentation {
        kind: RowKind::WorkChild,
        text: item_text_stub(item),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id.clone(), pres, cx);
    into.push_order(id);
}

fn materialize_tool_span(call: &Item, output: Option<&Item>, into: &mut RowStore, cx: &mut App) {
    let id = RowId::Work(call.id.clone());
    let mut text = item_text_stub(call);
    if let Some(out) = output {
        text.push_str(" → ");
        text.push_str(&item_text_stub(out));
    }
    let pres = RowPresentation {
        kind: RowKind::WorkChild,
        text,
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id.clone(), pres, cx);
    into.push_order(id);
}

fn materialize_streaming_reasoning(acc: &ReasoningAcc, into: &mut RowStore, cx: &mut App) {
    let id = RowId::StreamTail(acc.acc_id.clone());
    let pres = RowPresentation {
        kind: RowKind::StreamingReasoning,
        text: acc.full_text.clone(),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id.clone(), pres, cx);
    into.push_order(id);
}

fn materialize_streaming_message(acc: &MessageAcc, into: &mut RowStore, cx: &mut App) {
    let id = RowId::StreamTail(acc.acc_id.clone());
    let pres = RowPresentation {
        kind: RowKind::StreamingMessage,
        text: acc.text.clone(),
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id.clone(), pres, cx);
    into.push_order(id);
}

fn sibling_row_kind(item: &Item) -> RowKind {
    match &item.kind {
        ItemKind::Message { role, .. } if *role == Role::User => RowKind::UserMessage,
        ItemKind::Message { .. } => RowKind::Message,
        ItemKind::ResourceEvent { .. } => RowKind::ResourceEvent,
        _ => RowKind::Message,
    }
}

fn item_text_stub(item: &Item) -> String {
    match &item.kind {
        ItemKind::Message { content, .. } => content
            .iter()
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join(""),
        ItemKind::Reasoning { full_text, .. } => full_text.clone(),
        ItemKind::FunctionCall { name, status, .. } => format!("{name} ({status})"),
        ItemKind::FunctionCallOutput { output, .. } => output.clone(),
        ItemKind::ResourceEvent { resource, .. } => resource.name.clone(),
        ItemKind::Compaction { summary, .. } => summary.clone(),
        ItemKind::Error { message, .. } => message.clone(),
        ItemKind::SlashCommand { name, .. } => name.clone(),
        ItemKind::TerminalCommand { command, .. } => command.clone(),
        ItemKind::NativeTool { tool_type, .. } => tool_type.clone(),
        ItemKind::AgentChanged { to, .. } => to.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::ids::CallId;
    use lens_core::domain::item::{BlockContext, ContentBlock};
    use lens_core::reduce::{group_work_section, project};
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

    fn call(id: &str, resp: Option<&str>, call_id: &str) -> Item {
        item(
            id,
            resp,
            ItemKind::FunctionCall {
                call_id: CallId::new(call_id),
                name: "read".into(),
                arguments: Value::Null,
                status: "completed".into(),
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

    fn user_msg(id: &str, text: &str) -> Item {
        item(
            id,
            None,
            ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        )
    }

    fn m_acc() -> MessageAcc {
        MessageAcc {
            acc_id: AccId::new("acc_stream_m"),
            message_id: None,
            text: "partial".into(),
            block_index: 0,
        }
    }

    #[gpui::test]
    fn materialize_work_section_sibling_and_stream_tail(cx: &mut gpui::TestAppContext) {
        let resp_a = ResponseId::new("resp_a");
        let items = [
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1"),
            output("o1", Some("resp_a"), "call_1"),
            user_msg("u1", "hello"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = lens_core::domain::item::StreamScratch {
            open_message: Some(m_acc()),
            ..Default::default()
        };
        let projected = project(&refs, &scratch, Some(&resp_a));
        let blocks = group_work_section(projected, Some(&resp_a));

        let mut store = RowStore::new();
        cx.update(|cx| RowStore::materialize(&blocks, &mut store, cx));

        let kinds: Vec<RowKind> = cx.read(|cx| {
            (0..store.len())
                .map(|ix| store.kind_at(ix, cx).expect("kind"))
                .collect()
        });
        assert_eq!(
            kinds,
            vec![
                RowKind::SectionChip,
                RowKind::SectionRail,
                RowKind::WorkChild,
                RowKind::WorkChild,
                RowKind::UserMessage,
                RowKind::StreamingMessage,
            ]
        );

        let section_ids: Vec<_> = store
            .order()
            .iter()
            .filter_map(|id| match id {
                RowId::Section(resp, run_index) => Some((resp.as_str(), *run_index)),
                _ => None,
            })
            .collect();
        assert_eq!(section_ids, vec![("resp_a", 0)]);

        assert!(matches!(
            store.order().last(),
            Some(RowId::StreamTail(acc)) if acc.as_str() == "acc_stream_m"
        ));
    }

    #[gpui::test]
    fn upsert_same_work_id_preserves_entity_id(cx: &mut gpui::TestAppContext) {
        let item_id = ItemId::new("tool_1");
        let row_id = RowId::Work(item_id);
        let mut store = RowStore::new();

        let first_entity = cx.update(|cx| {
            let effect = store.upsert(
                row_id.clone(),
                RowPresentation {
                    kind: RowKind::WorkChild,
                    text: "v1".into(),
                    collapsed: false,
                    height_hint: None,
                },
                cx,
            );
            assert_eq!(effect, UpsertEffect::Inserted);
            store.entity_id(&row_id, cx).expect("entity")
        });

        cx.update(|cx| {
            let effect = store.upsert(
                row_id.clone(),
                RowPresentation {
                    kind: RowKind::WorkChild,
                    text: "v2".into(),
                    collapsed: false,
                    height_hint: None,
                },
                cx,
            );
            assert_eq!(
                effect,
                UpsertEffect::UpdatedInPlace {
                    entity_id_stable: true
                }
            );
            let second = store.entity_id(&row_id, cx).expect("entity");
            assert_eq!(
                first_entity, second,
                "same RowId must keep the same EntityId"
            );
            assert_eq!(
                store.entity(&row_id).unwrap().read(cx).presentation.text,
                "v2"
            );
        });
    }
}
