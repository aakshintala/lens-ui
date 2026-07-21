//! The custom scrollable surface — the B-2 container spike proper.
//!
//! - **Unknown 1 (scroll):** absolute-positioned tiles inside an
//!   `overflow_scroll` div whose single in-flow child has explicit
//!   `content_height` → establishes the scroll extent; read `ScrollHandle`
//!   offset each frame.
//! - **Unknown 2 (cull):** build only tiles whose y-range intersects the
//!   visible band (+ overdraw). Culled cards are simply absent from the child
//!   vec → gpui never builds them (proven by their `render_count`).
//! - **Unknown 3 (timer gate):** the container owns the visible set and pushes
//!   `set_visible` to member cards **via `App::defer`** — off its own render
//!   path, so it never reads sibling card entities during render (the
//!   viewport-reentry-freeze `.cached()` dirty-tracking landmine). A card
//!   scrolled back into view respawns its timer here → freeze fixed at root.

use std::collections::HashSet;

use gpui::{
    AnyView, App, Context, IntoElement, ParentElement, Render, StyleRefinement, Styled, Window,
    div, prelude::*, px, rgb,
};

use crate::card::SpikeCard;
use crate::packer::{
    CARD_H, CARD_W, CELL_H, CELL_W, GAP, HEADER, INSET, Kind, Placed, cols_for_width, pack,
};

/// One packed tile plus the leaf card-ids it owns.
struct Tile {
    placed: Placed,
    /// Card-ids of the leaf cards this tile renders (1 for a loose card, N for
    /// a group's members). These share the tile's y-range for cull/gate.
    card_ids: Vec<usize>,
    group_color: u32,
}

pub struct Container {
    tiles: Vec<Tile>,
    cards: Vec<gpui::Entity<SpikeCard>>,
    scroll: gpui::ScrollHandle,
    content_height: f32,
    viewport_h: f32,
    overdraw: f32,
    /// Card-ids currently gated visible (timer allowed to run).
    gated_visible: HashSet<usize>,

    /// When false (`--all-timers` measurement mode) culling AND gating are off:
    /// every tile is built and every card's timer runs. The A/B baseline.
    cull_enabled: bool,

    // ---- container-owned HUD state (never reads card entities in render) ----
    last_scroll_top: f32,
    built_tiles: usize,

    probe_mode: bool,
    probe_spawned: bool,
}

impl Container {
    pub fn new(cull_enabled: bool, probe_mode: bool, cx: &mut Context<Self>) -> Self {
        // Fixture: the SSOT's 7-item set repeated to build a surface several
        // viewports tall (so most tiles are off-screen → cull/measure signal).
        const REPEATS: usize = 8;
        // (member_count) — 1 means a loose card; >1 a group.
        const BASE: [usize; 7] = [1, 4, 1, 1, 2, 1, 3];

        let mut items = Vec::new();
        let mut cards: Vec<gpui::Entity<SpikeCard>> = Vec::new();
        let mut tile_card_ids: Vec<Vec<usize>> = Vec::new();
        let mut tile_colors: Vec<u32> = Vec::new();

        for rep in 0..REPEATS {
            for (bi, &n) in BASE.iter().enumerate() {
                let mut ids = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = cards.len();
                    // Most cards animate; every 5th is static (a Ready/Slept
                    // card) — shows the gate is animating-aware, not just
                    // visibility-aware.
                    let animating = id % 5 != 0;
                    cards.push(cx.new(|_| SpikeCard::new(id, animating)));
                    ids.push(id);
                }
                items.push(if n == 1 {
                    crate::packer::Item::card()
                } else {
                    crate::packer::Item::group(n)
                });
                tile_card_ids.push(ids);
                tile_colors.push(group_color(rep * BASE.len() + bi));
            }
        }

        // 3 columns at the SSOT window width (avail ≈ 940 → cols = 3).
        let cols = cols_for_width(940.0);
        let packing = pack(&items, cols);

        let tiles = packing
            .tiles
            .into_iter()
            .map(|placed| Tile {
                card_ids: tile_card_ids[placed.item_index].clone(),
                group_color: tile_colors[placed.item_index],
                placed,
            })
            .collect();

        Self {
            tiles,
            cards,
            scroll: gpui::ScrollHandle::new(),
            content_height: packing.content_height,
            viewport_h: 640.0,
            overdraw: CELL_H, // one cell of margin covers the one-frame offset lag
            gated_visible: HashSet::new(),
            cull_enabled,
            last_scroll_top: 0.0,
            built_tiles: 0,
            probe_mode,
            probe_spawned: false,
        }
    }

    // --- probe accessors (read off the render path) ---
    pub fn scroll_handle(&self) -> gpui::ScrollHandle {
        self.scroll.clone()
    }
    pub fn card(&self, id: usize) -> gpui::Entity<SpikeCard> {
        self.cards[id].clone()
    }
    pub fn card_count(&self) -> usize {
        self.cards.len()
    }
    pub fn built_tiles(&self) -> usize {
        self.built_tiles
    }
    pub fn total_tiles(&self) -> usize {
        self.tiles.len()
    }
    pub fn content_height(&self) -> f32 {
        self.content_height
    }
    pub fn viewport_h(&self) -> f32 {
        self.viewport_h
    }

    fn mount(card: &gpui::Entity<SpikeCard>) -> AnyView {
        // Mirror `mount_cached_card`: pin the cached wrapper to the fixed tile
        // size so the cache key is bounds-stable regardless of parent layout.
        let style = StyleRefinement::default().w(px(CARD_W)).h(px(CARD_H));
        AnyView::from(card.clone()).cached(style)
    }
}

