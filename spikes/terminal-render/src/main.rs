//! Spike A — GPUI terminal-grid paint viability harness.
//!
//! Task 1: blank window with a solid background via `canvas` / `paint_quad`.

use gpui::{
    Application, Bounds, Context, IntoElement, Render, Window, WindowOptions, canvas, fill,
    point, prelude::*, px, rgb, size,
};

struct GridView;

impl GridView {
    fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        canvas(
            |_bounds, _window, _cx| {},
            |bounds, _prepaint, window, _cx| {
                window.paint_quad(fill(bounds, rgb(0x1a1b26)));
                // Tiny sentinel quad so a blank window is visually confirmable.
                let sentinel = Bounds::new(
                    point(bounds.origin.x + px(16.0), bounds.origin.y + px(16.0)),
                    size(px(48.0), px(48.0)),
                );
                window.paint_quad(fill(sentinel, rgb(0x7aa2f7)));
            },
        )
        .size_full()
    }
}

fn main() {
    Application::new().run(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| GridView::new(window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
