//! Backend A — gpui native `list()` + `ListState` + `ListAlignment::Bottom`.

use gpui::{App, EntityId, ListAlignment, ListState, Pixels, Window, div, prelude::*, px};

use crate::fixture::Fixture;
use crate::row_render::render_row;
use crate::rowsource::{HandoffMode, RowId, RowSource, RowStore};

const OVERDRAW: Pixels = px(200.);

pub struct BackendA {
    pub fixture: Fixture,
    pub store: RowStore,
    pub list_state: ListState,
}

impl BackendA {
    pub fn new(n: usize, cx: &mut App) -> Self {
        let fixture = Fixture::synthetic(n);
        let mut store = RowStore::new();
        store.load_fixture(&fixture.rows, cx);
        let list_state = ListState::new(n, ListAlignment::Bottom, OVERDRAW);
        Self {
            fixture,
            store,
            list_state,
        }
    }

    pub fn new_handoff(n: usize, cx: &mut App) -> Self {
        let fixture = Fixture::handoff_scripted(n);
        let mut store = RowStore::new();
        store.load_fixture(&fixture.rows, cx);
        let list_state = ListState::new(n, ListAlignment::Bottom, OVERDRAW);
        Self {
            fixture,
            store,
            list_state,
        }
    }

    pub fn reload_handoff(&mut self, n: usize, cx: &mut App) {
        self.fixture = Fixture::handoff_scripted(n);
        self.store.load_fixture(&self.fixture.rows, cx);
        self.list_state.reset(n);
    }

    pub fn finalize_handoff(&mut self, mode: HandoffMode, cx: &mut App) -> RowId {
        let finalized = self.store.finalize_handoff(mode, cx);
        self.list_state.reset(self.store.len());
        finalized
    }

    pub fn reload(&mut self, n: usize, cx: &mut App) {
        self.fixture = Fixture::synthetic(n);
        self.store.load_fixture(&self.fixture.rows, cx);
        self.list_state.reset(n);
    }

    pub fn item_count(&self) -> usize {
        self.store.len()
    }

    pub fn scroll_anchor_ix(&self) -> usize {
        self.item_count() / 2
    }

    pub fn invalidate_row_height(&self, index: usize) {
        self.list_state.splice(index..index + 1, 1);
    }

    pub fn invalidate_last_row(&self) {
        let last = self.item_count().saturating_sub(1);
        if last < self.item_count() {
            self.list_state.splice(last..last + 1, 1);
        }
    }

    pub fn render_list_item(
        &self,
        ix: usize,
        window: &mut Window,
        cx: &mut App,
        render_counter: &mut u32,
    ) -> gpui::AnyElement {
        *render_counter += 1;
        let Some(id) = self.store.id_at(ix) else {
            return div().into_any_element();
        };
        let Some(entity) = self.store.entity(id) else {
            return div().into_any_element();
        };
        entity.update(cx, |state, cx| render_row(state, window, cx))
    }

    pub fn identity_target_entity_id(&self, ix: usize, _cx: &App) -> Option<EntityId> {
        let id = self.store.id_at(ix)?;
        let entity = self.store.entity(id)?;
        Some(entity.entity_id())
    }

    pub fn identity_markdown_inits(&self, ix: usize, cx: &App) -> u32 {
        let Some(id) = self.store.id_at(ix) else {
            return 0;
        };
        let Some(entity) = self.store.entity(id) else {
            return 0;
        };
        entity.read(cx).markdown_init_count
    }
}

impl RowSource for BackendA {
    fn item_count(&self, _cx: &App) -> usize {
        self.store.len()
    }

    fn row_id_at(&self, index: usize, _cx: &App) -> Option<RowId> {
        self.store.id_at(index)
    }

    fn row_entity(&self, id: RowId, _cx: &App) -> Option<gpui::Entity<crate::rowsource::RowState>> {
        self.store.entity(id).cloned()
    }

    fn append_to_last(&mut self, chunk: &str, _window: &mut Window, cx: &mut App) {
        self.fixture.append_to_last(chunk);
        self.store.append_to_last(chunk, cx);
        self.invalidate_last_row();
    }

    fn mutate_height(&mut self, id: RowId, delta: Pixels, _window: &mut Window, cx: &mut App) {
        self.fixture.mutate_offscreen_height(delta);
        self.store.mutate_height(id, delta, cx);
        if let Some(ix) = self.store.order.iter().position(|&rid| rid == id) {
            self.invalidate_row_height(ix);
        }
    }
}
