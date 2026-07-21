mod layout_adapter;
mod rollup;

use crate::PtyProbe;
use crate::card::view::{SessionCardView, mount_cached_card};
use crate::fleet::store::FleetStore;
use crate::slot::{TabHandle, placeholder_tab};
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, ClickEvent, Context, Entity, IntoElement,
    ParentElement, Pixels, Render, ScrollHandle, Styled, Window, div, prelude::*, px,
};
use layout_adapter::build_ephemeral_layout;
use lens_core::domain::board::BoardNode;
use lens_core::domain::ids::SessionId;
use lens_core::pack::{self, CARD_H, CARD_W, CELL_H, CELL_W, GAP, HEADER, INSET, Item};
use std::collections::{HashMap, HashSet};

/// Width of the left nav rail (unchanged placeholder).
const NAV_RAIL_W: f32 = 48.0;
/// Width of the focused-mode session rail (spec §5; `.boards` strip = 286px).
const RAIL_W: f32 = 286.0;

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
    /// Session ids currently gated visible (their anim timers allowed to run).
    /// The container is the sole authority; diffed each render, applied via defer.
    gated_visible: HashSet<SessionId>,
    /// Scroll position of the board masonry surface (spec §4 unknown 1).
    board_scroll: ScrollHandle,
    /// Scroll position of the focused-mode rail (same container at 1 col, §5).
    rail_scroll: ScrollHandle,
    /// Session ids whose tiles were in the visible band at the last render —
    /// the cull result (test hook + Task 5's gate input).
    last_built: Vec<SessionId>,
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
            gated_visible: HashSet::new(),
            board_scroll: ScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            last_built: Vec::new(),
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

    /// Apply the container-computed visible set to the card views — the diff since
    /// last frame, pushed via `App::defer` so no sibling card entity is read inside
    /// `render`'s accessed-entity window (the `.cached()` dirty-tracking landmine,
    /// [[viewport-reentry-freeze]]). Newly-visible cards spawn their timers; newly-
    /// hidden cards drop them. Cards absent from any surface stay hidden.
    fn apply_visibility_gate(&mut self, want: HashSet<SessionId>, cx: &mut Context<Self>) {
        if want == self.gated_visible {
            return;
        }
        let newly_vis: Vec<SessionId> = want.difference(&self.gated_visible).cloned().collect();
        let newly_hid: Vec<SessionId> = self.gated_visible.difference(&want).cloned().collect();
        let views = self.card_views.clone(); // Entity clones are cheap (Rc)
        self.gated_visible = want;
        cx.defer(move |app: &mut App| {
            for id in newly_vis {
                if let Some(v) = views.get(&id) {
                    v.update(app, |c, cx| c.set_visible(true, cx));
                }
            }
            for id in newly_hid {
                if let Some(v) = views.get(&id) {
                    v.update(app, |c, cx| c.set_visible(false, cx));
                }
            }
        });
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
            .w(px(NAV_RAIL_W))
            .h_full()
            .flex_shrink_0()
            .child("nav")
    }

    /// The masonry scroll container (spec §4). Builds the ephemeral tree, packs
    /// it into `cols_for_width(avail_width)` columns, and renders only tiles whose
    /// y-range intersects the visible band (+ `1×CELL_H` overdraw). Returns the
    /// element and the visible-band session ids (Task 5's gate consumes them).
    fn pack_and_render(
        &mut self,
        avail_width: f32,
        viewport_h: f32,
        scroll: ScrollHandle,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Vec<SessionId>) {
        let layout = build_ephemeral_layout(self.fleet.read(cx));
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
        let nodes = layout.board_tree(&board_id).unwrap_or_default();

        // nodes → parallel (pack items, per-tile session ids)
        let mut items: Vec<Item> = Vec::with_capacity(nodes.len());
        let mut tile_sessions: Vec<Vec<SessionId>> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            let sessions: Vec<SessionId> = node.leaf_sessions().into_iter().cloned().collect();
            items.push(match node {
                BoardNode::Card(_) => Item::card(),
                BoardNode::Group { .. } => Item::group(sessions.len()),
            });
            tile_sessions.push(sessions);
        }

        let cols = pack::cols_for_width(avail_width);
        let packing = pack::pack(&items, cols);

        // Last frame's painted offset (one-frame lag → overdraw covers it, §8).
        let scroll_top = (-f32::from(scroll.offset().y)).max(0.0);
        let overdraw = CELL_H;
        let lo = scroll_top - overdraw;
        let hi = scroll_top + viewport_h + overdraw;

        let mut content = div()
            .relative()
            .w(px(cols as f32 * CELL_W))
            .h(px(packing.content_height));

        let mut visible: Vec<SessionId> = Vec::new();
        for placed in &packing.tiles {
            if !placed.intersects_band(lo, hi) {
                continue; // culled → absent from child vec → gpui never builds it
            }
            let sessions = &tile_sessions[placed.item_index];
            for s in sessions {
                visible.push(s.clone());
            }
            match placed.item.kind {
                pack::Kind::Card => {
                    if let Some(tile) = self.absolute_card(
                        &sessions[0],
                        placed.cell_left(),
                        placed.cell_top() + HEADER,
                        cx,
                    ) {
                        content = content.child(tile);
                    }
                }
                pack::Kind::Group { .. } => {
                    for el in self.absolute_group(placed, sessions, cx) {
                        content = content.child(el);
                    }
                }
            }
        }

        self.last_built = visible.clone();

        let el = div()
            .id("board-scroll")
            .size_full()
            .overflow_scroll()
            .track_scroll(&scroll)
            .child(content)
            .into_any_element();
        (el, visible)
    }

    /// One loose card absolutely positioned at its body-zone (`top` already offset
    /// by HEADER by the caller). Clickable (focus the session).
    fn absolute_card(
        &self,
        session_id: &SessionId,
        left: f32,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let cached = self.cached_tiles.get(session_id)?.clone();
        let entity_id = self.card_views.get(session_id)?.entity_id();
        let sid = session_id.clone();
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(CARD_W))
                .h(px(CARD_H))
                .id(("session-card-click", entity_id))
                .on_click(cx.listener(move |board, event, window, cx| {
                    board.card_click(sid.clone(), event, window, cx);
                }))
                .child(cached)
                .into_any_element(),
        )
    }

    /// A group tile: a **bare neutral placeholder box** in the inter-tile gap plus
    /// its member cards at full size in body-zones. Chrome (ring color / header /
    /// rollups) is B-3; this arm proves the geometry and gives B-3 something to
    /// fill. Under basis B no group is reachable at runtime — exercised in B-4.
    fn absolute_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (fc, fr) = (placed.item.fc, placed.item.fr);
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = fc as f32 * CELL_W - GAP;
        let block_h = fr as f32 * CELL_H - GAP;

        let mut out: Vec<AnyElement> = Vec::with_capacity(sessions.len() + 1);
        // Bare neutral placeholder box in the gap (group chrome = B-3). A SIBLING
        // of the member cards in content space — never their parent, or the group
        // origin would apply twice.
        out.push(
            div()
                .absolute()
                .left(px(x - INSET))
                .top(px(y - INSET))
                .w(px(block_w + 2.0 * INSET))
                .h(px(block_h + 2.0 * INSET))
                .rounded(px(12.0))
                .border_1()
                .border_color(gpui::rgb(0x3a3a42))
                .into_any_element(),
        );
        for (i, session) in sessions.iter().enumerate() {
            let cc = i % fc;
            let rr = i / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * CELL_H;
            if let Some(tile) = self.absolute_card(session, x - INSET + mx, y - INSET + my, cx) {
                out.push(tile);
            }
        }
        out
    }

    /// Test hook: the session ids whose tiles were built (in the visible band) at
    /// the last render — proves culling.
    pub fn visible_session_ids_for_test(&self) -> Vec<SessionId> {
        self.last_built.clone()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_card_views(cx);
        let mode = ShellMode::from_fleet(self.fleet.read(cx));
        let viewport = window.viewport_size();
        let viewport_h = f32::from(viewport.height);
        let viewport_w = f32::from(viewport.width);

        let (body, visible): (_, Vec<SessionId>) = match &mode {
            ShellMode::Board => {
                let avail = (viewport_w - NAV_RAIL_W).max(CELL_W);
                let (surface, visible) =
                    self.pack_and_render(avail, viewport_h, self.board_scroll.clone(), cx);
                let el = div()
                    .id("shell-board")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().flex_grow().h_full().child(surface));
                (el.into_any_element(), visible)
            }
            ShellMode::Focused { .. } => {
                let (rail, visible) =
                    self.pack_and_render(RAIL_W, viewport_h, self.rail_scroll.clone(), cx);
                let el = div()
                    .id("shell-focused")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail())
                    .child(div().w(px(RAIL_W)).flex_shrink_0().h_full().child(rail))
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
                    );
                (el.into_any_element(), visible)
            }
        };
        self.apply_visibility_gate(visible.into_iter().collect(), cx);
        div().id("board-view").size_full().child(body)
    }
}

