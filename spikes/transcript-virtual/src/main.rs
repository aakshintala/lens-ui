#![allow(dead_code)]
#![allow(unused_variables)]

mod anchor;
mod app;
mod backend;
mod backend_a;
mod backend_b;
mod fixture;
mod probe;
mod row_render;
mod rowsource;

use gpui::{App, AppContext, Application, WindowOptions};
use gpui_component::Root;

use app::{HarnessView, register_keybindings};
use backend::BackendChoice;

fn parse_n() -> usize {
    let mut n = 200usize;
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--n=") {
            n = v.parse().unwrap_or(200);
        } else if let Ok(v) = arg.parse::<usize>()
            && arg.chars().all(|c| c.is_ascii_digit())
        {
            n = v;
        }
    }
    n
}

fn parse_handoff() -> bool {
    std::env::args().skip(1).any(|arg| arg == "--handoff")
}

fn main() {
    let handoff = parse_handoff();
    let n = if handoff { 40 } else { parse_n() };
    let backend = if handoff {
        BackendChoice::A
    } else {
        BackendChoice::parse()
    };
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        register_keybindings(cx);
        cx.open_window(WindowOptions::default(), move |window, cx| {
            let view = cx.new(|cx| HarnessView::new(backend, n, handoff, window, cx));
            let any: gpui::AnyView = view.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
