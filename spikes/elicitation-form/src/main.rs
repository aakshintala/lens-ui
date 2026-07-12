#![allow(dead_code)]
#![allow(unused_variables)]

mod app;
mod ask_user_question;
mod elicitation_card;
mod fixtures;
mod probe;
mod raw_editor;
mod schema;
mod schema_form;

use gpui::{App, AppContext as _, Application, WindowOptions};
use gpui_component::Root;

use app::{HarnessView, register_keybindings};

fn main() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        // NumberInput::init is not re-exported from gpui_component::input (private mod).
        register_keybindings(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| HarnessView::new(window, cx));
            let any: gpui::AnyView = view.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
