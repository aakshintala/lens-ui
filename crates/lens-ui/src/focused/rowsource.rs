//! Id-keyed retained row store — owned `RowPresentation` per projected block (T-2 §6).
//! Two-level nesting: `Section` owns child rows; flattening is derived from the collapse flag.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui::{App, AppContext, Entity, EntityId, ListState};
use lens_core::domain::ids::{AccId, ItemId, ResponseId};
use lens_core::domain::item::{Item, ItemKind, MessageAcc, ReasoningAcc};
use lens_core::domain::scalars::Role;
use lens_core::reduce::ViewBlock;

use super::content_key::ContentKey;
use super::Marker;

/// Stable row identity — keyed store, not list index.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RowId {
    /// Finalize-stable section key: `(response_id, run_anchor)`.
    Section(ResponseId, ItemId),
    /// Work child inside a section (reasoning id, tool call item id, …).
    Work(ItemId),
    /// Top-level sibling (message, resource event, …).
    Sibling(ItemId),
    /// Live streaming tail keyed by accumulator id.
    StreamTail(AccId),
    /// Section rail row paired with `Section` chip — same `(response_id, run_anchor)`.
    SectionRail(ResponseId, ItemId),
    /// Reconnect-break marker (Task 14).
    Marker(u64),
    /// Top-of-list scroll-back sentinel (T-2b §5).
    LoadOlder,
}

/// Section identity — finalize-stable `(response_id, run_anchor)`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SectionKey {
    pub response_id: ResponseId,
    pub run_anchor: ItemId,
}

impl SectionKey {
    pub fn chip_id(&self) -> RowId {
        RowId::Section(self.response_id.clone(), self.run_anchor.clone())
    }

    pub fn rail_id(&self) -> RowId {
        RowId::SectionRail(self.response_id.clone(), self.run_anchor.clone())
    }
}

/// Typed row payload — markdown rendering consumes this in T3 Tasks 2–5.
#[derive(Clone, Debug, PartialEq)]
pub enum RowContent {
    Stub { text: String },
    AssistantMarkdown {
        source: String,
        content_key: ContentKey,
    },
    UserVerbatim { text: String },
    Reasoning {
        summary: String,
        full: String,
        encrypted: bool,
        duration_secs: Option<u32>,
        content_key: ContentKey,
        live: bool,
    },
}

impl RowContent {
    pub fn stub_text(&self) -> &str {
        match self {
            RowContent::Stub { text } => text.as_str(),
            RowContent::AssistantMarkdown { source, .. } => source.as_str(),
            RowContent::UserVerbatim { text } => text.as_str(),
            RowContent::Reasoning { full, .. } => full.as_str(),
        }
    }
}

/// Minimal owned presentation the stub renderer needs — not the whole `Item`.
#[derive(Clone, Debug, PartialEq)]
pub struct RowPresentation {
    pub kind: RowKind,
    pub content: RowContent,
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
    LoadOlder,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StructureEntry {
    Section(SectionKey),
    Sibling(RowId),
    #[allow(dead_code)] // constructed in Task 14 (ReconnectBreak markers)
    Marker(RowId),
    LoadOlder,
}

/// Owned child list for a work section (Level 2).
#[derive(Clone, Debug, Default)]
struct SectionNode {
    children: Vec<RowId>,
}

/// Retained `Entity`-per-row store keyed by `RowId`. Order is separate from identity
/// so virtualization recycle never remounts row state.
pub struct RowStore {
    pub(crate) order: Vec<RowId>,
    entities: HashMap<RowId, Entity<RowState>>,
    sections: HashMap<SectionKey, SectionNode>,
    pub(crate) structure: Vec<StructureEntry>,
    /// Per-`response_id` derived expand flag (all runs of a turn fold together).
    response_expanded: HashMap<ResponseId, bool>,
    /// Section a staged reasoning tail belonged to at `stage_stream_finalize` time —
    /// survives scratch-clear reprojection that drops reasoning-only sections.
    pending_tail_section: HashMap<AccId, SectionKey>,
}

impl RowStore {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            entities: HashMap::new(),
            sections: HashMap::new(),
            structure: Vec::new(),
            response_expanded: HashMap::new(),
            pending_tail_section: HashMap::new(),
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

    pub fn entity_id_at(&self, index: usize, cx: &App) -> Option<EntityId> {
        let id = self.order.get(index)?;
        self.entity_id(id, cx)
    }

    pub fn section_expanded(&self, response_id: &ResponseId) -> bool {
        self.response_expanded
            .get(response_id)
            .copied()
            .unwrap_or(true)
    }

    pub fn set_response_expanded(
        &mut self,
        response_id: &ResponseId,
        expanded: bool,
        list: Option<&ListState>,
    ) {
        let prev = self.section_expanded(response_id);
        if prev == expanded {
            return;
        }
        self.response_expanded.insert(response_id.clone(), expanded);
        let prev_len = self.order.len();
        self.rebuild_flat_order();
        if let Some(list) = list {
            self.sync_list_count(list, prev_len);
        }
    }

    pub fn set_all_response_expansion(
        &mut self,
        flags: &HashMap<ResponseId, bool>,
        list: Option<&ListState>,
    ) {
        let changed = flags
            .iter()
            .any(|(r, &exp)| self.section_expanded(r) != exp);
        if !changed {
            return;
        }
        for (r, &exp) in flags {
            self.response_expanded.insert(r.clone(), exp);
        }
        let prev_len = self.order.len();
        self.rebuild_flat_order();
        if let Some(list) = list {
            self.sync_list_count(list, prev_len);
        }
    }

