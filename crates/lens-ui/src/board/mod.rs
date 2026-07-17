use crate::PtyProbe;
use crate::card::view::{SessionCardView, mount_cached_card};
use crate::fleet::store::FleetStore;
use crate::slot::{TabHandle, placeholder_tab};
use gpui::{
    AnyView, App, AppContext, Bounds, ClickEvent, Context, Entity, IntoElement, ParentElement,
    Pixels, Render, Styled, Window, div, prelude::*, px,
};
use lens_core::domain::ids::SessionId;
use std::collections::HashMap;

/// Shell layout mode derived from `FleetStore::focused`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMode {
    Board,
    Focused { session_id: SessionId },
}

impl ShellMode {
    pub fn from_fleet(fleet: &FleetStore) -> Self {
        match fleet.focused() {
            None => ShellMode::Board,
            Some(id) => ShellMode::Focused {
                session_id: id.clone(),
            },
        }
    }
}

pub struct BoardView {
    fleet: Entity<FleetStore>,
    card_views: HashMap<SessionId, Entity<SessionCardView>>,
    /// Stable `.cached()` wrappers — created once per card so layout recompose reuses cache.
    cached_tiles: HashMap<SessionId, AnyView>,
    working_tab: TabHandle,
    pty_probe: Option<PtyProbe>,
}

impl BoardView {
    /// Builds a board view inside an existing entity context (window root or `cx.new`).
    pub fn mount(
        fleet: Entity<FleetStore>,
        working_tab: TabHandle,
        pty_probe: Option<PtyProbe>,
        cx: &mut Context<Self>,
    ) -> Self {
        let cards: Vec<_> = fleet
            .read(cx)
            .cards
            .iter()
            .map(|(id, card)| (id.clone(), card.clone()))
            .collect();
        let mut card_views = HashMap::new();
        let mut cached_tiles = HashMap::new();
        for (id, card) in cards {
            let clock = fleet.read(cx).clock();
            let view =
                cx.new(|cx| SessionCardView::new(card, clock, fleet.clone(), id.clone(), cx));
            cached_tiles.insert(id.clone(), mount_cached_card(view.clone()));
            card_views.insert(id, view);
        }
        let fleet_for_observe = fleet.clone();
        cx.observe(&fleet_for_observe, |board: &mut BoardView, _, cx| {
            board.sync_card_views(cx);
            cx.notify();
        })
        .detach();
        Self {
            fleet: fleet_for_observe,
            card_views,
            cached_tiles,
            working_tab,
            pty_probe,
        }
    }

    pub fn new(fleet: Entity<FleetStore>, cx: &mut App) -> Entity<Self> {
        let working_tab = placeholder_tab(cx);
        cx.new(|cx| Self::mount(fleet, working_tab, None, cx))
    }

    fn make_card_view(
        &self,
        id: SessionId,
        card: Entity<crate::card::model::SessionCard>,
        cx: &mut Context<Self>,
    ) -> Entity<SessionCardView> {
        let clock = self.fleet.read(cx).clock();
        cx.new(|cx| SessionCardView::new(card, clock, self.fleet.clone(), id, cx))
    }

    fn insert_card_view(&mut self, id: SessionId, view: Entity<SessionCardView>) {
        self.cached_tiles
            .insert(id.clone(), mount_cached_card(view.clone()));
        self.card_views.insert(id, view);
    }

    fn sync_card_views(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<_> = {
            let fleet = self.fleet.read(cx);
            fleet
                .cards
                .iter()
                .filter(|(id, _)| !self.card_views.contains_key(*id))
                .map(|(id, card)| (id.clone(), card.clone()))
                .collect()
        };
        for (id, card) in missing {
            let view = self.make_card_view(id.clone(), card, cx);
            self.insert_card_view(id, view);
        }
    }

    fn card_click(
        &mut self,
        session_id: SessionId,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fleet
            .update(cx, |fleet, cx| fleet.focus_session(session_id, cx));
    }

