//! A minimal animated card entity that mirrors the *timer + visibility* shape
//! of the real `card/view.rs` — the only card behaviour the container/culling
//! spike needs to reason about. It carries an anim timer (self-notify loop),
//! a container-driven visibility gate, and instrumentation counters. Rendered
//! `.cached()` exactly like production so caching/dirty-tracking behaves the
//! same (memory: viewport-reentry-freeze — `.cached()` re-renders only on
//! dirty_views OR bounds-change).

use std::time::Duration;

use gpui::{
    Context, IntoElement, ParentElement, Render, Rgba, Styled, Task, Window, div, px, rgb,
};

/// 20fps, matching the shipped wave anim driver (memory: wave-perf).
const TICK: Duration = Duration::from_millis(50);

pub struct SpikeCard {
    pub id: usize,
    color: Rgba,
    /// Does this card *want* to animate (mirrors `anim_tick_for(wave).is_some()`
    /// — a Working/attention card does; a Ready/Slept card doesn't).
    animating: bool,

    /// Container-driven visibility gate (replaces the paint-time `last_bounds`
    /// gate). Timer runs iff `visible && animating`.
    visible: bool,
    anim_task: Option<Task<()>>,

    // ---- instrumentation (read by the probe, off the render path) ----
    /// Times the anim timer fired (advances only while visible+animating).
    pub tick_count: u64,
    /// Times `render` was called (proves culling: a culled card never renders).
    pub render_count: u64,
}

impl SpikeCard {
    pub fn new(id: usize, animating: bool) -> Self {
        // Init HIDDEN: the container is the sole visibility authority. Its first
        // cull pass flips the truly-visible cards to `set_visible(true)`, which
        // starts their timers. (If we init visible=true, that call early-returns
        // and the timer never spawns.)
        SpikeCard {
            id,
            color: palette(id),
            animating,
            visible: false,
            anim_task: None,
            tick_count: 0,
            render_count: 0,
        }
    }

    pub fn animating(&self) -> bool {
        self.animating
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn timer_running(&self) -> bool {
        self.anim_task.is_some()
    }

    /// Container calls this (via `cx.defer`, off its own render path) when a
    /// tile enters/leaves the visible band. Starts/stops the anim timer at the
    /// root — a card scrolled back into view respawns its driver here, which is
    /// exactly what the old edge-triggered gate failed to do (the freeze).
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        self.sync_timer(cx);
        cx.notify();
    }

    fn sync_timer(&mut self, cx: &mut Context<Self>) {
        let want = self.visible && self.animating;
        if want && self.anim_task.is_none() {
            self.anim_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    if this
                        .update(cx, |card, cx| {
                            card.tick_count = card.tick_count.wrapping_add(1);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        } else if !want {
            self.anim_task = None; // drop cancels the timer
        }
    }

}

impl Render for SpikeCard {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count = self.render_count.wrapping_add(1);
        let badge = if self.visible { "VIS" } else { "HID" };
        let tick = self.tick_count;
        // Pulse the border alpha off the tick so the animation is visible to the
        // eye and its running/stopped state is obvious on screen.
        let pulse = if self.timer_running() && (tick % 8) < 4 {
            0xffffff
        } else {
            0x000000
        };
        div()
            .size_full()
            .rounded(px(10.0))
            .bg(rgb(0x1a1a1e))
            .border_2()
            .border_color(self.color)
            .p(px(10.0))
            .child(
                div()
                    .text_color(rgb(0xffffff))
                    .child(format!("#{} · {}", self.id, badge)),
            )
            .child(
                div()
                    .text_color(rgb(pulse))
                    .child(format!("tick {tick}")),
            )
            .child(
                div()
                    .text_color(rgb(0x777777))
                    .child(format!("paints {}", self.render_count)),
            )
    }
}

fn palette(id: usize) -> Rgba {
    const COLORS: [u32; 6] = [
        0x4ade80, // green
        0xfb923c, // orange
        0x60a5fa, // blue
        0xf87171, // red
        0x9ca3af, // gray
        0xa78bfa, // violet
    ];
    rgb(COLORS[id % COLORS.len()])
}
