//! `RowSource` seam + retained id-keyed row store (spec §2).

use std::collections::HashMap;

use gpui::{App, AppContext, Entity, Pixels, Window};

use crate::fixture::{FixtureRow, RowKind};

/// Stable item identity — keyed store, not list index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RowId(pub u64);

/// Per-row retained state. Backends render a handle into this store.
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
