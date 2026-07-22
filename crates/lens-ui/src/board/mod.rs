pub mod replica;
mod rollup;

pub use replica::{BoardReplica, ReplicaState, WriteDisposition};

use crate::PtyProbe;
use crate::card::model::SessionCard;
use crate::card::view::{SessionCardView, mount_cached_card};
use crate::fleet::store::FleetStore;
use crate::slot::TabHandle;
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, ClickEvent, Context, Entity, IntoElement,
    ParentElement, Pixels, Render, ScrollHandle, Styled, Window, div, prelude::*, px,
};
use lens_core::domain::board::BoardNode;
use lens_core::domain::ids::SessionId;
use lens_core::pack::{self, CARD_H, CARD_W, CELL_H, CELL_W, GAP, HEADER, INSET, Item};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

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

/// Per-tile group metadata threaded from `board_tree` into the renderer (B-3).
/// `completed_count` is Archive-side (B-6); B-3 passes 0.
struct GroupMeta {
    name: String,
    color_token: Option<String>,
    completed_count: u32,
}

/// The chrome computed for one rendered group tile — asserted by fixture tests
/// (the group render path is not runtime-reachable until B-4).
#[derive(Clone, Debug)]
pub struct GroupChromeSnapshot {
    pub session_ids: Vec<SessionId>,
    pub name: String,
    pub accent: gpui::Hsla,
    pub rollup: rollup::GroupRollup,
    pub header: String,
}

pub struct BoardView {
    fleet: Entity<FleetStore>,
    replica: Entity<BoardReplica>,
    card_views: HashMap<SessionId, Entity<SessionCardView>>,
    /// Stable `.cached()` wrappers — created once per card so layout recompose reuses cache.
    cached_tiles: HashMap<SessionId, AnyView>,
    working_tab: TabHandle,
    pty_probe: Option<PtyProbe>,
    /// Session ids currently gated visible (their anim timers allowed to run).
    /// The container is the sole authority; diffed each render, applied via defer.
    gated_visible: HashSet<SessionId>,
    /// Bumps on each gate apply so stale `defer` closures from an older `want`
    /// cannot resurrect visibility after a newer gate (replica async re-notify path).
    gate_epoch: Rc<Cell<u64>>,
    /// Scroll position of the board masonry surface (spec §4 unknown 1).
    board_scroll: ScrollHandle,
    /// Scroll position of the focused-mode rail (same container at 1 col, §5).
    rail_scroll: ScrollHandle,
    /// Session ids whose tiles were in the visible band at the last render —
    /// the cull result (test hook + Task 5's gate input).
    last_built: Vec<SessionId>,
    /// B-3 render snapshot: the group chrome computed at the last render (test hook;
    /// also the eventual B-4 live-inspection point). Recomputed each frame.
    last_group_chrome: Vec<GroupChromeSnapshot>,
}

