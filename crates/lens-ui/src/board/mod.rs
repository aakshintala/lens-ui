mod drag;
pub mod replica;
mod rollup;

pub use replica::{BoardReplica, ReplicaState, WriteDisposition};

use crate::PtyProbe;
use crate::card::motion::RING_REACH_PX;
use crate::card::view::{SessionCardView, mount_cached_card};
use crate::card::wave::{Wave, derive_wave, wave_deadline};
use crate::fleet::store::FleetStore;
use crate::slot::{TabHandle, focused_transcript_tab};
use crate::theme::ActiveLensTheme as _;
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, ClickEvent, Context, DragMoveEvent, Entity,
    EntityId, IntoElement, ParentElement, Pixels, Render, ScrollHandle, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px,
};
use lens_core::domain::board::{BoardItemKind, BoardNode};
use lens_core::domain::ids::{BoardId, BoardItemId, SessionId};
use lens_core::pack::{
    self, CARD_H, CARD_W, CELL_H, CELL_W, DraggedKind, DropTile, GAP, HEADER, INSET, Item,
    item_height,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// Ring-gutter: a group's tint/border overhangs its tile by this on every side, and the
/// board reserves it around the whole grid. Two bounds pin it (see `gutter_bounds` test):
/// - `>= RING_REACH_PX` — contains a member's attention pulse inside the group border
///   (and keeps a loose card's pulse off the viewport edge). `+1` gives 1px slack.
/// - `<= GAP/2` — two ADJACENT groups each overhang into the shared inter-tile gap; if the
///   sum exceeds `GAP` their boxes overlap (on-device: "the groups clip"). GAP/2 = 8.
///
/// Distinct from `pack::INSET` (the card-origin offset within a cell, which cancels in
/// member placement).
const GUTTER: f32 = RING_REACH_PX + 1.0;

// The two bounds above are compile-time invariants — a runtime test would const-fold to
// `assert!(true)`. Break either (e.g. bump RING_REACH_PX past GAP/2) and the build fails here.
const _: () = assert!(
    GUTTER >= RING_REACH_PX,
    "GUTTER must contain the attention ring, else a member pulse leaks past the group border",
);
const _: () = assert!(
    2.0 * GUTTER <= pack::GAP,
    "2*GUTTER must be <= GAP, else two adjacent groups' boxes overlap in the shared gap",
);

/// Width of the left nav rail (unchanged placeholder).
const NAV_RAIL_W: f32 = 48.0;
/// Horizontal + vertical breathing room around the masonry content, inside a pane (so cards
/// and group boxes never sit flush against the pane/rail edges).
const PAD: f32 = 16.0;
/// Width of the focused-mode session rail. Sized to hold a 1-col group's box (a card plus its
/// `2·GUTTER` ring overhang) with `PAD` breathing room each side, so the group tile fits
/// without horizontal scroll (was a flat 286 that clipped the 294px group box).
const RAIL_W: f32 = CARD_W + 2.0 * GUTTER + 2.0 * PAD;

/// Edge-band auto-scroll tuning (on-device refinement in Task 6).
const EDGE_BAND_PX: f32 = 40.0;
const EDGE_NUDGE_PX: f32 = 12.0;

fn node_pack_row(node: &BoardNode<'_>) -> (BoardItemId, Item, bool) {
    match node {
        BoardNode::Card(item) => (item.id.clone(), Item::card(), false),
        BoardNode::Group { item, members } => {
            let collapsed = matches!(
                item.kind,
                BoardItemKind::Group {
                    collapsed: true,
                    ..
                }
            );
            let pack_item = if collapsed {
                Item::group_collapsed(members.len())
            } else {
                Item::group(members.len())
            };
            (item.id.clone(), pack_item, collapsed)
        }
    }
}

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
#[derive(Clone)]
struct GroupMeta {
    id: BoardItemId,
    name: String,
    color_token: Option<String>,
    collapsed: bool,
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
    /// The "spend · age" middle text — the single source for the rendered element.
    pub spend_age: String,
    /// Whether the `✓N` element is rendered (⇔ `rollup.completed_count > 0`).
    pub shows_completed: bool,
    pub collapsed: bool,
    pub status_rows: Vec<(Wave, u32)>,
}

pub struct BoardView {
    fleet: Entity<FleetStore>,
    replica: Entity<BoardReplica>,
    card_views: HashMap<SessionId, Entity<SessionCardView>>,
    /// Stable `.cached()` wrappers — created once per card so layout recompose reuses cache.
    cached_tiles: HashMap<SessionId, AnyView>,
    working_tab: TabHandle,
    chat_tab: Option<TabHandle>,
    chat_replica_id: Option<EntityId>,
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
    /// B-3 render snapshot: the group chrome computed at the last render (test hook;
    /// also the eventual B-4 live-inspection point). Recomputed each frame.
    last_group_chrome: Vec<GroupChromeSnapshot>,
    /// One-shot repaint timer for the earliest collapsed-group time-based wave deadline
    /// (Ready decay / Scheduled wake). Collapsed members hold no per-card anim timer, so
    /// without this their rollup would show a stale wave until an unrelated re-render
    /// (codex final-review Important #1). `gpui::Task` is cancel-on-drop.
    rollup_wake: Option<gpui::Task<()>>,
    /// The deadline `rollup_wake` is currently armed for — re-arm only when it changes.
    armed_rollup_deadline: Option<i64>,
    /// Active drag session (`None` = Idle).
    drag: Option<drag::DragSession>,
    /// Column count from the last `pack_and_render` — snapshot building reuses it.
    last_pack_cols: usize,
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
        cx.observe(&replica, |board: &mut BoardView, _, cx| {
            if let Some(ref mut session) = board.drag
                && session.phase == drag::DragPhase::Committing
            {
                drag::on_wrote(session);
                board.drag = None;
            }
            if board.drag.is_some() && board.replica.read(cx).state() == ReplicaState::Stale {
                if let Some(ref mut session) = board.drag {
                    drag::on_failed(session);
                }
                board.drag = None;
            }
            cx.notify();
        })
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
            board.sync_chat_tab(cx);
            cx.notify();
        })
        .detach();
        Self {
            fleet: fleet_for_observe,
            replica,
            card_views,
            cached_tiles,
            working_tab,
            chat_tab: None,
            chat_replica_id: None,
            pty_probe,
            gated_visible: HashSet::new(),
            board_scroll: ScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            last_built: Vec::new(),
            last_group_chrome: Vec::new(),
            rollup_wake: None,
            armed_rollup_deadline: None,
            drag: None,
            last_pack_cols: 1,
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

    fn sync_chat_tab(&mut self, cx: &mut Context<Self>) {
        match ShellMode::from_fleet(self.fleet.read(cx)) {
            ShellMode::Board => {
                self.chat_tab = None;
                self.chat_replica_id = None;
            }
            ShellMode::Focused { .. } => {
                if let Some(replica) = self.fleet.read(cx).focused_replica() {
                    let rid = replica.entity_id();
                    if self.chat_replica_id != Some(rid) {
                        self.chat_tab = Some(focused_transcript_tab(replica, cx));
                        self.chat_replica_id = Some(rid);
                    }
                } else {
                    self.chat_tab = None;
                    self.chat_replica_id = None;
                }
            }
        }
    }

    fn sync_card_views(&mut self, cx: &mut Context<Self>) {
        // Collapsed-group members feed the status rollup (fleet card read) but must
        // not hold a SessionCardView entity — prune any that slipped in at mount.
        //
        // INVARIANT (do not break): pruning here is what CANCELS a collapsed member's
        // anim timer — `anim_task` is an entity-owned `gpui::Task` (cancel-on-drop) whose
        // spawn captures a WeakEntity, so dropping the last strong ref (these two maps are
        // the only holders) cancels the timer. The visibility gate never sends
        // `set_visible(false)` here (it clones `card_views` AFTER this prune, so the id
        // misses). Therefore: NEVER retain an `Entity<SessionCardView>` for a collapsed
        // member anywhere else, or its timer would leak with nothing to stop it.
        let collapsed_members = self.collapsed_group_member_ids(cx);
        self.card_views
            .retain(|id, _| !collapsed_members.contains(id));
        self.cached_tiles
            .retain(|id, _| !collapsed_members.contains(id));

        let missing: Vec<_> = {
            let fleet = self.fleet.read(cx);
            fleet
                .cards
                .iter()
                .filter(|(id, _)| !self.card_views.contains_key(*id))
                .filter(|(id, _)| !collapsed_members.contains(*id))
                .map(|(id, card)| (id.clone(), card.clone()))
                .collect()
        };
        for (id, card) in missing {
            let view = self.make_card_view(id.clone(), card, cx);
            self.insert_card_view(id, view);
        }
    }

    /// Session ids placed under a collapsed group on the default board — excluded
    /// from card-view entities (§3.1 visibility fork).
    fn collapsed_group_member_ids(&self, cx: &App) -> HashSet<SessionId> {
        let layout = self.replica.read(cx).layout();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return HashSet::new(),
        };
        let nodes = layout.board_tree(&board_id).unwrap_or_default();
        let mut out = HashSet::new();
        for node in &nodes {
            collect_collapsed_group_members(node, &mut out);
        }
        out
    }

    /// Schedule a single repaint at the earliest clock deadline across collapsed-group
    /// members (Ready decay / Scheduled wake), so their status rollup — which holds no
    /// per-card anim timer — refreshes when a time-based wave expires (codex final-review
    /// Important #1). The gpui timer is real-time; the fired repaint re-derives waves from
    /// the UI clock (the same dual-clock pattern as the card anim timer). Re-armed only
    /// when the deadline changes (no per-frame task churn); the fired task clears the
    /// armed deadline so the re-render re-arms the next one. Converges: once every
    /// collapsed member is clock-stable the deadline is `None` and no timer is held.
    fn arm_collapsed_rollup_wake(&mut self, cx: &mut Context<Self>) {
        let now_ms = self.fleet.read(cx).clock().now_millis();
        let members = self.collapsed_group_member_ids(cx);
        let next = {
            let fleet = self.fleet.read(cx);
            members
                .iter()
                .filter_map(|s| {
                    fleet
                        .cards
                        .get(s)
                        .and_then(|e| wave_deadline(e.read(cx), now_ms))
                })
                .min()
        };
        if next == self.armed_rollup_deadline {
            return; // unchanged — keep the pending timer, avoid re-spawning every frame
        }
        self.armed_rollup_deadline = next;
        self.rollup_wake = next.map(|deadline| {
            let delay = Duration::from_millis((deadline - now_ms).max(0) as u64);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |this, cx| {
                    // Spent — let the re-render re-arm the next deadline (or none).
                    this.armed_rollup_deadline = None;
                    cx.notify();
                });
            })
        });
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

    /// The caret toggle entry point (both ⌄ expanded and ▸ collapsed call this).
    /// Reads the current flag from the replica's committed layout, issues the flipped
    /// `SetCollapsed` write (commit-gated; a non-writable replica refuses + banners).
    fn toggle_group_collapsed(&mut self, group_id: BoardItemId, cx: &mut Context<Self>) {
        let current = matches!(
            self.replica
                .read(cx)
                .layout()
                .item(&group_id)
                .map(|it| &it.kind),
            Some(BoardItemKind::Group {
                collapsed: true,
                ..
            })
        );
        self.replica.update(cx, |r, cx| {
            r.write(
                replica::Op::SetCollapsed {
                    group_id,
                    collapsed: !current,
                },
                cx,
            );
        });
    }

    fn sibling_index(
        layout: &lens_core::domain::board::BoardLayout,
        item_id: &BoardItemId,
        parent: Option<&BoardItemId>,
    ) -> Option<usize> {
        let mut siblings: Vec<_> = layout
            .items
            .iter()
            .filter(|i| i.parent_item_id.as_ref() == parent)
            .collect();
        siblings.sort_by_key(|i| i.ordinal);
        siblings.iter().position(|i| &i.id == item_id)
    }

    fn build_drop_snapshot(
        layout: &lens_core::domain::board::BoardLayout,
        board_id: &BoardId,
        dragged_id: &BoardItemId,
        cols: usize,
    ) -> Vec<DropTile> {
        let nodes = layout.board_tree(board_id).unwrap_or_default();
        let mut items = Vec::new();
        let mut ids = Vec::new();
        let mut collapsed = Vec::new();
        for node in &nodes {
            let (item_id, item, is_collapsed) = node_pack_row(node);
            if item_id == *dragged_id {
                continue;
            }
            items.push(item);
            ids.push(item_id);
            collapsed.push(is_collapsed);
        }
        let packing = pack::pack(&items, cols);
        packing
            .tiles
            .into_iter()
            .map(|placed| DropTile {
                id: ids[placed.item_index].clone(),
                collapsed: collapsed[placed.item_index],
                placed,
            })
            .collect()
    }

    fn begin_item_drag(
        &mut self,
        item_id: BoardItemId,
        kind: DraggedKind,
        cursor: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if self
            .drag
            .as_ref()
            .is_some_and(|s| s.phase != drag::DragPhase::Idle)
        {
            return;
        }
        let layout = self.replica.read(cx).layout();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return,
        };
        let snapshot = Self::build_drop_snapshot(&layout, &board_id, &item_id, self.last_pack_cols);
        let Some(item) = layout.item(&item_id) else {
            return;
        };
        let start_parent = item.parent_item_id.clone();
        let sibling_idx = Self::sibling_index(&layout, &item_id, start_parent.as_ref());
        let layout_gen = self.replica.read(cx).layout_generation();
        self.drag = Some(drag::start_drag(
            item_id,
            kind,
            snapshot,
            layout_gen,
            cursor,
            sibling_idx,
            start_parent,
            board_id,
        ));
    }

    fn group_ghost(&self, id: BoardItemId, cx: &App) -> drag::DragGhost {
        let layout = self.replica.read(cx).layout();
        let fleet = self.fleet.read(cx);
        let now_ms = fleet.clock().now_millis();
        let item = layout.item(&id);
        let (name, accent, spend_age) = item
            .and_then(|it| match &it.kind {
                BoardItemKind::Group {
                    name, color_token, ..
                } => {
                    let accent = group_accent(color_token.as_deref());
                    let sessions: Vec<SessionId> = layout
                        .items
                        .iter()
                        .filter_map(|child| match &child.kind {
                            BoardItemKind::Card { session, .. }
                                if child.parent_item_id.as_ref() == Some(&it.id) =>
                            {
                                Some(session.clone())
                            }
                            _ => None,
                        })
                        .collect();
                    let members: Vec<rollup::MemberCost> = sessions
                        .iter()
                        .filter_map(|s| {
                            fleet
                                .cards
                                .get(s)
                                .map(|e| rollup::MemberCost::from_card(e.read(cx)))
                        })
                        .collect();
                    let rollup = rollup::group_rollup(&members, 0);
                    let spend_age = format!(
                        "{} · {}",
                        rollup::format_group_spend(rollup.spend_usd),
                        rollup::format_age(rollup.oldest_created_at, now_ms),
                    );
                    Some((name.clone(), accent, spend_age))
                }
                _ => None,
            })
            .unwrap_or_else(|| (String::new(), group_accent(None), String::new()));
        drag::DragGhost::group(id, name, accent, spend_age)
    }

    fn gap_placeholder_element(placed: &pack::Placed) -> AnyElement {
        let (fc, _fr) = drag::reflow_preview_placeholder_footprint(&placed.item);
        let w = fc as f32 * CELL_W - GAP;
        let h = item_height(&placed.item);
        div()
            .absolute()
            .left(px(placed.cell_left()))
            .top(px(placed.cell_top()))
            .w(px(w))
            .h(px(h))
            .rounded(px(8.0))
            .border_1()
            .border_color(gpui::rgb(0x4a4a54))
            .bg(gpui::rgb(0x1a1a22))
            .into_any_element()
    }

    fn render_nav_rail(&self, cx: &App) -> impl IntoElement {
        // Placeholder rail (real nav is unbuilt): a clean themed sidebar strip rather than
        // bare "nav" text bleeding at the window edge.
        let t = cx.lens_theme();
        div()
            .id("nav-rail")
            .w(px(NAV_RAIL_W))
            .h_full()
            .flex_shrink_0()
            .bg(t.base.sidebar)
            .border_r_1()
            .border_color(t.base.sidebar_border)
    }

    /// The masonry scroll container (spec §4). Builds the ephemeral tree, packs
    /// it into `cols_for_width(avail_width)` columns, and renders only tiles whose
    /// y-range intersects the visible band (+ `1×CELL_H` overdraw). Returns the
    /// element and the visible-band session ids (Task 5's gate consumes them).
    fn pack_and_render(
        &mut self,
        avail_width: f32,
        viewport_h: f32,
        max_cols: usize,
        scroll: ScrollHandle,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Vec<SessionId>) {
        let layout = self.replica.read(cx).layout();
        let board_id = match layout.default_board_id() {
            Ok(id) => id.clone(),
            Err(_) => return (div().into_any_element(), Vec::new()),
        };
        let nodes = layout.board_tree(&board_id).unwrap_or_default();

        struct PackRow {
            item: Item,
            item_id: BoardItemId,
            sessions: Vec<SessionId>,
            meta: Option<GroupMeta>,
            is_gap: bool,
        }

        let mut rows: Vec<PackRow> = nodes
            .iter()
            .map(|node| {
                let sessions: Vec<SessionId> = node.leaf_sessions().into_iter().cloned().collect();
                let (item_id, item, collapsed) = node_pack_row(node);
                let meta = match node {
                    BoardNode::Card(_) => None,
                    BoardNode::Group { item, .. } => {
                        let (name, color_token) = match &item.kind {
                            BoardItemKind::Group {
                                name, color_token, ..
                            } => (name.clone(), color_token.clone()),
                            _ => (String::new(), None),
                        };
                        Some(GroupMeta {
                            id: item.id.clone(),
                            name,
                            color_token,
                            collapsed,
                            completed_count: 0,
                        })
                    }
                };
                PackRow {
                    item,
                    item_id,
                    sessions,
                    meta,
                    is_gap: false,
                }
            })
            .collect();

        let drag_preview = self.drag.as_ref().filter(|s| {
            matches!(
                s.phase,
                drag::DragPhase::Dragging | drag::DragPhase::Committing
            )
        });
        if let Some(session) = drag_preview {
            let dragged_id = session.dragged_id.clone();
            let dragged_footprint = layout
                .item(&dragged_id)
                .map(|it| match &it.kind {
                    BoardItemKind::Card { .. } => Item::card(),
                    BoardItemKind::Group { .. } => {
                        let n = layout
                            .items
                            .iter()
                            .filter(|i| i.parent_item_id.as_ref() == Some(&dragged_id))
                            .count();
                        Item::group(n.max(1))
                    }
                })
                .unwrap_or(Item::card());

            let meta_by_id: HashMap<BoardItemId, Option<GroupMeta>> = rows
                .iter()
                .map(|r| (r.item_id.clone(), r.meta.clone()))
                .collect();
            let inputs: Vec<drag::ReflowPreviewInput> = rows
                .iter()
                .map(|r| drag::ReflowPreviewInput {
                    item: r.item,
                    item_id: r.item_id.clone(),
                    sessions: r.sessions.clone(),
                })
                .collect();
            let (preview_rows, _) = drag::apply_reflow_preview(
                &inputs,
                &dragged_id,
                &session.target,
                dragged_footprint,
                session.start_parent.as_ref(),
            );
            rows = preview_rows
                .into_iter()
                .map(|r| PackRow {
                    item: r.item,
                    item_id: r.item_id.clone(),
                    sessions: r.sessions,
                    meta: meta_by_id.get(&r.item_id).cloned().flatten(),
                    is_gap: r.is_gap,
                })
                .collect();
        }

        let items: Vec<Item> = rows.iter().map(|r| r.item).collect();
        let tile_sessions: Vec<Vec<SessionId>> = rows.iter().map(|r| r.sessions.clone()).collect();
        let tile_groups: Vec<Option<GroupMeta>> = rows.iter().map(|r| r.meta.clone()).collect();
        let tile_is_gap: Vec<bool> = rows.iter().map(|r| r.is_gap).collect();
        let tile_item_ids: Vec<BoardItemId> = rows.iter().map(|r| r.item_id.clone()).collect();

        // Cap the natural column count so a wide/ultrawide viewport packs into a bounded
        // block (centered below) instead of fanning a handful of sessions edge-to-edge.
        let cols = pack::cols_for_width(avail_width).min(max_cols);
        self.last_pack_cols = cols;
        let packing = pack::pack(&items, cols);

        // Vertical mirror of `center_offset` (see the horizontal block below): when the whole
        // board is shorter than the viewport, center it vertically; otherwise it overflows and
        // this is 0. A short board's block drifts up and hard-snaps to top-aligned as sessions
        // push `content_height` past the viewport (accepted trade-off).
        let block_height = 2.0 * PAD + 2.0 * GUTTER + packing.content_height;
        let fits_viewport = block_height <= viewport_h;
        let v_center_offset = if fits_viewport {
            (viewport_h - block_height) / 2.0
        } else {
            0.0
        };

        // Last frame's painted offset (one-frame lag → overdraw covers it, §8). When the block
        // fits the viewport the board cannot scroll, so pin the cull band to the top: a stale
        // negative offset (e.g. a tall scrolled board that just shrank below the viewport, not
        // yet clamped by gpui) would otherwise ride `v_center_offset > 0` and cull the now-short
        // block's tiles for a frame — vertical culling has no horizontal analog to lean on.
        let scroll_top = if fits_viewport {
            0.0
        } else {
            (-f32::from(scroll.offset().y)).max(0.0)
        };
        let overdraw = CELL_H;
        let lo = scroll_top - overdraw;
        let hi = scroll_top + viewport_h + overdraw;

        let now_ms = self.fleet.read(cx).clock().now_millis();
        let mut group_chrome: Vec<GroupChromeSnapshot> = Vec::new();

        // Center the packed block in the pane on wider screens. Cards are fixed-size, so the
        // occupied width is `used_cols` (≤ cols — a capped board with few sessions leaves
        // trailing columns empty) times CELL_W less the last column's absent trailing GAP,
        // plus the PAD/GUTTER frame. `pane_width` is the scroll container's width (both modes:
        // avail + the frame). The offset is purely horizontal and slides the absolutely-placed
        // tiles as a block; vertical culling (`intersects_band` on `py`) is untouched.
        let used_cols = packing.used_cols();
        let content_extent = 2.0 * PAD + 2.0 * GUTTER + used_cols as f32 * CELL_W - GAP;
        let pane_width = avail_width + 2.0 * PAD + 2.0 * GUTTER;
        let center_offset = ((pane_width - content_extent) / 2.0).max(0.0);

        // Grid offset by GUTTER inside `padded` (below): group rings/tints — and loose
        // cards' expanding attention rings — overhang their tile by up to GUTTER on every
        // side, so a tile at cell (0,0) would paint at (-GUTTER, -GUTTER) and clip against
        // the scroll viewport (on-device: "Group card clipped on the left and top"; the ring
        // reach also clipped loose top-left cards). `content` stays a positioning context.
        let content = div()
            .absolute()
            .left(px(PAD + GUTTER + center_offset))
            .top(px(PAD + GUTTER + v_center_offset))
            .w(px(used_cols as f32 * CELL_W))
            .h(px(packing.content_height));
        let mut content = content;

        let dragged_session = drag_preview.and_then(|s| {
            layout.item(&s.dragged_id).and_then(|it| match &it.kind {
                BoardItemKind::Card { session, .. } => Some(session.clone()),
                _ => None,
            })
        });

        let mut visible: Vec<SessionId> = Vec::new();
        for placed in &packing.tiles {
            if !placed.intersects_band(lo, hi) {
                continue;
            }
            if tile_is_gap.get(placed.item_index).copied().unwrap_or(false) {
                content = content.child(Self::gap_placeholder_element(placed));
                continue;
            }
            let sessions = &tile_sessions[placed.item_index];
            let item_id = tile_item_ids[placed.item_index].clone();
            match placed.item.kind {
                pack::Kind::Card => {
                    if dragged_session.as_ref() == Some(&sessions[0]) {
                        continue;
                    }
                    visible.push(sessions[0].clone());
                    if let Some(tile) = self.absolute_card(
                        &sessions[0],
                        &item_id,
                        placed.cell_left(),
                        placed.cell_top(),
                        cx,
                    ) {
                        content = content.child(tile);
                    }
                }
                pack::Kind::Group { .. } => {
                    let meta = tile_groups[placed.item_index].as_ref();
                    let collapsed = meta.map(|m| m.collapsed).unwrap_or(false);
                    if collapsed {
                        let (el, snap) =
                            self.absolute_collapsed_group(placed, sessions, meta, now_ms, cx);
                        content = content.child(el);
                        group_chrome.push(snap);
                    } else {
                        for s in sessions {
                            if dragged_session.as_ref() != Some(s) {
                                visible.push(s.clone());
                            }
                        }
                        let (els, snap) = self.absolute_group(
                            placed,
                            sessions,
                            meta,
                            now_ms,
                            dragged_session.as_ref(),
                            drag_preview.map(|s| &s.target),
                            cx,
                        );
                        for el in els {
                            content = content.child(el);
                        }
                        group_chrome.push(snap);
                    }
                }
            }
        }

        self.last_built = visible.clone();
        self.last_group_chrome = group_chrome;

        // Content extent = PAD breathing room + GUTTER ring overhang on each side, around the
        // occupied tile block (`used_cols·CELL_W − GAP`, the last column has no trailing gap)
        // and the masonry height. When the pane is wider (centered), span the full pane so the
        // block sits at `center_offset` without horizontal-scroll slack; when narrower, the
        // extent governs and the vertical-only scroll clips the horizontal overflow as before.
        // Height mirrors this: span the full viewport when the block fits (so `v_center_offset`
        // has room to center it), else the block height governs and the board scrolls.
        let padded = div()
            .relative()
            .w(px(content_extent.max(pane_width)))
            .h(px(block_height.max(viewport_h)))
            .child(content);
        // Drag handlers live on the SCROLL VIEWPORT (`el`, size_full), not the tightly-sized
        // `content` box — a drop released below the last row or in the side/bottom margin is
        // outside `content`'s hitbox, so gpui's hitbox-based drop dispatch would never fire
        // `on_drop` and the drag would strand in `Dragging` (card renders as a permanent gap).
        // Cursor is viewport-local here (`el` does not scroll), so translate to content-local by
        // subtracting the content origin (PAD+GUTTER+center offsets) and the live scroll offset.
        let scroll_for_drag = scroll.clone();
        let dx_off = PAD + GUTTER + center_offset;
        let dy_off = PAD + GUTTER + v_center_offset;
        let el = div()
            .id("board-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .on_drag_move(cx.listener(
                move |board, event: &DragMoveEvent<BoardItemId>, _window, cx| {
                    let bounds = event.bounds; // el = the viewport (unscrolled)
                    let cursor = event.event.position;
                    let vp_y = f32::from(cursor.y) - f32::from(bounds.origin.y);
                    // content-local: gpui scroll offset.y is ≤ 0 when scrolled down, so
                    // subtracting it adds the scrolled distance back.
                    let scroll_y = f32::from(scroll_for_drag.offset().y);
                    let local = (
                        f32::from(cursor.x) - f32::from(bounds.origin.x) - dx_off,
                        vp_y - scroll_y - dy_off,
                    );
                    let layout_gen = board.replica.read(cx).layout_generation();
                    if let Some(ref mut session) = board.drag
                        && session.phase == drag::DragPhase::Dragging
                    {
                        if !drag::on_cursor_move(session, local, layout_gen) {
                            board.drag = None;
                            cx.notify();
                            return;
                        }
                        // Edge auto-scroll keys on the true viewport band (vp_y ∈ [0, height]).
                        let dy = drag::edge_scroll_delta(
                            vp_y,
                            0.0,
                            f32::from(bounds.size.height),
                            EDGE_BAND_PX,
                            EDGE_NUDGE_PX,
                        );
                        if dy != 0.0 {
                            let mut off = scroll_for_drag.offset();
                            off.y -= px(dy);
                            scroll_for_drag.set_offset(off);
                        }
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|board, id: &BoardItemId, _window, cx| {
                let layout_gen = board.replica.read(cx).layout_generation();
                let Some(ref mut session) = board.drag else {
                    return;
                };
                if &session.dragged_id != id {
                    return;
                }
                if let Some(op) = drag::begin_commit(session, layout_gen) {
                    board.replica.update(cx, |r, cx| {
                        r.write(op, cx);
                    });
                } else {
                    board.drag = None;
                }
                cx.notify();
            }))
            .child(padded)
            .into_any_element();
        (el, visible)
    }

    /// One loose card absolutely positioned at its pixel-masonry top-left (`top` is the tile
    /// top — a loose card has no header lane). Clickable (focus the session).
    fn absolute_card(
        &self,
        session_id: &SessionId,
        item_id: &BoardItemId,
        left: f32,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let cached = self.cached_tiles.get(session_id)?.clone();
        let card_entity = self.card_views.get(session_id)?.clone();
        let entity_id = card_entity.entity_id();
        let sid = session_id.clone();
        let drag_id = item_id.clone();
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(CARD_W))
                .h(px(CARD_H))
                .id(("session-card-click", entity_id))
                .cursor_move()
                .on_click(cx.listener(move |board, event, window, cx| {
                    board.card_click(sid.clone(), event, window, cx);
                }))
                .on_drag(drag_id.clone(), {
                    let card_left = left;
                    let card_top = top;
                    let card_entity = card_entity.clone();
                    move |id, offset, _window, cx: &mut App| {
                        let cursor = (
                            card_left + f32::from(offset.x),
                            card_top + f32::from(offset.y),
                        );
                        let _ = weak.update(cx, |board, cx| {
                            board.begin_item_drag(id.clone(), DraggedKind::Card, cursor, cx);
                        });
                        card_entity.clone()
                    }
                })
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
    #[allow(clippy::too_many_arguments)] // render helper threading the drag preview target (B-4c §5)
    fn absolute_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        meta: Option<&GroupMeta>,
        now_ms: i64,
        hide_session: Option<&SessionId>,
        drop_target: Option<&lens_core::pack::DropTarget>,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, GroupChromeSnapshot) {
        let (fc, fr) = (placed.item.fc, placed.item.fr);
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = fc as f32 * CELL_W - GAP;
        // Tight box: header lane + fr member rows separated by GAP. The grid reserves fr full
        // CELL_H rows (a per-row header lane for masonry alignment), but a group has ONE header
        // — stacking members at CELL_H stride left a phantom HEADER-tall gap between every row
        // (glaring in a 1×N reflow). Wrap members tightly; the reserved-cell slack falls BELOW
        // the box, reading as clean separation before the next tile.
        let block_h = HEADER + fr as f32 * CARD_H + (fr as f32 - 1.0) * GAP;

        let name = meta.map(|m| m.name.clone()).unwrap_or_default();
        let completed = meta.map(|m| m.completed_count).unwrap_or(0);
        let accent = group_accent(meta.and_then(|m| m.color_token.as_deref()));
        let group_id = meta.map(|m| m.id.clone());

        // Fold the rollup from member cards via a NARROW projection (cost + created_at
        // only) — no full SessionCard clone per member per frame (codex final-review I2).
        let members: Vec<rollup::MemberCost> = {
            let fleet = self.fleet.read(cx);
            sessions
                .iter()
                .filter_map(|s| {
                    fleet
                        .cards
                        .get(s)
                        .map(|e| rollup::MemberCost::from_card(e.read(cx)))
                })
                .collect()
        };
        let rollup = rollup::group_rollup(&members, completed);
        let spend_age = format!(
            "{} · {}",
            rollup::format_group_spend(rollup.spend_usd),
            rollup::format_age(rollup.oldest_created_at, now_ms),
        );
        // One source for the ✓N badge: the painted string AND the snapshot bool both
        // derive from `badge` (the "render ✓N iff N>0" rule lives in `completed_badge`).
        let badge = completed_badge(rollup.completed_count);
        let shows_completed = badge.is_some();

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            spend_age: spend_age.clone(),
            shows_completed,
            collapsed: false,
            status_rows: Vec::new(),
        };

        let mut out: Vec<AnyElement> = Vec::with_capacity(sessions.len() + 2);

        // Ring + tint box in the gap (spec §3). Sibling of the member cards. Overhangs by
        // GUTTER (= ring reach) so an edge member's NeedsInput/Failed pulse is contained by
        // this border rather than leaking past it (on-device: "ring leaks past the box").
        out.push(
            div()
                .absolute()
                .left(px(x - GUTTER))
                .top(px(y - GUTTER))
                .w(px(block_w + 2.0 * GUTTER))
                .h(px(block_h + 2.0 * GUTTER))
                .rounded(px(12.0))
                .border_1()
                .border_color(accent)
                .bg(accent.opacity(0.12)) // SSOT color-mix ~12% body wash (cards are opaque; no bleed)
                .into_any_element(),
        );

        // Header-lane (top HEADER-tall band): dot · name · spend · age · ✓N · caret.
        let header_x = x;
        let header_y = y;
        let weak = cx.weak_entity();
        let caret = {
            let gid = group_id.clone();
            div()
                .id(("group-caret", placed.item_index))
                .cursor_pointer()
                .text_color(gpui::rgb(0x8a8a94))
                .child("⌄")
                .on_click(cx.listener(move |board, _ev, _win, cx| {
                    cx.stop_propagation();
                    if let Some(gid) = gid.clone() {
                        board.toggle_group_collapsed(gid, cx);
                    }
                }))
        };
        out.push(match group_id.clone() {
            Some(gid) => div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(block_w))
                .h(px(HEADER))
                .id(("group-header-drag", placed.item_index))
                .cursor_move()
                .on_drag(
                    gid,
                    move |id: &BoardItemId, offset, _window, cx: &mut App| {
                        let cursor = (
                            header_x + f32::from(offset.x),
                            header_y + f32::from(offset.y),
                        );
                        let ghost = weak
                            .update(cx, |board, cx| {
                                board.begin_item_drag(id.clone(), DraggedKind::Group, cursor, cx);
                                board.group_ghost(id.clone(), cx)
                            })
                            .ok();
                        cx.new(|_| {
                            ghost.unwrap_or_else(|| {
                                drag::DragGhost::group(
                                    id.clone(),
                                    String::new(),
                                    group_accent(None),
                                    String::new(),
                                )
                            })
                        })
                    },
                )
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
                        .child(spend_age.clone()),
                )
                .children(
                    badge
                        .clone()
                        .map(|t| div().text_color(gpui::rgb(0x8a8a94)).child(t)),
                )
                .child(caret)
                .into_any_element(),
            None => div()
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
                .cursor_move()
                .child(div().size(px(8.0)).rounded_full().bg(accent))
                .child(div().text_color(gpui::rgb(0xd6d6de)).child(name.clone()))
                .child(
                    div()
                        .flex_grow()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(spend_age.clone()),
                )
                .children(
                    badge
                        .clone()
                        .map(|t| div().text_color(gpui::rgb(0x8a8a94)).child(t)),
                )
                .child(caret)
                .into_any_element(),
        });

        let layout = self.replica.read(cx).layout();
        let in_group_target = drop_target.and_then(|t| {
            if t.parent.as_ref() == group_id.as_ref() {
                Some(t.ordinal)
            } else {
                None
            }
        });

        let reserved_slot = |k: usize| -> AnyElement {
            let cc = k % fc;
            let rr = k / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * (CARD_H + GAP);
            div()
                .absolute()
                .left(px(x - INSET + mx))
                .top(px(y - INSET + my))
                .w(px(CARD_W))
                .h(px(CARD_H))
                .rounded(px(8.0))
                .border_1()
                .border_color(gpui::rgb(0x4a4a54))
                .bg(gpui::rgb(0x1a1a22))
                .into_any_element()
        };

        if let Some(k) = in_group_target {
            out.push(reserved_slot(k));
        }

        // Members at full size in body-zones; shift right of the reserved slot.
        for (i, session) in sessions.iter().enumerate() {
            if hide_session == Some(session) {
                continue;
            }
            let render_index = in_group_target.map_or(i, |k| i + usize::from(i >= k));
            let cc = render_index % fc;
            let rr = render_index / fc;
            let mx = INSET + cc as f32 * CELL_W;
            let my = INSET + HEADER + rr as f32 * (CARD_H + GAP);
            let member_left = x - INSET + mx;
            let member_top = y - INSET + my;
            let member_item_id = layout
                .items
                .iter()
                .find(
                    |it| matches!(&it.kind, BoardItemKind::Card { session: s, .. } if s == session),
                )
                .map(|it| it.id.clone());
            let Some(member_item_id) = member_item_id else {
                continue;
            };
            if let Some(tile) =
                self.absolute_card(session, &member_item_id, member_left, member_top, cx)
            {
                out.push(tile);
            }
        }

        (out, snapshot)
    }

    /// A collapsed group (§7): a 1×1 tile reusing the group ring/accent/tint, with a
    /// header `● name · [spend · age] · ▸` and a body of status-rollup rows
    /// (`● N <label>` per non-empty wave), plus a `✓N done` footer rendered iff N>0.
    /// Members feed the rollup (read here) but are not rendered as cards.
    fn absolute_collapsed_group(
        &self,
        placed: &pack::Placed,
        sessions: &[SessionId],
        meta: Option<&GroupMeta>,
        now_ms: i64,
        cx: &mut Context<Self>,
    ) -> (AnyElement, GroupChromeSnapshot) {
        let x = placed.cell_left();
        let y = placed.cell_top();
        let block_w = CELL_W - GAP;
        let block_h = CELL_H - GAP;

        let name = meta.map(|m| m.name.clone()).unwrap_or_default();
        let completed = meta.map(|m| m.completed_count).unwrap_or(0);
        let accent = group_accent(meta.and_then(|m| m.color_token.as_deref()));
        let group_id = meta.map(|m| m.id.clone());

        // Narrow projections read from member cards: cost/age (spend·age) + Wave (rollup).
        let (member_costs, member_waves): (Vec<rollup::MemberCost>, Vec<Wave>) = {
            let fleet = self.fleet.read(cx);
            let mut costs = Vec::with_capacity(sessions.len());
            let mut waves = Vec::with_capacity(sessions.len());
            for s in sessions {
                if let Some(e) = fleet.cards.get(s) {
                    let card = e.read(cx);
                    costs.push(rollup::MemberCost::from_card(card));
                    waves.push(derive_wave(card, now_ms, false));
                }
            }
            (costs, waves)
        };
        let rollup = rollup::group_rollup(&member_costs, completed);
        let status = rollup::status_rollup(&member_waves);
        let spend_age = format!(
            "{} · {}",
            rollup::format_group_spend(rollup.spend_usd),
            rollup::format_age(rollup.oldest_created_at, now_ms),
        );
        // Same source as the expanded header (§5 "one rule, both sites"): the collapsed
        // footer's `✓ N done →` gates through `completed_badge` so a future threshold
        // change propagates to both chrome forms.
        let shows_completed = completed_badge(rollup.completed_count).is_some();

        let snapshot = GroupChromeSnapshot {
            session_ids: sessions.to_vec(),
            name: name.clone(),
            accent,
            rollup: rollup.clone(),
            spend_age: spend_age.clone(),
            shows_completed,
            collapsed: true,
            status_rows: status.rows.clone(),
        };

        let theme = cx.lens_theme();
        let mut body = div()
            .absolute()
            .left(px(x))
            .top(px(y + HEADER))
            .w(px(block_w))
            .h(px(block_h - HEADER))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px_1p5();
        for (w, n) in &status.rows {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .child(div().size(px(8.0)).rounded_full().bg(w.status_color(theme)))
                    .child(
                        div()
                            .text_color(gpui::rgb(0xc4c4cc))
                            .child(format!("{n} {}", wave_rollup_label(*w))),
                    ),
            );
        }
        body = body.children(shows_completed.then(|| {
            div()
                .text_color(gpui::rgb(0x8a8a94))
                .child(format!("✓ {} done →", rollup.completed_count))
        }));

        // Ring/tint + header. The `▸` caret is interactive (Task 6); mirrors expanded `⌄`.
        // Same GUTTER overhang as the expanded box so the border does not jump on toggle.
        let ring = div()
            .absolute()
            .left(px(x - GUTTER))
            .top(px(y - GUTTER))
            .w(px(block_w + 2.0 * GUTTER))
            .h(px(block_h + 2.0 * GUTTER))
            .rounded(px(12.0))
            .border_1()
            .border_color(accent)
            .bg(accent.opacity(0.12)); // SSOT color-mix ~12% body wash (matches expanded box)
        let header_x = x;
        let header_y = y;
        let weak = cx.weak_entity();
        let caret = {
            let gid = group_id.clone();
            div()
                .id(("group-caret", placed.item_index))
                .cursor_pointer()
                .text_color(gpui::rgb(0x8a8a94))
                .child("▸")
                .on_click(cx.listener(move |board, _ev, _win, cx| {
                    cx.stop_propagation();
                    if let Some(gid) = gid.clone() {
                        board.toggle_group_collapsed(gid, cx);
                    }
                }))
        };
        let header = match group_id.clone() {
            Some(gid) => div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(block_w))
                .h(px(HEADER))
                .id(("collapsed-group-drag", placed.item_index))
                .cursor_move()
                .on_drag(
                    gid,
                    move |id: &BoardItemId, offset, _window, cx: &mut App| {
                        let cursor = (
                            header_x + f32::from(offset.x),
                            header_y + f32::from(offset.y),
                        );
                        let ghost = weak
                            .update(cx, |board, cx| {
                                board.begin_item_drag(id.clone(), DraggedKind::Group, cursor, cx);
                                board.group_ghost(id.clone(), cx)
                            })
                            .ok();
                        cx.new(|_| {
                            ghost.unwrap_or_else(|| {
                                drag::DragGhost::group(
                                    id.clone(),
                                    String::new(),
                                    group_accent(None),
                                    String::new(),
                                )
                            })
                        })
                    },
                )
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
                        .child(spend_age.clone()),
                )
                .child(caret)
                .into_any_element(),
            None => div()
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
                .cursor_move()
                .child(div().size(px(8.0)).rounded_full().bg(accent))
                .child(div().text_color(gpui::rgb(0xd6d6de)).child(name.clone()))
                .child(
                    div()
                        .flex_grow()
                        .text_color(gpui::rgb(0x8a8a94))
                        .child(spend_age.clone()),
                )
                .child(caret)
                .into_any_element(),
        };

        let tile = div()
            .child(ring)
            .child(header)
            .child(body)
            .into_any_element();
        (tile, snapshot)
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

    /// Test hook: the deadline the collapsed-rollup repaint wake is armed for.
    pub fn armed_rollup_deadline_for_test(&self) -> Option<i64> {
        self.armed_rollup_deadline
    }

    /// Test hook: whether a live repaint-wake `Task` is actually held (a dropped task
    /// would be cancelled and never fire).
    pub fn rollup_wake_held_for_test(&self) -> bool {
        self.rollup_wake.is_some()
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

    /// Non-blocking banner copy from replica health (§5). `None` when healthy or dismissed.
    pub fn banner_text(&self, cx: &App) -> Option<String> {
        let replica = self.replica.read(cx);
        if replica.banner_dismissed() {
            return None;
        }
        match replica.state() {
            ReplicaState::Loading | ReplicaState::Writable => None,
            ReplicaState::Degraded => {
                Some("Some board items couldn't be read — changes won't save.".into())
            }
            ReplicaState::LoadFailed => {
                Some("Couldn't load your board — data on disk is untouched.".into())
            }
            ReplicaState::Stale => {
                let mut text = "Couldn't save — reconnecting.".to_string();
                let dropped = replica.dropped_writes();
                if dropped > 0 {
                    text.push_str(&format!(" ({dropped} change(s) not saved)."));
                }
                Some(text)
            }
        }
    }

    fn render_replica_banner(&self, text: String, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("replica-error-banner")
            .absolute()
            .top(px(8.0))
            .left(px(NAV_RAIL_W + 8.0))
            .right(px(8.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .bg(gpui::rgb(0x2a1f1f))
            .border_1()
            .border_color(gpui::rgb(0x8b3a3a))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_grow()
                    .text_color(gpui::rgb(0xf0d0d0))
                    .child(text),
            )
            .child(
                div()
                    .id("replica-banner-retry")
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .text_color(gpui::rgb(0xd6d6de))
                    .cursor_pointer()
                    .child("Retry")
                    .on_click(cx.listener(|board, _, _, cx| {
                        board.replica.update(cx, |r, cx| r.retry_recovery(cx));
                    })),
            )
            .child(
                div()
                    .id("replica-banner-dismiss")
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .text_color(gpui::rgb(0x8a8a94))
                    .cursor_pointer()
                    .child("Dismiss")
                    .on_click(cx.listener(|board, _, _, cx| {
                        board.replica.update(cx, |r, cx| {
                            r.dismiss_banner();
                            cx.notify();
                        });
                    })),
            )
    }
}

impl Render for BoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_card_views(cx);
        self.sync_chat_tab(cx);
        let mode = ShellMode::from_fleet(self.fleet.read(cx));
        let viewport = window.viewport_size();
        let viewport_h = f32::from(viewport.height);
        let viewport_w = f32::from(viewport.width);

        let (body, visible): (_, Vec<SessionId>) = match &mode {
            ShellMode::Board => {
                // True pane width, NOT clamped to CELL_W: `pane_width` inside pack_and_render
                // derives from this to center the block, so a floor here would lie about the
                // pane on a narrow window and apply a spurious rightward offset (right ring
                // then clips under the vertical-only scroll). `cols_for_width` already self-
                // clamps to ≥1 col, so no floor is needed for packing. (codex P2, 2026-07-22)
                let avail = viewport_w - NAV_RAIL_W - 2.0 * PAD - 2.0 * GUTTER;
                let max_cols = pack::max_cols_for_width(viewport_w);
                let (surface, visible) = self.pack_and_render(
                    avail,
                    viewport_h,
                    max_cols,
                    self.board_scroll.clone(),
                    cx,
                );
                let el = div()
                    .id("shell-board")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail(cx))
                    .child(div().flex_grow().h_full().child(surface));
                (el.into_any_element(), visible)
            }
            ShellMode::Focused { .. } => {
                let (rail, visible) = self.pack_and_render(
                    RAIL_W - 2.0 * PAD - 2.0 * GUTTER,
                    viewport_h,
                    // The rail is a single fixed column; the cap is a no-op (never narrows
                    // 1 col, never centers since content_extent == RAIL_W == pane_width).
                    usize::MAX,
                    self.rail_scroll.clone(),
                    cx,
                );
                // Real focused-transcript tab (T-2). `pack_and_render` above already took
                // `&mut self`, so build the chat slot after it to avoid a borrow overlap.
                let chat_slot = if let Some(tab) = &self.chat_tab {
                    div().id("chat-slot").flex_grow().child(tab.view.clone())
                } else {
                    div()
                        .id("chat-slot")
                        .flex_grow()
                        .child(div().id("chat-empty"))
                };
                let el = div()
                    .id("shell-focused")
                    .flex()
                    .flex_row()
                    .size_full()
                    .child(self.render_nav_rail(cx))
                    .child(div().w(px(RAIL_W)).flex_shrink_0().h_full().child(rail))
                    .child(chat_slot)
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
        self.arm_collapsed_rollup_wake(cx); // refresh collapsed rollups on Ready/Scheduled expiry
        let banner = self.banner_text(cx);
        let bg = cx.lens_theme().base.background;
        // In-app themed titlebar (drag region + traffic-light clearance handled by the
        // component) over a dark-filled window, replacing the white system titlebar strip.
        // min_h(0): without it this flex item's default `min-height:auto` = its content
        // height, so the masonry's full height leaks out and the inner scroll container can
        // never scroll (offset clamps to 0). min_h(0) lets flex_grow bound it to the window.
        let mut shell = div().flex_grow().min_h(px(0.0)).relative().child(body);
        if let Some(text) = banner {
            shell = shell.child(self.render_replica_banner(text, cx));
        }
        div()
            .id("board-view")
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .child(gpui_component::TitleBar::new())
            .child(shell)
    }
}

