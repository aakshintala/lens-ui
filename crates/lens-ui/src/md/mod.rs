#![allow(clippy::all, warnings)]

mod format;
mod global_state;
mod inline;
mod node;
mod stream_flag;
mod style;
pub(crate) mod text_view;
mod utils;

use gpui::{App, EntityId, SharedString, Window};
use mdstitch::{stitch, StitchOptions};

pub use style::*;
pub use text_view::TextView;

pub fn init(cx: &mut App) {
    global_state::init(cx);
    text_view::init(cx);
}

pub fn safe_prefix(text: &str) -> String {
    stitch(text, &StitchOptions::default()).into_owned()
}

pub struct MarkdownView {
    inner: TextView,
}

impl MarkdownView {
    pub fn new(
        id: impl Into<SharedString>,
        markdown: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        Self {
            // TextView::markdown takes `impl Into<ElementId>`; SharedString: Into<ElementId>.
            inner: TextView::markdown(id.into(), markdown, window, cx),
        }
    }

    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.inner = self.inner.scrollable(scrollable);
        self
    }

    pub fn selectable(mut self, selectable: bool) -> Self {
        self.inner = self.inner.selectable(selectable);
        self
    }

    pub fn into_inner(self) -> TextView {
        self.inner
    }
}

pub fn markdown_state_entity_id(
    id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<EntityId> {
    let key = SharedString::from(format!("{id}/state"));
    let state = window.use_keyed_state::<text_view::TextViewState>(
        key,
        cx,
        |_, cx| text_view::TextViewState::new(cx),
    );
    Some(state.entity_id())
}

pub fn markdown_probe_arm_selection(id: &str, window: &mut Window, cx: &mut App) {
    let key = SharedString::from(format!("{id}/state"));
    let state = window.use_keyed_state::<text_view::TextViewState>(key, cx, |_, cx| {
        text_view::TextViewState::new(cx)
    });
    state.update(cx, |s, _| {
        s.set_selection_for_test(gpui::point(gpui::px(1.), gpui::px(1.)));
        s.mark_streaming_for_test();
    });
}

pub fn markdown_probe_selection_is_some(id: &str, window: &mut Window, cx: &mut App) -> bool {
    let key = SharedString::from(format!("{id}/state"));
    let state = window.use_keyed_state::<text_view::TextViewState>(key, cx, |_, cx| {
        text_view::TextViewState::new(cx)
    });
    state.read(cx).selection_is_some_for_test()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_prefix_closes_bold() {
        assert_eq!(safe_prefix("**wor"), "**wor**");
    }
}