    /// Id-keyed upsert: reuse the existing `Entity` when the `RowId` is present.
    pub fn upsert(&mut self, id: RowId, pres: RowPresentation, cx: &mut App) -> UpsertEffect {
        if let Some(entity) = self.entities.get_mut(&id) {
            entity.update(cx, |state, _| {
                state.id = id.clone();
                state.presentation = preserve_content_key(&state.presentation, pres);
            });
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

    pub fn sync_list_count(&self, list: &ListState, prev_len: usize) {
        let new_len = self.order.len();
        if prev_len != new_len {
            self.splice_into(list, 0..prev_len, new_len);
        }
    }

    /// Height-invalidate a content-mutated row (spike `invalidate_row_height`).
    pub fn invalidate_row_height(&self, list: &ListState, index: usize) {
        self.splice_into(list, index..index + 1, 1);
    }

    /// Replace the full projection from grouped `ViewBlock`s (baseline / reconcile path).
    pub fn materialize_full(blocks: &[ViewBlock<'_>], into: &mut RowStore, cx: &mut App) {
        into.structure.clear();
        into.sections.clear();
        for block in blocks {
            materialize_top_level(block, into, cx);
        }
        into.rebuild_flat_order();
    }

    /// Re-project only the tail blocks (live-section path) onto an existing prefix.
    pub fn materialize_live_tail(
        prefix_len: usize,
        blocks: &[ViewBlock<'_>],
        into: &mut RowStore,
        cx: &mut App,
    ) {
        into.structure
            .retain(|e| !matches!(e, StructureEntry::LoadOlder));
        into.structure.truncate(prefix_len);
        into.sections.retain(|key, _| {
            into.structure
                .iter()
                .any(|e| matches!(e, StructureEntry::Section(k) if k == key))
        });
        for block in blocks {
            materialize_top_level(block, into, cx);
        }
        into.rebuild_flat_order();
    }

    /// Stage a streaming tail for finalize — keep rendering under its section.
    pub fn stage_stream_finalize(
        &mut self,
        acc_id: &AccId,
        pres: RowPresentation,
        finalize_anchor: Option<ItemId>,
        response_id: Option<ResponseId>,
        cx: &mut App,
    ) -> Option<EntityId> {
        let id = RowId::StreamTail(acc_id.clone());
        // Capture the tail's own section before any reproject can drop it.
        if matches!(pres.kind, RowKind::StreamingReasoning) {
            if let Some(existing) = self.section_containing_child(&id).cloned() {
                if let (Some(anchor), Some(resp)) = (finalize_anchor.clone(), response_id.clone()) {
                    let target = SectionKey {
                        response_id: resp,
                        run_anchor: anchor,
                    };
                    if existing.run_anchor.as_str() == acc_id.as_str() {
                        self.rekey_section(&existing, &target, cx);
                        self.pending_tail_section.insert(acc_id.clone(), target);
                    } else {
                        self.pending_tail_section.insert(acc_id.clone(), existing);
                    }
                } else {
                    self.pending_tail_section.insert(acc_id.clone(), existing);
                }
            } else if let (Some(anchor), Some(resp)) = (finalize_anchor, response_id) {
                self.pending_tail_section.insert(
                    acc_id.clone(),
                    SectionKey {
                        response_id: resp,
                        run_anchor: anchor,
                    },
                );
            }
        }
        self.ensure_stream_tail_visible(acc_id, pres, cx);
        self.entity_id(&id, cx)
    }

    /// Keep a pending-finalize tail visible after scratch clears (structure + entity).
    pub fn ensure_stream_tail_visible(
        &mut self,
        acc_id: &AccId,
        pres: RowPresentation,
        cx: &mut App,
    ) {
        let id = RowId::StreamTail(acc_id.clone());
        self.upsert(id.clone(), pres.clone(), cx);
        let mut changed = false;
        match pres.kind {
            RowKind::StreamingMessage => {
                let absent = !self.structure.iter().any(|e| {
                    matches!(e, StructureEntry::Sibling(row) if row == &id)
                });
                if absent {
                    self.structure.push(StructureEntry::Sibling(id));
                    changed = true;
                }
            }
            RowKind::StreamingReasoning => {
                if self
                    .structure
                    .iter()
                    .any(|e| matches!(e, StructureEntry::Sibling(row) if row == &id))
                {
                    self.structure
                        .retain(|e| !matches!(e, StructureEntry::Sibling(row) if row == &id));
                    changed = true;
                }
                if self.section_containing_child(&id).is_none()
                    && let Some(key) = self.pending_tail_section.get(acc_id).cloned()
                {
                    if !self
                        .structure
                        .iter()
                        .any(|e| matches!(e, StructureEntry::Section(k) if k == &key))
                    {
                        self.structure.push(StructureEntry::Section(key.clone()));
                        changed = true;
                    }
                    let node = self.ensure_section_node(&key, cx);
                    if !node.children.contains(&id) {
                        node.children.push(id);
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        if changed {
            self.rebuild_flat_order();
        }
    }

    /// Drop a streaming tail immediately (`Discarded`).
    pub fn discard_stream_tail(&mut self, acc_id: &AccId, list: Option<&ListState>, cx: &mut App) {
        self.pending_tail_section.remove(acc_id);
        let id = RowId::StreamTail(acc_id.clone());
        for section in self.sections.values_mut() {
            section.children.retain(|c| c != &id);
        }
        self.structure.retain(|e| match e {
            StructureEntry::Sibling(row) | StructureEntry::Marker(row) => row != &id,
            StructureEntry::Section(_) | StructureEntry::LoadOlder => true,
        });
        self.entities.remove(&id);
        let prev_len = self.order.len();
        self.rebuild_flat_order();
        if let Some(list) = list {
            self.sync_list_count(list, prev_len);
        }
        let _ = cx;
    }

    /// Swap a staged stream tail to its durable row id in place (same `Entity`).
    pub fn commit_stream_finalize(
        &mut self,
        acc_id: &AccId,
        item_id: &ItemId,
        pres: RowPresentation,
        as_sibling: bool,
        list: Option<&ListState>,
        cx: &mut App,
    ) -> Option<EntityId> {
        self.pending_tail_section.remove(acc_id);
        let tail_id = RowId::StreamTail(acc_id.clone());
        let durable_id = if as_sibling {
            RowId::Sibling(item_id.clone())
        } else {
            RowId::Work(item_id.clone())
        };
        let entity = self.entities.remove(&tail_id)?;
        entity.update(cx, |state, _| {
            let stream_pres = state.presentation.clone();
            state.id = durable_id.clone();
            state.presentation = preserve_content_key(&stream_pres, pres);
        });
        self.entities.insert(durable_id.clone(), entity);
        let entity_id = self.entity_id(&durable_id, cx);
        for section in self.sections.values_mut() {
            if let Some(pos) = section.children.iter().position(|c| c == &tail_id) {
                section.children[pos] = durable_id.clone();
            }
        }
        if let Some(pos) = self.order.iter().position(|c| c == &tail_id) {
            self.order[pos] = durable_id.clone();
            if let Some(list) = list {
                self.invalidate_row_height(list, pos);
            }
        } else {
            let prev_len = self.order.len();
            self.rebuild_flat_order();
            if let Some(list) = list {
                self.sync_list_count(list, prev_len);
            }
        }
        if as_sibling {
            for entry in &mut self.structure {
                if let StructureEntry::Sibling(row) = entry
                    && *row == tail_id
                {
                    *row = durable_id.clone();
                }
            }
        }
        entity_id
    }

    pub fn structure_len(&self) -> usize {
        self.structure.len()
    }

    /// Remove marker entries from `structure` so live-tail truncation uses a
    /// marker-free prefix consistent with marker-exclusive `settled_structure_len`.
    pub(crate) fn strip_markers(&mut self) {
        self.structure
            .retain(|e| !matches!(e, StructureEntry::Marker(_)));
    }

    /// Re-insert reconnect markers into `structure` at their `after_ordinal` anchor,
    /// then rebuild the flat order. Deterministic across full reprojections.
    pub(crate) fn reinsert_markers(
        &mut self,
        markers: &[Marker],
        item_ordinals: &HashMap<ItemId, i64>,
        cx: &mut App,
    ) {
        if markers.is_empty() {
            return;
        }

        self.structure
            .retain(|e| !matches!(e, StructureEntry::Marker(_)));

        let reconnect_pres = RowPresentation {
            kind: RowKind::ReconnectBreak,
            content: RowContent::Stub {
                text: "reconnected".into(),
            },
            collapsed: false,
            height_hint: None,
        };

        let mut sorted: Vec<_> = markers.iter().collect();
        sorted.sort_by_key(|m| (m.after_ordinal, m.seq));

        for marker in sorted {
            let id = RowId::Marker(marker.seq);
            self.upsert(id.clone(), reconnect_pres.clone(), cx);

            let insert_at = self
                .structure
                .iter()
                .enumerate()
                .rev()
                .find_map(|(idx, entry)| {
                    let repr = entry_repr(entry, &self.sections, item_ordinals);
                    (repr != i64::MAX && repr <= marker.after_ordinal).then_some(idx)
                })
                .map(|idx| idx + 1)
                .unwrap_or(0);

            self.structure.insert(insert_at, StructureEntry::Marker(id));
        }

        self.rebuild_flat_order();
    }

    pub(crate) fn rebuild_flat_order(&mut self) {
        self.order.clear();
        for entry in &self.structure.clone() {
            match entry {
                StructureEntry::Section(key) => self.push_section_flat(key),
                StructureEntry::Sibling(id) | StructureEntry::Marker(id) => {
                    self.order.push(id.clone());
                }
                StructureEntry::LoadOlder => self.order.push(RowId::LoadOlder),
            }
        }
    }

    /// Drop `Entity` handles not referenced by the current projection (§6).
    pub(crate) fn gc_entities(&mut self, live_stream_tails: &HashSet<AccId>) {
        let mut retain = HashSet::new();
        for id in &self.order {
            retain.insert(id.clone());
        }
        for (key, node) in &self.sections {
            retain.insert(key.chip_id());
            retain.insert(key.rail_id());
            for child in &node.children {
                retain.insert(child.clone());
            }
        }
        for key in self.pending_tail_section.values() {
            retain.insert(key.chip_id());
            retain.insert(key.rail_id());
        }
        for acc in live_stream_tails {
            retain.insert(RowId::StreamTail(acc.clone()));
        }
        for entry in &self.structure {
            match entry {
                StructureEntry::Marker(id) => {
                    retain.insert(id.clone());
                }
                StructureEntry::LoadOlder => {
                    retain.insert(RowId::LoadOlder);
                }
                StructureEntry::Section(_) | StructureEntry::Sibling(_) => {}
            }
        }
        self.entities.retain(|id, _| retain.contains(id));
    }

    pub(crate) fn section_containing_child(&self, child: &RowId) -> Option<&SectionKey> {
        self.sections
            .iter()
            .find_map(|(key, node)| node.children.contains(child).then_some(key))
    }

    fn push_section_flat(&mut self, key: &SectionKey) {
        let expanded = self.section_expanded(&key.response_id);
        if expanded {
            self.order.push(key.rail_id());
            if let Some(node) = self.sections.get(key) {
                self.order.extend(node.children.iter().cloned());
            }
        } else {
            self.order.push(key.chip_id());
        }
    }

    fn move_row_entity(&mut self, from: &RowId, to: &RowId, cx: &mut App) {
        if from == to {
            return;
        }
        if let Some(entity) = self.entities.remove(from) {
            // Visible chrome lives on `from`; drop any placeholder at `to`.
            self.entities.remove(to);
            entity.update(cx, |state, _| {
                state.id = to.clone();
            });
            self.entities.insert(to.clone(), entity);
        }
    }

    fn rekey_section(&mut self, from: &SectionKey, to: &SectionKey, cx: &mut App) {
        if from == to {
            return;
        }
        if let Some(from_node) = self.sections.remove(from) {
            use std::collections::hash_map::Entry;
            match self.sections.entry(to.clone()) {
                Entry::Occupied(mut e) => {
                    for child in from_node.children {
                        if !e.get().children.contains(&child) {
                            e.get_mut().children.push(child);
                        }
                    }
                }
                Entry::Vacant(e) => {
                    e.insert(from_node);
                }
            }
        }
        for entry in &mut self.structure {
            if let StructureEntry::Section(key) = entry
                && key == from
            {
                *key = to.clone();
            }
        }
        let from_chip = from.chip_id();
        let from_rail = from.rail_id();
        let to_chip = to.chip_id();
        let to_rail = to.rail_id();
        self.move_row_entity(&from_chip, &to_chip, cx);
        self.move_row_entity(&from_rail, &to_rail, cx);
        for id in &mut self.order {
            if *id == from_chip {
                *id = to_chip.clone();
            } else if *id == from_rail {
                *id = to_rail.clone();
            }
        }
        self.ensure_section_node(to, cx);
        self.rebuild_flat_order();
    }

    fn ensure_section_node(&mut self, key: &SectionKey, cx: &mut App) -> &mut SectionNode {
        if !self.sections.contains_key(key) {
            let chip = RowPresentation {
                kind: RowKind::SectionChip,
                content: RowContent::Stub {
                    text: format!(
                        "section {} @{}",
                        key.response_id.as_str(),
                        key.run_anchor.as_str()
                    ),
                },
                collapsed: false,
                height_hint: None,
            };
            let rail = RowPresentation {
                kind: RowKind::SectionRail,
                content: RowContent::Stub {
                    text: format!(
                        "rail {} @{}",
                        key.response_id.as_str(),
                        key.run_anchor.as_str()
                    ),
                },
                collapsed: false,
                height_hint: None,
            };
            self.upsert(key.chip_id(), chip, cx);
            self.upsert(key.rail_id(), rail, cx);
            self.sections.insert(key.clone(), SectionNode::default());
        }
        self.sections.get_mut(key).expect("just inserted")
    }
}

impl Default for RowStore {
    fn default() -> Self {
        Self::new()
    }
}

fn child_ord(row_id: &RowId, item_ordinals: &HashMap<ItemId, i64>) -> i64 {
    match row_id {
        RowId::Work(id) | RowId::Sibling(id) => item_ordinals.get(id).copied().unwrap_or(i64::MAX),
        RowId::StreamTail(_) => i64::MAX,
        RowId::LoadOlder => i64::MIN,
        _ => i64::MIN,
    }
}

fn entry_repr(
    entry: &StructureEntry,
    sections: &HashMap<SectionKey, SectionNode>,
    item_ordinals: &HashMap<ItemId, i64>,
) -> i64 {
    match entry {
        StructureEntry::Section(key) => sections
            .get(key)
            .map(|node| {
                node.children
                    .iter()
                    .map(|c| child_ord(c, item_ordinals))
                    .max()
                    .unwrap_or(i64::MIN)
            })
            .unwrap_or(i64::MIN),
        StructureEntry::Sibling(id) => child_ord(id, item_ordinals),
        StructureEntry::Marker(_) | StructureEntry::LoadOlder => i64::MIN,
    }
}

fn materialize_top_level(block: &ViewBlock<'_>, into: &mut RowStore, cx: &mut App) {
    match block {
        ViewBlock::Item(item) => materialize_sibling_item(item, into, cx),
        ViewBlock::ToolSpan { call, output } => materialize_tool_span(call, *output, into, cx),
        ViewBlock::WorkSection {
            response_id,
            run_anchor,
            blocks,
        } => materialize_work_section(response_id, run_anchor.clone(), blocks, into, cx),
        ViewBlock::StreamingReasoning { acc, .. } => {
            materialize_streaming_reasoning(acc, into, cx);
        }
        ViewBlock::StreamingMessage(acc) => {
            materialize_streaming_message(acc, into, cx);
            into.structure
                .push(StructureEntry::Sibling(RowId::StreamTail(
                    acc.acc_id.clone(),
                )));
        }
    }
}

fn materialize_work_section(
    response_id: &ResponseId,
    run_anchor: ItemId,
    blocks: &[ViewBlock<'_>],
    into: &mut RowStore,
    cx: &mut App,
) {
    let key = SectionKey {
        response_id: response_id.clone(),
        run_anchor,
    };
    into.structure.push(StructureEntry::Section(key.clone()));
    into.ensure_section_node(&key, cx);
    let mut child_ids = Vec::new();
    for child in blocks {
        if let Some(id) = materialize_section_child_id(child, into, cx) {
            child_ids.push(id);
        }
    }
    if let Some(node) = into.sections.get_mut(&key) {
        node.children = child_ids;
    }
}

fn materialize_section_child_id(
    block: &ViewBlock<'_>,
    into: &mut RowStore,
    cx: &mut App,
) -> Option<RowId> {
    match block {
        ViewBlock::Item(item) => {
            materialize_work_item(item, into, cx);
            Some(RowId::Work(item.id.clone()))
        }
        ViewBlock::ToolSpan { call, output } => {
            materialize_tool_span(call, *output, into, cx);
            Some(RowId::Work(call.id.clone()))
        }
        ViewBlock::StreamingReasoning { acc, .. } => {
            materialize_streaming_reasoning(acc, into, cx);
            Some(RowId::StreamTail(acc.acc_id.clone()))
        }
        ViewBlock::WorkSection { .. } => None,
        ViewBlock::StreamingMessage(acc) => {
            materialize_streaming_message(acc, into, cx);
            Some(RowId::StreamTail(acc.acc_id.clone()))
        }
    }
}

fn materialize_sibling_item(item: &Item, into: &mut RowStore, cx: &mut App) {
    let id = RowId::Sibling(item.id.clone());
    let pres = presentation_for_item(item);
    into.upsert(id.clone(), pres, cx);
    into.structure.push(StructureEntry::Sibling(id));
}

fn materialize_work_item(item: &Item, into: &mut RowStore, cx: &mut App) {
    let id = RowId::Work(item.id.clone());
    let pres = presentation_for_work_item(item);
    into.upsert(id, pres, cx);
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
        content: RowContent::Stub { text },
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id, pres, cx);
}

fn materialize_streaming_reasoning(acc: &ReasoningAcc, into: &mut RowStore, cx: &mut App) {
    let id = RowId::StreamTail(acc.acc_id.clone());
    let content_key = ContentKey::from_acc(&acc.acc_id);
    let pres = RowPresentation {
        kind: RowKind::StreamingReasoning,
        content: RowContent::Reasoning {
            summary: acc.summary_text.clone(),
            full: acc.full_text.clone(),
            encrypted: acc.encrypted,
            duration_secs: None,
            content_key,
            live: true,
        },
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id, pres, cx);
}

fn materialize_streaming_message(acc: &MessageAcc, into: &mut RowStore, cx: &mut App) {
    let content_key = ContentKey::from_acc(&acc.acc_id);
    let source = crate::md::safe_prefix(&acc.text);
    let id = RowId::StreamTail(acc.acc_id.clone());
    let pres = RowPresentation {
        kind: RowKind::StreamingMessage,
        content: RowContent::AssistantMarkdown {
            source,
            content_key,
        },
        collapsed: false,
        height_hint: None,
    };
    into.upsert(id, pres, cx);
}

fn sibling_row_kind(item: &Item) -> RowKind {
    match &item.kind {
        ItemKind::Message { role, .. } if *role == Role::User => RowKind::UserMessage,
        ItemKind::Message { .. } => RowKind::Message,
        ItemKind::ResourceEvent { .. } => RowKind::ResourceEvent,
        _ => RowKind::Message,
    }
}

pub(crate) fn item_text_stub(item: &Item) -> String {
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

pub(crate) fn row_content_for_item(item: &Item) -> RowContent {
    match &item.kind {
        ItemKind::Message {
            role: Role::User,
            content,
            ..
        } => RowContent::UserVerbatim {
            text: content
                .iter()
                .filter_map(|block| block.text.as_deref())
                .collect::<Vec<_>>()
                .join(""),
        },
        ItemKind::Message { content, .. } => RowContent::AssistantMarkdown {
            source: crate::md::safe_prefix(
                &content
                    .iter()
                    .filter_map(|block| block.text.as_deref())
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            content_key: ContentKey::from_acc(&AccId::new(item.id.as_str())),
        },
        ItemKind::Reasoning {
            full_text,
            summary_text,
            encrypted,
            duration_ms,
        } => RowContent::Reasoning {
            summary: summary_text.clone(),
            full: full_text.clone(),
            encrypted: *encrypted,
            duration_secs: duration_ms.map(|ms| (ms / 1000) as u32),
            content_key: ContentKey::from_acc(&AccId::new(item.id.as_str())),
            live: false,
        },
        _ => RowContent::Stub {
            text: item_text_stub(item),
        },
    }
}

pub(crate) fn presentation_for_item(item: &Item) -> RowPresentation {
    RowPresentation {
        kind: sibling_row_kind(item),
        content: row_content_for_item(item),
        collapsed: false,
        height_hint: None,
    }
}

pub(crate) fn presentation_for_work_item(item: &Item) -> RowPresentation {
    RowPresentation {
        kind: RowKind::WorkChild,
        content: row_content_for_item(item),
        collapsed: false,
        height_hint: None,
    }
}

/// D11: streaming→finalize keeps the stream `content_key`; never retarget to item id.
fn preserve_content_key(stream_pres: &RowPresentation, mut pres: RowPresentation) -> RowPresentation {
    match (&stream_pres.content, &mut pres.content) {
        (
            RowContent::AssistantMarkdown { content_key: src, .. },
            RowContent::AssistantMarkdown { content_key: dst, .. },
        ) => *dst = src.clone(),
        (
            RowContent::Reasoning { content_key: src, .. },
            RowContent::Reasoning { content_key: dst, .. },
        ) => *dst = src.clone(),
        _ => {}
    }
    pres
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
                duration_ms: None,
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

    fn assistant_msg(id: &str, resp: Option<&str>, text: &str) -> Item {
        item(
            id,
            resp,
            ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        )
    }

    #[test]
    fn finalized_assistant_message_uses_safe_prefix_pipeline() {
        let item = assistant_msg("msg1", Some("resp"), "**bo");
        match row_content_for_item(&item) {
            RowContent::AssistantMarkdown { source, .. } => {
                assert_eq!(source, "**bo**");
            }
            _ => panic!("expected AssistantMarkdown"),
        }
    }

    #[gpui::test]
    fn run_anchor_stable_under_truncation(cx: &mut gpui::TestAppContext) {
        let resp_a = ResponseId::new("resp_a");
        let later_only = [reasoning("r2", Some("resp_a"))];
        let later_refs: Vec<&Item> = later_only.iter().collect();
        let scratch = lens_core::domain::item::StreamScratch::default();
        let later_blocks = group_work_section(project(&later_refs, &scratch, None), None);

        let mut store = RowStore::new();
        cx.update(|cx| RowStore::materialize_full(&later_blocks, &mut store, cx));
        let upper_key = SectionKey {
            response_id: resp_a.clone(),
            run_anchor: ItemId::new("r2"),
        };
        let entity_before = cx.read(|cx| {
            store
                .entity_id(&upper_key.rail_id(), cx)
                .or_else(|| store.entity_id(&upper_key.chip_id(), cx))
                .expect("section row")
        });

        let full = [
            reasoning("r1", Some("resp_a")),
            assistant_msg("a1", Some("resp_a"), "narration"),
            reasoning("r2", Some("resp_a")),
        ];
        let full_refs: Vec<&Item> = full.iter().collect();
        let full_blocks = group_work_section(project(&full_refs, &scratch, None), None);

        cx.update(|cx| RowStore::materialize_full(&full_blocks, &mut store, cx));
        let entity_after = cx.read(|cx| {
            store
                .entity_id(&upper_key.rail_id(), cx)
                .or_else(|| store.entity_id(&upper_key.chip_id(), cx))
                .expect("section row")
        });

        assert_eq!(
            entity_before, entity_after,
            "run_anchor-keyed section must not churn when older history is prepended"
        );
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
        cx.update(|cx| RowStore::materialize_full(&blocks, &mut store, cx));

        let kinds: Vec<RowKind> = cx.read(|cx| {
            (0..store.len())
                .map(|ix| store.kind_at(ix, cx).expect("kind"))
                .collect()
        });
        assert_eq!(
            kinds,
            vec![
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
                RowId::Section(resp, anchor) => Some((resp.as_str(), anchor.as_str())),
                _ => None,
            })
            .collect();
        assert!(
            section_ids.is_empty(),
            "expanded section shows rail not chip"
        );

        assert!(matches!(
            store.order().last(),
            Some(RowId::StreamTail(acc)) if acc.as_str() == "acc_stream_m"
        ));
    }

    #[gpui::test]
    fn collapse_splices_rail_to_chip(cx: &mut gpui::TestAppContext) {
        let resp_a = ResponseId::new("resp_a");
        let items = [reasoning("r1", Some("resp_a"))];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = lens_core::domain::item::StreamScratch::default();
        let projected = project(&refs, &scratch, Some(&resp_a));
        let blocks = group_work_section(projected, Some(&resp_a));

        let mut store = RowStore::new();
        let list = ListState::new(0, gpui::ListAlignment::Bottom, gpui::px(0.));
        cx.update(|cx| RowStore::materialize_full(&blocks, &mut store, cx));
        list.reset(store.len());
        assert_eq!(store.len(), 2);

        store.set_response_expanded(&resp_a, false, Some(&list));
        assert_eq!(store.len(), 1);
        cx.read(|cx| {
            assert_eq!(store.kind_at(0, cx), Some(RowKind::SectionChip));
        });

        store.set_response_expanded(&resp_a, true, Some(&list));
        assert_eq!(store.len(), 2);
    }

    #[gpui::test]
    fn collapsed_child_entity_survives_gc_and_expand(cx: &mut gpui::TestAppContext) {
        let resp_a = ResponseId::new("resp_a");
        let items = [
            reasoning("r1", Some("resp_a")),
            call("c1", Some("resp_a"), "call_1"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = lens_core::domain::item::StreamScratch::default();
        let projected = project(&refs, &scratch, Some(&resp_a));
        let blocks = group_work_section(projected, Some(&resp_a));

        let mut store = RowStore::new();
        cx.update(|cx| RowStore::materialize_full(&blocks, &mut store, cx));

        let child_id = RowId::Work(ItemId::new("c1"));
        let child_entity_before = cx.read(|cx| store.entity_id(&child_id, cx).expect("child"));

        store.set_response_expanded(&resp_a, false, None);
        store.gc_entities(&HashSet::new());

        let child_entity_collapsed =
            cx.read(|cx| store.entity_id(&child_id, cx).expect("child after gc"));
        assert_eq!(
            child_entity_before, child_entity_collapsed,
            "collapsed children must survive gc_entities"
        );

        store.set_response_expanded(&resp_a, true, None);
        let child_entity_expanded =
            cx.read(|cx| store.entity_id(&child_id, cx).expect("child after expand"));
        assert_eq!(
            child_entity_before, child_entity_expanded,
            "expand must reuse the same EntityId after gc"
        );
    }

    #[gpui::test]
    fn pending_finalize_stream_tail_survives_gc(cx: &mut gpui::TestAppContext) {
        let acc = AccId::new("acc_pending_gc");
        let mut store = RowStore::new();
        let entity_before = cx.update(|cx| {
            store.stage_stream_finalize(
                &acc,
                RowPresentation {
                    kind: RowKind::StreamingMessage,
                    content: RowContent::AssistantMarkdown {
                        source: "staged".into(),
                        content_key: ContentKey::from_acc(&acc),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            )
        });
        let live = HashSet::from([acc.clone()]);
        store.gc_entities(&live);
        let entity_after = cx.read(|cx| {
            store
                .entity_id(&RowId::StreamTail(acc), cx)
                .expect("pending tail entity")
        });
        assert_eq!(
            entity_before,
            Some(entity_after),
            "mid-finalize StreamTail must survive gc via live_stream_tails"
        );
    }

    /// Pre-fix rail keys used `RowId::Marker(0x8000… | hash)`; reconnect markers share that namespace.
    fn legacy_section_rail_marker_seq(response_id: &ResponseId, run_anchor: &ItemId) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in response_id.as_str().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        for byte in run_anchor.as_str().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        0x8000_0000_0000_0000 ^ hash
    }

    #[gpui::test]
    fn section_rail_and_marker_do_not_alias(cx: &mut gpui::TestAppContext) {
        let resp_a = ResponseId::new("resp_a");
        let items = [reasoning("r1", Some("resp_a"))];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = lens_core::domain::item::StreamScratch::default();
        let projected = project(&refs, &scratch, Some(&resp_a));
        let blocks = group_work_section(projected, Some(&resp_a));

        let mut store = RowStore::new();
        cx.update(|cx| RowStore::materialize_full(&blocks, &mut store, cx));

        let legacy_seq = legacy_section_rail_marker_seq(&resp_a, &ItemId::new("r1"));
        let marker_id = RowId::Marker(legacy_seq);
        let rail_id = RowId::SectionRail(resp_a.clone(), ItemId::new("r1"));
        assert_ne!(
            rail_id, marker_id,
            "rail and legacy-marker seq must be distinct ids"
        );

        cx.update(|cx| {
            store.upsert(
                marker_id.clone(),
                RowPresentation {
                    kind: RowKind::ReconnectBreak,
                    content: RowContent::Stub {
                        text: "reconnect".into(),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                cx,
            );
            store
                .structure
                .push(StructureEntry::Marker(marker_id.clone()));
            store.rebuild_flat_order();
        });

        let rail_in_order = store.order().iter().filter(|id| **id == rail_id).count();
        let marker_in_order = store.order().iter().filter(|id| **id == marker_id).count();
        assert_eq!(
            rail_in_order, 1,
            "rail id must appear exactly once in order"
        );
        assert_eq!(
            marker_in_order, 1,
            "marker id must appear exactly once in order"
        );

        let (rail_entity, marker_entity) = cx.read(|cx| {
            (
                store.entity_id(&rail_id, cx).expect("rail entity"),
                store.entity_id(&marker_id, cx).expect("marker entity"),
            )
        });
        assert_ne!(
            rail_entity, marker_entity,
            "distinct RowIds must map to distinct entities (no HashMap aliasing)"
        );
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
                    content: RowContent::Stub {
                        text: "v1".into(),
                    },
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
                    content: RowContent::Stub {
                        text: "v2".into(),
                    },
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
        });
    }

    #[gpui::test]
    fn commit_finalize_preserves_entity_id(cx: &mut gpui::TestAppContext) {
        let acc = AccId::new("acc_1");
        let item_id = ItemId::new("msg_local_0");
        let mut store = RowStore::new();
        let before = cx.update(|cx| {
            store.stage_stream_finalize(
                &acc,
                RowPresentation {
                    kind: RowKind::StreamingMessage,
                    content: RowContent::AssistantMarkdown {
                        source: "streaming".into(),
                        content_key: ContentKey::from_acc(&acc),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            )
        });
        let after = cx.update(|cx| {
            store.commit_stream_finalize(
                &acc,
                &item_id,
                RowPresentation {
                    kind: RowKind::Message,
                    content: RowContent::AssistantMarkdown {
                        source: "final".into(),
                        content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                true,
                None,
                cx,
            )
        });
        assert_eq!(before, after, "finalize must not recreate entity");
        cx.read(|cx| {
            let id = RowId::Sibling(item_id);
            assert_eq!(
                store.entity(&id).unwrap().read(cx).presentation.content.stub_text(),
                "final"
            );
        });
    }

    #[gpui::test]
    fn finalize_preserves_content_key(cx: &mut gpui::TestAppContext) {
        let acc = AccId::new("acc_stream_1");
        let key = ContentKey::from_acc(&acc);
        let item_id = ItemId::new("item_1");
        let mut store = RowStore::new();
        cx.update(|cx| {
            store.stage_stream_finalize(
                &acc,
                RowPresentation {
                    kind: RowKind::StreamingMessage,
                    content: RowContent::AssistantMarkdown {
                        source: "hello".into(),
                        content_key: key.clone(),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            );
            store.commit_stream_finalize(
                &acc,
                &item_id,
                RowPresentation {
                    kind: RowKind::Message,
                    content: RowContent::AssistantMarkdown {
                        source: "hello world".into(),
                        // Naive implementers would retarget to item id here — forbidden.
                        content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                true,
                None,
                cx,
            );
        });
        cx.read(|cx| {
            let id = RowId::Sibling(item_id);
            let final_pres = &store.entity(&id).unwrap().read(cx).presentation;
            match &final_pres.content {
                RowContent::AssistantMarkdown { source, content_key } => {
                    assert_eq!(source, "hello world");
                    assert_eq!(content_key, &key, "D11: content_key must stay the stream key");
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[gpui::test]
    fn finalize_preserves_content_key_reasoning(cx: &mut gpui::TestAppContext) {
        let acc = AccId::new("acc_reason_1");
        let key = ContentKey::from_acc(&acc);
        let item_id = ItemId::new("reason_1");
        let mut store = RowStore::new();
        cx.update(|cx| {
            store.stage_stream_finalize(
                &acc,
                RowPresentation {
                    kind: RowKind::StreamingReasoning,
                    content: RowContent::Reasoning {
                        summary: "thinking".into(),
                        full: "thinking aloud".into(),
                        encrypted: false,
                        duration_secs: None,
                        content_key: key.clone(),
                        live: true,
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            );
            store.commit_stream_finalize(
                &acc,
                &item_id,
                RowPresentation {
                    kind: RowKind::WorkChild,
                    content: RowContent::Reasoning {
                        summary: "done".into(),
                        full: "thinking aloud, finalized".into(),
                        encrypted: false,
                        duration_secs: Some(4),
                        content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                        live: false,
                    },
                    collapsed: false,
                    height_hint: None,
                },
                false,
                None,
                cx,
            );
        });
        cx.read(|cx| {
            let id = RowId::Work(item_id);
            let final_pres = &store.entity(&id).unwrap().read(cx).presentation;
            match &final_pres.content {
                RowContent::Reasoning {
                    content_key,
                    live,
                    duration_secs,
                    ..
                } => {
                    assert_eq!(content_key, &key, "D11: reasoning content_key must stay the stream key");
                    assert!(!live);
                    assert_eq!(*duration_secs, Some(4));
                }
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[gpui::test]
    fn upsert_preserves_content_key_across_reproject(cx: &mut gpui::TestAppContext) {
        let acc = AccId::new("acc_stream_1");
        let stream_key = ContentKey::from_acc(&acc);
        let item_id = ItemId::new("item_1");
        let mut store = RowStore::new();
        cx.update(|cx| {
            store.stage_stream_finalize(
                &acc,
                RowPresentation {
                    kind: RowKind::StreamingMessage,
                    content: RowContent::AssistantMarkdown {
                        source: "hi".into(),
                        content_key: stream_key.clone(),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            );
            store.commit_stream_finalize(
                &acc,
                &item_id,
                RowPresentation {
                    kind: RowKind::Message,
                    content: RowContent::AssistantMarkdown {
                        source: "hi there".into(),
                        content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                true,
                None,
                cx,
            );
            // Simulate the post-commit reproject re-materializing the durable row with an item-id key.
            store.upsert(
                RowId::Sibling(item_id.clone()),
                RowPresentation {
                    kind: RowKind::Message,
                    content: RowContent::AssistantMarkdown {
                        source: "hi there".into(),
                        content_key: ContentKey::from_acc(&AccId::new(item_id.as_str())),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                cx,
            );
        });
        cx.read(|cx| {
            let content = store
                .entity(&RowId::Sibling(item_id))
                .unwrap()
                .read(cx)
                .presentation
                .content
                .clone();
            match content {
                RowContent::AssistantMarkdown { content_key, .. } => assert_eq!(
                    content_key, stream_key,
                    "D11: reproject upsert must not clobber the stream key"
                ),
                other => panic!("unexpected {other:?}"),
            }
        });
    }

    #[test]
    fn finalized_reasoning_projects_duration_secs() {
        let item = item(
            "r1",
            Some("resp_a"),
            ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: "t".into(),
                encrypted: false,
                duration_ms: Some(4_500),
            },
        );
        let pres = presentation_for_work_item(&item);
        match pres.content {
            RowContent::Reasoning {
                duration_secs,
                live,
                ..
            } => {
                assert_eq!(duration_secs, Some(4));
                assert!(!live);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
