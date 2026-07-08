#![allow(dead_code)] // Phase 0 skeleton — modules wired in Phase 1/2.

mod fixture;
mod probe;
mod rowsource;

use gpui::{div, prelude::*, App, Application, Context, Window, WindowOptions};
use gpui_component::Root;

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
            let any: gpui::AnyView = cx.new(|_| EmptyView).into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
