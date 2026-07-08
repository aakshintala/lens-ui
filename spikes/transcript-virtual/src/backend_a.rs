//! Backend A — gpui native `list()` + `ListState` + `ListAlignment::Bottom`.

use gpui::{
    div, prelude::*, App, Entity, EntityId, ListAlignment, ListState, Pixels, SharedString, Window,
    px,
};
use gpui_component::text::TextView;

use crate::fixture::{Fixture, RowKind};
use crate::rowsource::{RowId, RowSource, RowState, RowStore};

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

    pub fn reload(&mut self, n: usize, cx: &mut App) {
        self.fixture = Fixture::synthetic(n);
        self.store.load_fixture(&self.fixture.rows, cx);
        self.list_state.reset(n);
    }

    pub fn item_count(&self) -> usize {
        self.store.len()
    }

    pub fn mutable_offscreen_ix(&self) -> usize {
        self.fixture.mutable_offscreen_id as usize
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

        entity.update(cx, |state, cx| {
            if state.use_markdown && !state.markdown_initialized {
                state.markdown_initialized = true;
                state.markdown_init_count += 1;
            }
            render_row(state, window, cx)
        })
    }

    pub fn identity_target_entity_id(&self, ix: usize, cx: &App) -> Option<EntityId> {
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

    fn row_entity(&self, id: RowId, cx: &App) -> Option<Entity<RowState>> {
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
        if let Some(ix) = self
            .store
            .order
            .iter()
            .position(|&rid| rid == id)
        {
            self.invalidate_row_height(ix);
        }
    }
}

fn render_row(state: &mut RowState, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let pad = px(4.) + state.height_delta;
    match state.kind {
        RowKind::CodeBlock => div()
            .w_full()
            .pb(pad)
            .child(
                TextView::markdown(
                    SharedString::from(format!("md-{}", state.id.0)),
                    state.text.clone(),
                    window,
                    cx,
                )
                .selectable(true)
                .scrollable(false),
            )
            .into_any_element(),
        RowKind::ImagePlaceholder => div()
            .w_full()
            .h(px(120.) + state.height_delta)
            .p_2()
            .child(state.text.clone())
            .into_any_element(),
        RowKind::ToolSpan => div()
            .w_full()
            .pb(pad)
            .p_2()
            .child(format!("{}\n(extra tool output line)\n(more output)", state.text))
            .into_any_element(),
        RowKind::OneLiner => div()
            .w_full()
            .pb(pad)
            .px_2()
            .child(state.text.clone())
            .into_any_element(),
    }
}