/// The `✓N` completed-badge text for group chrome (§5). `Some("✓{n}")` iff `n > 0`
/// (the unified "render ✓N iff N>0" rule — one source for both the painted string and
/// the snapshot bool); `None` suppresses the element. `completed_count` is Archive-side
/// (B-6), structurally 0 until then — this locks the rule for when B-6 makes it non-zero.
fn completed_badge(count: u32) -> Option<String> {
    (count > 0).then(|| format!("✓{count}"))
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

/// The prune set for a single **top-level** `board_tree` node: if it is a collapsed
/// group, every leaf session under it (those hide behind the 1×1 rollup tile). An
/// expanded group renders its members as cards (they need views), so nothing is
/// pruned — and we do NOT recurse into it. Recursing would prune a *nested* collapsed
/// group's members while `pack`/`absolute_group` (top-level tiles only) still render
/// them as cards → orphaned `visible` ids with no view (Grok T5 review Important #2).
/// Nested groups are unreachable until B-5; B-5 owns nested collapse rendering + prune
/// together. Keeping prune scope == pack scope (top-level tiles) makes B-4b consistent.
fn collect_collapsed_group_members(node: &BoardNode<'_>, out: &mut HashSet<SessionId>) {
    if let BoardNode::Group { item, .. } = node {
        let collapsed = matches!(
            &item.kind,
            lens_core::domain::board::BoardItemKind::Group {
                collapsed: true,
                ..
            }
        );
        if collapsed {
            for sid in node.leaf_sessions() {
                out.insert(sid.clone());
            }
        }
    }
}

/// Title-case label for a status-rollup row (§7). `Neutral` is never in a rollup
/// (excluded by `status_rollup`), so it maps to an empty label.
fn wave_rollup_label(w: Wave) -> &'static str {
    match w {
        Wave::NeedsInput => "Needs input",
        Wave::Failed => "Failed",
        Wave::Working => "Working",
        Wave::AwaitingReview => "Awaiting review",
        Wave::Scheduled => "Scheduled",
        Wave::Ready => "Ready",
        Wave::Slept => "Slept",
        Wave::Neutral => "",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::clock::{ManualUiClock, UiClock};
    use crate::slot::placeholder_tab;

    use super::*;

    fn test_fleet(cx: &mut gpui::App) -> Entity<FleetStore> {
        FleetStore::new(Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>, cx)
    }

    #[gpui::test]
    async fn banner_shows_for_load_failed(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx); // render root now reads lens_theme (bg + nav rail)
        });
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
        });
        cx.run_until_parked();
        let (board, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(
                fleet.clone(),
                replica.clone(),
                placeholder_tab(cx),
                None,
                cx,
            )
        });
        board.read_with(vcx, |b, cx| assert!(b.banner_text(cx).is_some()));

        let fleet_ok = cx.update(test_fleet);
        let writable = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet_ok.clone(), cx)));
        cx.run_until_parked();
        let (board_ok, vcx_ok) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet_ok, writable, placeholder_tab(cx), None, cx)
        });
        board_ok.read_with(vcx_ok, |b, cx| assert!(b.banner_text(cx).is_none()));
    }

    /// B-4a residual: prove off-screen animating cards hold NO anim timer *at scale*.
    /// The drop mechanism (`set_visible(false)` cancels the task) is unit-tested in
    /// `card::view`; this is the end-to-end container proof — that the band-culling gate
    /// actually flips the culled tiles hidden, so the live-timer set equals the visible
    /// band EXACTLY (no off-screen wakeups, no on-screen misses) across a scroll.
    #[gpui::test]
    async fn culled_animating_cards_hold_no_timer_at_scale(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::scalars::SessionStatusValue;

        // ≫ one 1920×1080 test-window band (~7 rows × 6 cols ≈ 42 tiles) → most are culled.
        const N: usize = 150;

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let (fleet, replica) = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                for i in 0..N {
                    // zero-padded id → deterministic sorted ordinals (row-major placement).
                    let sid = SessionId::new(format!("s{i:03}"));
                    let card = f.spawn_fake_session(sid, cx);
                    card.update(cx, |c, _| c.status = SessionStatusValue::Running); // animates
                }
            });
            let replica = cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx));
            (fleet, replica)
        });
        cx.run_until_parked(); // Load + reconcile places all N under conn_test.

        let (board, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(
                fleet.clone(),
                replica.clone(),
                placeholder_tab(cx),
                None,
                cx,
            )
        });
        vcx.run_until_parked();

        // (visible band, cards whose anim task is live) — one read of the settled frame.
        fn snapshot(
            board: &Entity<BoardView>,
            cx: &mut gpui::VisualTestContext,
        ) -> (HashSet<SessionId>, HashSet<SessionId>) {
            board.read_with(cx, |b, app| {
                let visible: HashSet<SessionId> =
                    b.visible_session_ids_for_test().into_iter().collect();
                let running: HashSet<SessionId> = b
                    .card_views_for_test()
                    .iter()
                    .filter(|(_, v)| v.read(app).timer_running_for_test())
                    .map(|(id, _)| id.clone())
                    .collect();
                (visible, running)
            })
        }

        let (v0, running0) = snapshot(&board, vcx);
        assert!(!v0.is_empty(), "some tiles must be in the visible band");
        assert!(
            v0.len() < N,
            "at scale most tiles must be culled (visible={} of {N})",
            v0.len()
        );
        // THE RESIDUAL: every Running card animates, so the live-timer set must equal the
        // visible set EXACTLY — no more (off-screen wakeups), no fewer (on-screen misses).
        assert_eq!(
            running0, v0,
            "anim timers must be exactly the in-band cards; off-screen ones hold none"
        );

        // Scroll deep so a disjoint band shows: top rows cull (DROP path), new rows enter
        // (SPAWN path). `set_offset` writes the shared scroll cell; notify re-renders.
        vcx.update(|_, cx| {
            board.update(cx, |b, cx| {
                b.board_scroll
                    .set_offset(gpui::point(gpui::px(0.0), gpui::px(-4000.0)));
                cx.notify();
            });
        });
        vcx.run_until_parked();

        let (v1, running1) = snapshot(&board, vcx);
        assert!(!v1.is_empty(), "band non-empty after scroll");
        assert_ne!(v1, v0, "scrolling must move the visible band");
        assert_eq!(
            running1, v1,
            "after scroll, live timers still track the band exactly"
        );

        let scrolled_off: Vec<&SessionId> = v0.difference(&v1).collect();
        assert!(
            !scrolled_off.is_empty(),
            "some top cards must have scrolled off"
        );
        for id in &scrolled_off {
            assert!(
                !running1.contains(*id),
                "card {} scrolled off-screen but kept its timer",
                id.as_str()
            );
        }
        let scrolled_in: Vec<&SessionId> = v1.difference(&v0).collect();
        assert!(
            !scrolled_in.is_empty(),
            "new cards must have entered the band"
        );
        for id in &scrolled_in {
            assert!(
                running1.contains(*id),
                "card {} entered the band but never spawned its timer",
                id.as_str()
            );
        }

        let _ = (fleet, replica);
    }

    #[gpui::test]
    async fn expanded_group_snapshot_suppresses_completed_when_zero(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
        use lens_core::domain::ids::{BoardId, ConnectionId};
        use lens_core::persist::{BoardStore, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                f.spawn_fake_session(SessionId::new("s1"), cx);
            });
            fleet
        });
        let conn = ConnectionId::new("conn_test");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        store
            .place_session(
                &conn,
                &SessionId::new("s1"),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g1.clone()),
                    ordinal: Some(0),
                },
            )
            .unwrap();
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();

        let (board_view, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx)
        });
        vcx.run_until_parked();

        board_view.read_with(vcx, |b, _| {
            let chrome = b.group_chrome_for_test();
            assert_eq!(chrome.len(), 1);
            // completed_count is Archive-side (B-6) → 0 → the ✓N element is suppressed.
            assert_eq!(chrome[0].rollup.completed_count, 0);
            assert!(!chrome[0].shows_completed, "✓N hidden when count is 0");
        });
    }

    #[test]
    fn completed_badge_renders_only_when_positive() {
        // The unified "✓N iff N>0" rule, both sides (closes the N>0 coverage the
        // retired `group_header_text` test used to hold — Grok T4 review Important #2).
        assert_eq!(completed_badge(0), None);
        assert_eq!(completed_badge(2), Some("✓2".to_string()));
        assert_eq!(completed_badge(1), Some("✓1".to_string()));
    }

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

    #[gpui::test]
    async fn collapsed_group_renders_1x1_and_excludes_members_from_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
        use lens_core::domain::ids::{BoardId, ConnectionId};
        use lens_core::domain::scalars::SessionStatusValue;
        use lens_core::persist::{BoardStore, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let conn = ConnectionId::new("conn_test");
        let (s1, s2) = (SessionId::new("s1"), SessionId::new("s2"));
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                for s in [&s1, &s2] {
                    let card = f.spawn_fake_session(s.clone(), cx);
                    card.update(cx, |c, _| c.status = SessionStatusValue::Running); // → Wave::Working
                }
            });
            fleet
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        for (i, s) in [&s1, &s2].into_iter().enumerate() {
            store
                .place_session(
                    &conn,
                    s,
                    &PlacementTarget {
                        board_id: Some(board.clone()),
                        parent_item_id: Some(g1.clone()),
                        ordinal: Some(i as i32),
                    },
                )
                .unwrap();
        }
        store.set_collapsed(&g1, true).unwrap(); // collapsed on disk
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();

        let (board_view, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx)
        });
        vcx.run_until_parked();

        board_view.read_with(vcx, |b, _| {
            let chrome = b.group_chrome_for_test();
            assert_eq!(chrome.len(), 1);
            assert!(chrome[0].collapsed, "group renders collapsed");
            // 2 Running members → one Working row, count 2.
            assert_eq!(
                chrome[0].status_rows,
                vec![(crate::card::wave::Wave::Working, 2)]
            );
            // THE FORK: collapsed members are NOT in the visible (card-view) set.
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert!(
                !visible.contains(&SessionId::new("s1"))
                    && !visible.contains(&SessionId::new("s2")),
                "collapsed members must be excluded from the visibility gate"
            );
            // and no card view is instantiated for them (no hidden-entity leak).
            let views = b.card_views_for_test();
            assert!(
                !views.contains_key(&SessionId::new("s1"))
                    && !views.contains_key(&SessionId::new("s2")),
                "no card view spawned for a collapsed member"
            );
        });
    }

    // codex final-review Important #1: a collapsed group's rollup has no per-card anim
    // timer, so the board must schedule a repaint at the earliest time-based wave
    // deadline. Here a collapsed `Ready` member (idle + just-completed) must arm the wake
    // for `last_completed_at + READY_DECAY_MS`. (The real-time timer FIRING vs the manual
    // test clock is the dual-clock limitation — we assert the ARMED deadline, as the card
    // anim tests assert `timer_running` rather than real firing.)
    #[gpui::test]
    async fn collapsed_ready_member_arms_rollup_wake_for_decay(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
        use lens_core::domain::ids::{BoardId, ConnectionId};
        use lens_core::persist::{BoardStore, SqliteBoardStore};

        const NOW: i64 = 10_000;
        let clock = Arc::new(ManualUiClock::new(NOW)) as Arc<dyn UiClock>;
        let conn = ConnectionId::new("conn_test");
        let s1 = SessionId::new("s1");
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                let card = f.spawn_fake_session(s1.clone(), cx);
                // Idle + just-completed + unfocused → Wave::Ready (decays at NOW + decay).
                card.update(cx, |c, _| c.last_completed_at = Some(NOW));
            });
            fleet
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        store
            .place_session(
                &conn,
                &s1,
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g1.clone()),
                    ordinal: Some(0),
                },
            )
            .unwrap();
        store.set_collapsed(&g1, true).unwrap();
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();

        let (board_view, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx)
        });
        vcx.run_until_parked();

        board_view.read_with(vcx, |b, _| {
            // The collapsed Ready member's rollup will go stale at decay unless a repaint
            // is scheduled for exactly that instant.
            assert_eq!(
                b.armed_rollup_deadline_for_test(),
                Some(NOW + crate::card::model::READY_DECAY_MS),
                "collapsed Ready member must arm the rollup wake for its decay deadline"
            );
            assert!(
                b.rollup_wake_held_for_test(),
                "a live repaint-wake Task must be held (a dropped one never fires)"
            );
        });
    }

    #[gpui::test]
    async fn toggle_group_collapsed_flips_render_and_visibility(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
        use lens_core::domain::ids::{BoardId, ConnectionId};
        use lens_core::domain::scalars::SessionStatusValue;
        use lens_core::persist::{BoardStore, SqliteBoardStore};

        let clock = Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>;
        let conn = ConnectionId::new("conn_test");
        let (s1, s2) = (SessionId::new("s1"), SessionId::new("s2"));
        let fleet = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            fleet.update(cx, |f, cx| {
                for s in [&s1, &s2] {
                    let card = f.spawn_fake_session(s.clone(), cx);
                    card.update(cx, |c, _| c.status = SessionStatusValue::Running);
                }
            });
            fleet
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        let store = SqliteBoardStore::open(&path).unwrap();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let g1 = store.create_group(&board, None, 0, "G").unwrap();
        for (i, s) in [&s1, &s2].into_iter().enumerate() {
            store
                .place_session(
                    &conn,
                    s,
                    &PlacementTarget {
                        board_id: Some(board.clone()),
                        parent_item_id: Some(g1.clone()),
                        ordinal: Some(i as i32),
                    },
                )
                .unwrap();
        }
        let boxed: Box<dyn BoardStore + Send> = Box::new(store);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::new(Some(boxed), path, conn, fleet.clone(), cx))
        });
        cx.run_until_parked();
        let (board_view, vcx) = cx.add_window_view(|_, cx| {
            BoardView::mount(fleet, replica, placeholder_tab(cx), None, cx)
        });
        vcx.run_until_parked();

        let collapsed = |b: &BoardView| b.group_chrome_for_test()[0].collapsed;
        board_view.read_with(vcx, |b, _| assert!(!collapsed(b), "starts expanded"));

        // Toggle → collapse (the caret closure calls exactly this).
        let gid = g1.clone();
        vcx.update(|_, cx| {
            board_view.update(cx, |b, cx| b.toggle_group_collapsed(gid.clone(), cx));
        });
        vcx.run_until_parked();
        board_view.read_with(vcx, |b, _| {
            assert!(collapsed(b), "collapsed after toggle");
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert!(
                visible.is_empty(),
                "collapsed members leave the visible set"
            );
            let views = b.card_views_for_test();
            assert!(
                !views.contains_key(&s1) && !views.contains_key(&s2),
                "collapse drops member card views (cancels anim timers)"
            );
        });

        // Toggle again → expand.
        vcx.update(|_, cx| {
            board_view.update(cx, |b, cx| b.toggle_group_collapsed(g1.clone(), cx));
        });
        vcx.run_until_parked();
        board_view.read_with(vcx, |b, _| {
            assert!(!collapsed(b), "expanded after second toggle");
            let visible: HashSet<SessionId> =
                b.visible_session_ids_for_test().into_iter().collect();
            assert_eq!(visible.len(), 2, "members visible again");
            let views = b.card_views_for_test();
            assert!(
                views.contains_key(&s1) && views.contains_key(&s2),
                "expand recreates member card views via sync_card_views"
            );
        });
    }
}
