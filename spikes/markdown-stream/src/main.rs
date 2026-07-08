mod replay;
mod sanitize;

// Task 1 — dependency-feasibility probe + static markdown render.
// Goal: confirm gpui-component 0.5.1 builds and opens a window rendering
// markdown via `TextView::markdown`. See NOTES.md for the discovered API.

use gpui::{div, prelude::*, App, Application, Context, Window, WindowOptions};
use gpui_component::{text::TextView, Root};

const SAMPLE: &str = "# Hello\n\nSome **bold** and a list:\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[a link](https://example.com)\n";

struct MdView;

impl Render for MdView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_4()
            .child(TextView::markdown("md", SAMPLE, window, cx))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|_| MdView);
            let any: gpui::AnyView = view.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
