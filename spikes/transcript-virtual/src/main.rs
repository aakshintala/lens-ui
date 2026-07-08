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

use app::{register_keybindings, HarnessView};
use backend::BackendChoice;

fn parse_n() -> usize {
    let mut n = 200usize;
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--n=") {
            n = v.parse().unwrap_or(200);
        } else if let Ok(v) = arg.parse::<usize>() {
            if arg.chars().all(|c| c.is_ascii_digit()) {
                n = v;
            }
        }
    }
    n
}

fn main() {
    let n = parse_n();
    let backend = BackendChoice::parse();
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        register_keybindings(cx);
        cx.open_window(WindowOptions::default(), move |window, cx| {
            let view = cx.new(|cx| HarnessView::new(backend, n, window, cx));
            let any: gpui::AnyView = view.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