impl BoardView {
    /// Builds a board view inside an existing entity context (window root or `cx.new`).
    pub fn mount(
        fleet: Entity<FleetStore>,
        replica: Entity<BoardReplica>,
        working_tab: TabHandle,
        pty_probe: Option<PtyProbe>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&replica, |_b: &mut BoardView, _, cx| cx.notify())
            .detach();
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
            replica,
            card_views,
            cached_tiles,
            working_tab,
            pty_probe,
            gated_visible: HashSet::new(),
            gate_epoch: Rc::new(Cell::new(0)),
            board_scroll: ScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            last_built: Vec::new(),
            last_group_chrome: Vec::new(),
        }
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
        let epoch = self.gate_epoch.get().saturating_add(1);
        self.gate_epoch.set(epoch);
        let gate_epoch = self.gate_epoch.clone();
        cx.defer(move |app: &mut App| {
            if gate_epoch.get() != epoch {
                return;
            }
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
        let layout = self.replica.read(cx).layout().clone();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
        let nodes = layout.board_tree(&board_id).unwrap_or_default();

        // nodes → parallel (pack items, per-tile session ids, per-tile group meta)
        let mut items: Vec<Item> = Vec::with_capacity(nodes.len());
        let mut tile_sessions: Vec<Vec<SessionId>> = Vec::with_capacity(nodes.len());
        let mut tile_groups: Vec<Option<GroupMeta>> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            let sessions: Vec<SessionId> = node.leaf_sessions().into_iter().cloned().collect();
            let (item, meta) = match node {
                BoardNode::Card(_) => (Item::card(), None),
                BoardNode::Group { item, .. } => {
                    let meta = match &item.kind {
                        lens_core::domain::board::BoardItemKind::Group {
                            name,
                            color_token,
                            ..
                        } => Some(GroupMeta {
                            name: name.clone(),
                            color_token: color_token.clone(),
                            // ✓N is Archive-side (B-6); B-3 has no source → 0.
                            completed_count: 0,
                        }),
                        // A Group node always carries a Group kind; defensively None.
                        _ => None,
                    };
                    (Item::group(sessions.len()), meta)
                }
            };
            items.push(item);
            tile_sessions.push(sessions);
            tile_groups.push(meta);
        }

        let cols = pack::cols_for_width(avail_width);
        let packing = pack::pack(&items, cols);

        // Last frame's painted offset (one-frame lag → overdraw covers it, §8).
        let scroll_top = (-f32::from(scroll.offset().y)).max(0.0);
        let overdraw = CELL_H;
        let lo = scroll_top - overdraw;
        let hi = scroll_top + viewport_h + overdraw;

        let now_ms = self.fleet.read(cx).clock().now_millis();
        let mut group_chrome: Vec<GroupChromeSnapshot> = Vec::new();

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
                    let meta = tile_groups[placed.item_index].as_ref();
                    let (els, snap) = self.absolute_group(placed, sessions, meta, now_ms, cx);
                    for el in els {
                        content = content.child(el);
                    }
                    group_chrome.push(snap);
                }
            }
        }

        self.last_built = visible.clone();
        self.last_group_chrome = group_chrome;

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

    /// A group tile (B-3): a colored ring + tint in the inter-tile gap, a header-lane
    /// (`● dot · name · [spend · age] · ✓N · ⌄`) folded from member cards, plus the
    /// members at full size in their body-zones. Returns the elements and a chrome
    /// snapshot (fixture tests assert the snapshot; the path is not runtime-reachable
    /// until B-4).
    ///
    /// NOTE (B-4): this reads member `SessionCard` entities during `render` to fold the
    /// rollup. B-3 is runtime-dormant so this never executes live; when B-4 makes groups
    /// live, verify this does not re-trip the `.cached()` dirty-tracking freeze
    /// ([[viewport-reentry-freeze]]). If it does, hoist the fold into `sync_card_views`.
    fn absolute_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        meta: Option<&GroupMeta>,
        now_ms: i64,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, GroupChromeSnapshot) {
        let (fc, fr) = (placed.item.fc, placed.item.fr);
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = fc as f32 * CELL_W - GAP;
        let block_h = fr as f32 * CELL_H - GAP;

        let name = meta.map(|m| m.name.clone()).unwrap_or_default();
        let completed = meta.map(|m| m.completed_count).unwrap_or(0);
        let accent = group_accent(meta.and_then(|m| m.color_token.as_deref()));

        // Fold the rollup from member cards (snapshot the values we need — owned).
        let members: Vec<SessionCard> = {
            let fleet = self.fleet.read(cx);
            sessions
                .iter()
                .filter_map(|s| fleet.cards.get(s).map(|e| e.read(cx).clone()))
                .collect()
        };
        let member_refs: Vec<&SessionCard> = members.iter().collect();
        let rollup = rollup::group_rollup(&member_refs, completed);
        let header = rollup::group_header_text(&name, &rollup, now_ms);

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            header: header.clone(),
        };

        let mut out: Vec<AnyElement> = Vec::with_capacity(sessions.len() + 2);

        // Ring + tint box in the gap (spec §3). Sibling of the member cards.
        out.push(
            div()
                .absolute()
                .left(px(x - INSET))
                .top(px(y - INSET))
                .w(px(block_w + 2.0 * INSET))
                .h(px(block_h + 2.0 * INSET))
                .rounded(px(12.0))
                .border_1()
                .border_color(accent)
                .bg(accent.opacity(0.07)) // SSOT color-mix ~7% body wash
                .into_any_element(),
        );

        // Header-lane (top HEADER-tall band): dot · name · spend · age · ✓N · caret.
        out.push(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(block_w))
                .h(px(HEADER))
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .px_1p5()
                .child(div().size(px(8.0)).rounded_full().bg(accent))
                .child(div().text_color(gpui::rgb(0xd6d6de)).child(name.clone()))
                .child(
                    div()
                        .flex_grow()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(format!(
                            "{} · {}",
                            rollup::format_group_spend(rollup.spend_usd),
                            rollup::format_age(rollup.oldest_created_at, now_ms),
                        )),
                )
                .child(
                    div()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(format!("✓{completed}")),
                )
                .child(div().text_color(gpui::rgb(0x8a8a94)).child("⌄"))
                .into_any_element(),
        );

        // Members at full size in body-zones (unchanged geometry from B-2).
        for (i, session) in sessions.iter().enumerate() {
            let cc = i % fc;
            let rr = i / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * CELL_H;
            if let Some(tile) = self.absolute_card(session, x - INSET + mx, y - INSET + my, cx) {
                out.push(tile);
            }
        }

        (out, snapshot)
    }

    /// Test hook: the session ids whose tiles were built (in the visible band) at
    /// the last render — proves culling.
    pub fn visible_session_ids_for_test(&self) -> Vec<SessionId> {
        self.last_built.clone()
    }

    /// Test hook: the group chrome computed at the last render.
    pub fn group_chrome_for_test(&self) -> Vec<GroupChromeSnapshot> {
        self.last_group_chrome.clone()
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
