//! Backend B — gpui-component `v_virtual_list` + `VirtualListScrollHandle`.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Entity, EntityId, Pixels, Size, Window, div, point, prelude::*, px, size};
use gpui_component::VirtualListScrollHandle;

use crate::anchor::{derive_anchor, scroll_y_for_anchor};
use crate::fixture::Fixture;
use crate::probe::AnchorSnapshot;
use crate::row_render::{estimated_height, render_row};
use crate::rowsource::{RowId, RowSource, RowState, RowStore};

pub struct BackendB {
    pub fixture: Fixture,
    pub store: RowStore,
    pub scroll_handle: VirtualListScrollHandle,
    pub item_heights: Rc<RefCell<Vec<Pixels>>>,
    pub item_sizes: Rc<Vec<Size<Pixels>>>,
    pub follow_bottom: bool,
    pub pending_scroll_bottom: bool,
    pub anchor_1b_derived: bool,
}

impl BackendB {
    pub fn new(n: usize, cx: &mut App) -> Self {
        let fixture = Fixture::synthetic(n);
        let mut store = RowStore::new();
        store.load_fixture(&fixture.rows, cx);
        let heights = Self::heights_from_store(&store, cx);
        let item_sizes = Self::sizes_from_heights(&heights);
        Self {
            fixture,
            store,
            scroll_handle: VirtualListScrollHandle::new(),
            item_heights: Rc::new(RefCell::new(heights)),
            item_sizes,
            follow_bottom: true,
            pending_scroll_bottom: true,
            anchor_1b_derived: true,
        }
    }

    pub fn reload(&mut self, n: usize, cx: &mut App) {
        self.fixture = Fixture::synthetic(n);
        self.store.load_fixture(&self.fixture.rows, cx);
        self.resync_heights(cx);
        self.pending_scroll_bottom = true;
        self.follow_bottom = true;
    }

    fn heights_from_store(store: &RowStore, cx: &App) -> Vec<Pixels> {
        store
            .order
            .iter()
            .filter_map(|id| store.entity(*id).map(|e| estimated_height(e.read(cx))))
            .collect()
    }

    fn sizes_from_heights(heights: &[Pixels]) -> Rc<Vec<Size<Pixels>>> {
        Rc::new(heights.iter().map(|&h| size(px(0.), h)).collect::<Vec<_>>())
    }

    fn resync_heights(&mut self, cx: &mut App) {
        let heights = Self::heights_from_store(&self.store, cx);
        *self.item_heights.borrow_mut() = heights;
        self.item_sizes = Self::sizes_from_heights(&self.item_heights.borrow());
    }

    pub fn item_count(&self) -> usize {
        self.store.len()
    }

    pub fn scroll_anchor_ix(&self) -> usize {
        self.item_count() / 2
    }

    pub fn derived_anchor(&self) -> AnchorSnapshot {
        derive_anchor(self.scroll_handle.offset().y, &self.item_heights.borrow())
    }

    pub fn is_at_bottom(&self) -> bool {
        let o = self.scroll_handle.offset();
        let max = self.scroll_handle.max_offset();
        (o.y - max.height).abs() < px(3.)
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_handle.scroll_to_bottom();
        self.follow_bottom = true;
    }

    pub fn scroll_to_logical(&mut self, k: usize, o: Pixels) {
        let heights = self.item_heights.borrow();
        let y = scroll_y_for_anchor(k, o, &heights);
        self.scroll_handle.set_offset(point(px(0.), y));
        self.follow_bottom = false;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_handle.set_offset(point(px(0.), px(0.)));
        self.follow_bottom = false;
    }

    pub fn scroll_to_reveal(&mut self, ix: usize) {
        self.scroll_handle
            .scroll_to_item(ix, gpui::ScrollStrategy::Top);
        self.follow_bottom = false;
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

    pub fn bump_row_height(&mut self, ix: usize, delta: Pixels, cx: &mut App) {
        let mut heights = self.item_heights.borrow_mut();
        if let Some(h) = heights.get_mut(ix) {
            *h += delta;
        }
        drop(heights);
        self.item_sizes = Self::sizes_from_heights(&self.item_heights.borrow());
        if let Some(id) = self.store.id_at(ix)
            && let Some(entity) = self.store.entity(id)
        {
            let h = self.item_heights.borrow()[ix];
            entity.update(cx, |state, _| state.measured_height = Some(h));
        }
    }

    pub fn invalidate_last_row_height(&mut self, cx: &mut App) {
        let last = self.item_count().saturating_sub(1);
        if let Some(id) = self.store.id_at(last)
            && let Some(entity) = self.store.entity(id)
        {
            entity.update(cx, |state, _| state.measured_height = None);
        }
        self.resync_heights(cx);
    }
}

impl RowSource for BackendB {
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
        self.fixture.append_to_last(chunk);
        self.store.append_to_last(chunk, cx);
        self.invalidate_last_row_height(cx);
        if self.follow_bottom {
            self.scroll_to_bottom();
        }
    }

    fn mutate_height(&mut self, id: RowId, delta: Pixels, _window: &mut Window, cx: &mut App) {
        self.fixture.mutate_offscreen_height(delta);
        self.store.mutate_height(id, delta, cx);
        if let Some(ix) = self.store.order.iter().position(|&rid| rid == id) {
            self.bump_row_height(ix, delta, cx);
        }
    }
}
