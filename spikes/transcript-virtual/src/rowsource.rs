//! `RowSource` seam + retained id-keyed row store (spec §2).

use std::collections::HashMap;

use gpui::{App, AppContext, Entity, EntityId, Pixels, Window};

use crate::fixture::{FixtureRow, RowKind};

/// Stable item identity — keyed store, not list index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RowId(pub u64);

/// Per-row retained state. Backends render a handle into this store.
#[derive(Clone)]
pub struct RowState {
    pub id: RowId,
    pub kind: RowKind,
    pub text: String,
    pub height_delta: Pixels,
    pub use_markdown: bool,
    pub markdown_initialized: bool,
    pub markdown_init_count: u32,
    pub measured_height: Option<Pixels>,
}

impl RowState {
    fn from_fixture(row: &FixtureRow) -> Self {
        Self {
            id: RowId(row.id),
            kind: row.kind,
            text: row.text.clone(),
            height_delta: row.height_delta,
            use_markdown: row.kind == RowKind::CodeBlock,
            markdown_initialized: false,
            markdown_init_count: 0,
            measured_height: None,
        }
    }
}

/// How a live scratch row is settled into the retained disk window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffMode {
    /// Re-ingest by id, reusing existing `Entity<RowState>` handles.
    UpsertById,
    /// Clear+recreate entities for the same ids (negative control).
    ClearRecreate,
}

/// Retained `Entity`-per-item store keyed by item id. Order is separate from
/// identity so virtualization recycle never remounts row state.
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

    pub fn id_at(&self, index: usize) -> Option<RowId> {
        self.order.get(index).copied()
    }

    pub fn entity(&self, id: RowId) -> Option<&Entity<RowState>> {
        self.entities.get(&id)
    }

    pub fn entity_mut(&mut self, id: RowId) -> Option<&mut Entity<RowState>> {
        self.entities.get_mut(&id)
    }

    /// Ingest fixture rows, creating retained entities for each id.
    pub fn load_fixture(&mut self, rows: &[FixtureRow], cx: &mut App) {
        self.order.clear();
        self.entities.clear();
        for row in rows {
            let id = RowId(row.id);
            let entity = cx.new(|_| RowState::from_fixture(row));
            self.order.push(id);
            self.entities.insert(id, entity);
        }
    }

    /// Append text to the last item (contract 1a streaming hook).
    pub fn append_to_last(&mut self, chunk: &str, cx: &mut App) {
        let Some(&id) = self.order.last() else {
            return;
        };
        if let Some(entity) = self.entities.get_mut(&id) {
            entity.update(cx, |state, _| state.text.push_str(chunk));
        }
    }

    /// Mutate height of a specific item by id (contract 1b hook).
    pub fn mutate_height(&mut self, id: RowId, delta: Pixels, cx: &mut App) {
        if let Some(entity) = self.entities.get_mut(&id) {
            entity.update(cx, |state, _| state.height_delta += delta);
        }
    }

    pub fn entity_id(&self, id: RowId, _cx: &App) -> Option<EntityId> {
        self.entities.get(&id).map(|entity| entity.entity_id())
    }

    pub fn markdown_init_count(&self, id: RowId, cx: &App) -> u32 {
        self.entities
            .get(&id)
            .map(|entity| entity.read(cx).markdown_init_count)
            .unwrap_or(0)
    }

    fn next_scratch_id(&self) -> RowId {
        let max = self.order.iter().map(|id| id.0).max().unwrap_or(0);
        RowId(max + 1)
    }

    fn append_scratch_row(&mut self, cx: &mut App) -> RowId {
        let id = self.next_scratch_id();
        let entity = cx.new(|_| RowState {
            id,
            kind: RowKind::OneLiner,
            text: String::new(),
            height_delta: gpui::px(0.),
            use_markdown: false,
            markdown_initialized: false,
            markdown_init_count: 0,
            measured_height: None,
        });
        self.order.push(id);
        self.entities.insert(id, entity);
        id
    }

    /// Settle the live scratch row into the disk window, then open a fresh scratch tail.
    pub fn finalize_handoff(&mut self, mode: HandoffMode, cx: &mut App) -> RowId {
        let finalized = *self
            .order
            .last()
            .expect("finalize_handoff needs at least one row");

        match mode {
            HandoffMode::UpsertById => {
                for &id in &self.order.clone() {
                    if let Some(entity) = self.entities.get(&id) {
                        let text = entity.read(cx).text.clone();
                        entity.update(cx, |state, _| state.text = text);
                    }
                }
            }
            HandoffMode::ClearRecreate => {
                let snapshots = self
                    .order
                    .iter()
                    .filter_map(|&id| {
                        self.entities
                            .get(&id)
                            .map(|entity| (id, entity.read(cx).clone()))
                    })
                    .collect::<Vec<_>>();
                self.entities.clear();
                for (id, state) in snapshots {
                    let entity = cx.new(|_| state);
                    self.entities.insert(id, entity);
                }
            }
        }

        self.append_scratch_row(cx);
        finalized
    }
}

impl Default for RowStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Backend-agnostic virtualizer seam. Phase 1/2 bind native `list()` and
/// gpui-component `List` behind this trait.
pub trait RowSource {
    fn item_count(&self, cx: &App) -> usize;

    fn row_id_at(&self, index: usize, cx: &App) -> Option<RowId>;

    fn row_entity(&self, id: RowId, cx: &App) -> Option<Entity<RowState>>;

    fn append_to_last(&mut self, chunk: &str, window: &mut Window, cx: &mut App);

    fn mutate_height(&mut self, id: RowId, delta: Pixels, window: &mut Window, cx: &mut App);
}

/// Harness-side row model: store only (no virtualizer binding yet).
pub struct ModelRowSource {
    pub store: RowStore,
}

impl ModelRowSource {
    pub fn new() -> Self {
        Self {
            store: RowStore::new(),
        }
    }
}

impl RowSource for ModelRowSource {
    fn item_count(&self, _cx: &App) -> usize {
        self.store.len()
    }

    fn row_id_at(&self, index: usize, _cx: &App) -> Option<RowId> {
        self.store.id_at(index)
    }

    fn row_entity(&self, id: RowId, _cx: &App) -> Option<Entity<RowState>> {
        self.store.entity(id).cloned()
    }

    fn append_to_last(&mut self, chunk: &str, _window: &mut Window, cx: &mut App) {
        self.store.append_to_last(chunk, cx);
    }

    fn mutate_height(&mut self, id: RowId, delta: Pixels, _window: &mut Window, cx: &mut App) {
        self.store.mutate_height(id, delta, cx);
    }
}

impl Default for ModelRowSource {
    fn default() -> Self {
        Self::new()
    }
}
