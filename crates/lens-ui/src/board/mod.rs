use crate::PtyProbe;
use crate::actions::BackToBoard;
use crate::card::view::{SessionCardView, mount_cached_card};
use crate::fleet::store::FleetStore;
use crate::slot::{TabHandle, placeholder_tab};
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window, div, prelude::*, px,
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
    working_tab: TabHandle,
    #[allow(dead_code)]
    pty_probe: Option<PtyProbe>,
}

impl BoardView {
    pub fn new(fleet: Entity<FleetStore>, cx: &mut App) -> Entity<Self> {
        let working_tab = placeholder_tab(cx);
        let cards: Vec<_> = fleet
            .read(cx)
            .cards
            .iter()
            .map(|(id, card)| (id.clone(), card.clone()))
            .collect();
        let mut card_views = HashMap::new();
        for (id, card) in cards {
            card_views.insert(id, cx.new(|cx| SessionCardView::new(card, cx)));
        }
        let fleet_for_observe = fleet.clone();
        cx.new(move |cx| {
            cx.observe(&fleet_for_observe, |board: &mut BoardView, _, cx| {
                board.sync_card_views(cx);
                cx.notify();
            })
            .detach();
            Self {
                fleet: fleet_for_observe,
                card_views,
                working_tab,
                pty_probe: None,
            }
        })
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
            self.card_views
                .insert(id, cx.new(|cx| SessionCardView::new(card, cx)));
        }
    }

    fn on_back_to_board(&mut self, _: &BackToBoard, _: &mut Window, cx: &mut Context<Self>) {
        self.fleet.update(cx, |fleet, cx| fleet.blur_to_board(cx));
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

    fn render_card_tile(
        &self,
        session_id: SessionId,
        view: Entity<SessionCardView>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity_id = view.entity_id();
        div()
            .id(("session-card-click", entity_id))
            .on_click(cx.listener(move |board, event, window, cx| {
                board.card_click(session_id.clone(), event, window, cx);
            }))
            .child(mount_cached_card(view))
    }

    fn render_board_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut session_ids: Vec<_> = self.card_views.keys().cloned().collect();
        session_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut grid = div().id("board-grid").flex().flex_wrap().gap_2();
        for id in session_ids {
            if let Some(view) = self.card_views.get(&id) {
                grid = grid.child(self.render_card_tile(id, view.clone(), cx));
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
            if let Some(view) = self.card_views.get(&id) {
                column = column.child(self.render_card_tile(id, view.clone(), cx));
            }
        }
        column
    }

    #[cfg(test)]
    pub fn card_views_for_test(&self) -> &HashMap<SessionId, Entity<SessionCardView>> {
        &self.card_views
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
        div()
            .id("board-view")
            .size_full()
            .on_action(cx.listener(Self::on_back_to_board))
            .child(body)
    }
}