    fn render_nav_rail(&self) -> impl IntoElement {
        div()
            .id("nav-rail")
            .w(px(48.0))
            .h_full()
            .flex_shrink_0()
            .child("nav")
    }

    fn render_card_tile(&self, session_id: SessionId, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.card_views.get(&session_id).expect("card view");
        let entity_id = view.entity_id();
        let cached = self
            .cached_tiles
            .get(&session_id)
            .expect("cached tile")
            .clone();
        div()
            .id(("session-card-click", entity_id))
            .on_click(cx.listener(move |board, event, window, cx| {
                board.card_click(session_id.clone(), event, window, cx);
            }))
            .child(cached)
    }

    fn render_board_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut session_ids: Vec<_> = self.card_views.keys().cloned().collect();
        session_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        // gap ≥ 2× the expanding-ring reach (12px each side, motion.rs) so the breathe
        // animation of adjacent cards doesn't bleed; padding lifts the top row off the edge;
        // justify_center + content_start centers the columns and pins rows to the top.
        let mut grid = div()
            .id("board-grid")
            .flex()
            .flex_wrap()
            .flex_grow()
            .h_full()
            .content_start()
            .justify_center()
            .gap(px(28.0))
            .p(px(28.0));
        for id in session_ids {
            if self.card_views.contains_key(&id) {
                grid = grid.child(self.render_card_tile(id, cx));
            }
        }
        grid
    }

    fn render_shrunk_boards(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut session_ids: Vec<_> = self.card_views.keys().cloned().collect();
        session_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut column = div()
            .id("boards-shrunk")
            .flex()
            .flex_col()
            .gap_1()
            .w(px(280.0))
            .flex_shrink_0();
        for id in session_ids {
            if self.card_views.contains_key(&id) {
                column = column.child(self.render_card_tile(id, cx));
            }
        }
        column
    }

    /// Acceptance-test hook: map session id → cached card view entity.
    pub fn card_views_for_test(&self) -> &HashMap<SessionId, Entity<SessionCardView>> {
        &self.card_views
    }

    /// Acceptance-test hook: last canvas-captured layout bounds for a card tile.
    pub fn card_bounds_for_test(&self, id: &SessionId, cx: &App) -> Option<Bounds<Pixels>> {
        self.card_views
            .get(id)
            .and_then(|view| view.read(cx).last_bounds.get())
    }

    /// Acceptance-test hook: install PTY byte counter for BackToBoard routing checks.
    pub fn set_pty_probe_for_test(&mut self, probe: PtyProbe) {
        self.pty_probe = Some(probe);
    }

    /// Acceptance-test hook: focus the working-area placeholder tab (terminal stand-in).
    pub fn focus_working_tab_for_test(&self, window: &mut Window, _cx: &App) {
        window.focus(&self.working_tab.focus_handle);
    }
}

impl Render for BoardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_card_views(cx);
        let mode = ShellMode::from_fleet(self.fleet.read(cx));
        let body = match mode {
            ShellMode::Board => div()
                .id("shell-board")
                .flex()
                .flex_row()
                .size_full()
                .child(self.render_nav_rail())
                .child(self.render_board_grid(cx)),
            ShellMode::Focused { .. } => div()
                .id("shell-focused")
                .flex()
                .flex_row()
                .size_full()
                .child(self.render_nav_rail())
                .child(self.render_shrunk_boards(cx))
                .child(div().id("chat-slot").flex_grow().child("chat"))
                .child(
                    div()
                        .id("navigator-slot")
                        .w(px(200.0))
                        .flex_shrink_0()
                        .child("navigator"),
                )
                .child(
                    div()
                        .id("working-area-slot")
                        .flex_grow()
                        .child(self.working_tab.view.clone()),
                ),
        };
        div().id("board-view").size_full().child(body)
    }
}