/// Group accent color from its persisted `color_token` (spec §3, SSOT palette
/// `docs/design/renders/board-home.html:8-12`). Unknown / `None` → neutral slate.
/// B-3-local resolver; promoting these to `LensTheme` tokens is a documented
/// follow-up (matches the B-2 arm hardcoding its border color).
fn group_accent(token: Option<&str>) -> gpui::Hsla {
    let hex: u32 = match token {
        Some("blue") => 0x4c8dff,
        Some("orange") => 0xff8a3d,
        Some("green") => 0x36c98a,
        Some("purple") => 0xb08cff,
        _ => 0x6b7280, // neutral slate
    };
    gpui::rgb(hex).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_accent_maps_ssot_tokens() {
        assert_eq!(group_accent(Some("blue")), gpui::rgb(0x4c8dff).into());
        assert_eq!(group_accent(Some("orange")), gpui::rgb(0xff8a3d).into());
        assert_eq!(group_accent(Some("green")), gpui::rgb(0x36c98a).into());
        assert_eq!(group_accent(Some("purple")), gpui::rgb(0xb08cff).into());
    }

    #[test]
    fn group_accent_unknown_and_none_fall_back_to_neutral() {
        let neutral: gpui::Hsla = gpui::rgb(0x6b7280).into();
        assert_eq!(group_accent(None), neutral);
        assert_eq!(group_accent(Some("chartreuse")), neutral);
    }
}
