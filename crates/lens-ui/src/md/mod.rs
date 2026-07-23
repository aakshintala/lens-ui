#![allow(clippy::all, warnings)]

mod format;
mod global_state;
mod inline;
mod node;
mod style;
mod text_view;
mod utils;

use gpui::App;

pub use style::*;
pub use text_view::TextView;

pub fn init(cx: &mut App) {
    global_state::init(cx);
    text_view::init(cx);
}