impl Render for Container {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.probe_mode && !self.probe_spawned {
            self.probe_spawned = true;
            cx.spawn_in(window, |weak, wcx: &mut gpui::AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    crate::probe::run_probe(weak, wcx).await;
                }
            })
            .detach();
        }

        // Last frame's offset (updated during paint). One-frame lag is why we
        // carry an overdraw margin.
        let scroll_top = (-f32::from(self.scroll.offset().y)).max(0.0);
        let lo = scroll_top - self.overdraw;
        let hi = scroll_top + self.viewport_h + self.overdraw;

        let mut content = div()
            .relative()
            .w(px(CELL_W * 3.0))
            .h(px(self.content_height));

        let mut want_visible: HashSet<usize> = HashSet::new();
        let mut built = 0usize;

        for tile in &self.tiles {
            let visible =
                !self.cull_enabled || (tile.placed.cell_bottom() >= lo && tile.placed.cell_top() <= hi);
            if !visible {
                continue;
            }
            built += 1;
            for &id in &tile.card_ids {
                want_visible.insert(id);
            }

            match tile.placed.item.kind {
                Kind::Card => {
                    let x = tile.placed.cell_left();
                    let y = tile.placed.cell_top() + HEADER; // body-zone
                    content = content.child(
                        div()
                            .absolute()
                            .left(px(x))
                            .top(px(y))
                            .w(px(CARD_W))
                            .h(px(CARD_H))
                            .child(Self::mount(&self.cards[tile.card_ids[0]])),
                    );
                }
                Kind::Group { members } => {
                    let (fc, fr) = (tile.placed.item.fc, tile.placed.item.fr);
                    let x = tile.placed.cell_left();
                    let y = tile.placed.cell_top();
                    let block_w = fc as f32 * CELL_W - GAP;
                    let block_h = fr as f32 * CELL_H - GAP;

                    let mut ring = div()
                        .absolute()
                        .left(px(x - INSET))
                        .top(px(y - INSET))
                        .w(px(block_w + 2.0 * INSET))
                        .h(px(block_h + 2.0 * INSET))
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(rgb(tile.group_color))
                        .child(
                            div()
                                .absolute()
                                .left(px(INSET + 8.0))
                                .top(px(INSET + 4.0))
                                .text_color(rgb(tile.group_color))
                                .child(format!("● group ✓ ({members})")),
                        );

                    for (i, &id) in tile.card_ids.iter().enumerate() {
                        let cc = i % fc;
                        let rr = i / fc;
                        let mx = INSET + cc as f32 * CELL_W;
                        let my = INSET + HEADER + rr as f32 * CELL_H;
                        ring = ring.child(
                            div()
                                .absolute()
                                .left(px(mx))
                                .top(px(my))
                                .w(px(CARD_W))
                                .h(px(CARD_H))
                                .child(Self::mount(&self.cards[id])),
                        );
                    }
                    content = content.child(ring);
                }
            }
        }

        // --- Unknown 3: apply timer gate OFF the render path via App::defer ---
        // The gate always runs; culling only changes *which* cards are in
        // `want_visible` (the visible band vs. all tiles in `--all-timers`).
        if want_visible != self.gated_visible {
            let newly_vis: Vec<usize> =
                want_visible.difference(&self.gated_visible).copied().collect();
            let newly_hid: Vec<usize> =
                self.gated_visible.difference(&want_visible).copied().collect();
            let cards = self.cards.clone();
            self.gated_visible = want_visible;
            cx.defer(move |app: &mut App| {
                for id in newly_vis {
                    cards[id].update(app, |c, cx| c.set_visible(true, cx));
                }
                for id in newly_hid {
                    cards[id].update(app, |c, cx| c.set_visible(false, cx));
                }
            });
        }

        self.last_scroll_top = scroll_top;
        self.built_tiles = built;

        let hud = format!(
            "scroll_top {:>5.0} / {:.0}   tiles built {}/{}   viewport {:.0}   overdraw {:.0}   cull {}",
            scroll_top,
            self.content_height,
            built,
            self.tiles.len(),
            self.viewport_h,
            self.overdraw,
            if self.cull_enabled { "ON" } else { "OFF (--all-timers)" },
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0c0c0f))
            .text_color(rgb(0xcccccc))
            .child(
                div()
                    .flex_none()
                    .h(px(40.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .bg(rgb(0x000000))
                    .child(hud),
            )
            .child(
                div()
                    .id("board-scroll")
                    .flex_none()
                    .h(px(self.viewport_h))
                    .w_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .child(content),
            )
    }
}

fn group_color(seed: usize) -> u32 {
    const C: [u32; 4] = [0x60a5fa, 0xfb923c, 0x4ade80, 0xa78bfa];
    C[seed % C.len()]
}
