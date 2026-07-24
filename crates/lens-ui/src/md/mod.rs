#![allow(clippy::all, warnings)]

mod format;
mod global_state;
mod inline;
mod node;
mod stream_flag;
mod style;
pub(crate) mod text_view;
mod utils;

use gpui::{App, Entity, EntityId, ListOffset, SharedString, Window};
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

#[cfg(feature = "probe")]
pub fn markdown_probe_arm_selection(id: &str, window: &mut Window, cx: &mut App) {
    let key = SharedString::from(format!("{id}/state"));
    let state = window.use_keyed_state::<text_view::TextViewState>(key, cx, |_, cx| {
        text_view::TextViewState::new(cx)
    });
    state.update(cx, |s, _| {
        s.set_selection_for_test(gpui::point(gpui::px(1.), gpui::px(1.)));
    });
}

#[cfg(feature = "probe")]
pub fn markdown_probe_selection_is_some(id: &str, window: &mut Window, cx: &mut App) -> bool {
    let key = SharedString::from(format!("{id}/state"));
    let state = window.use_keyed_state::<text_view::TextViewState>(key, cx, |_, cx| {
        text_view::TextViewState::new(cx)
    });
    state.read(cx).selection_is_some_for_test()
}

#[cfg(feature = "probe")]
fn markdown_probe_state(
    id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<text_view::TextViewState> {
    let key = SharedString::from(format!("{id}/state"));
    window.use_keyed_state::<text_view::TextViewState>(key, cx, |_, cx| {
        text_view::TextViewState::new(cx)
    })
}

#[cfg(feature = "probe")]
pub fn markdown_probe_logical_scroll_top(id: &str, window: &mut Window, cx: &mut App) -> ListOffset {
    markdown_probe_state(id, window, cx)
        .read(cx)
        .list_logical_scroll_top()
}

#[cfg(feature = "probe")]
pub fn markdown_probe_list_item_count(id: &str, window: &mut Window, cx: &mut App) -> usize {
    markdown_probe_state(id, window, cx).read(cx).list_item_count()
}

#[cfg(feature = "probe")]
pub fn markdown_probe_scroll_list_to(id: &str, offset: ListOffset, window: &mut Window, cx: &mut App) {
    markdown_probe_state(id, window, cx).update(cx, |s, _| s.list_scroll_to(offset));
}

/// Opaque handle to a markdown row's keyed `TextViewState`, captured DURING render/paint
/// (where `use_keyed_state` is legal) so a probe driver can read scroll state BETWEEN frames
/// via the entity — entity reads/updates do not require paint context (unlike `use_keyed_state`,
/// which panics outside request_layout/prepaint/paint).
#[cfg(feature = "probe")]
#[derive(Clone)]
pub struct MarkdownProbeHandle(Entity<text_view::TextViewState>);

/// MUST be called during render/paint (uses `use_keyed_state`).
#[cfg(feature = "probe")]
pub fn markdown_probe_handle(id: &str, window: &mut Window, cx: &mut App) -> MarkdownProbeHandle {
    MarkdownProbeHandle(markdown_probe_state(id, window, cx))
}

/// Callable between frames (no paint context needed).
#[cfg(feature = "probe")]
pub fn markdown_probe_handle_scroll_top(handle: &MarkdownProbeHandle, cx: &mut App) -> ListOffset {
    handle.0.read(cx).list_logical_scroll_top()
}

#[cfg(feature = "probe")]
pub fn markdown_probe_handle_item_count(handle: &MarkdownProbeHandle, cx: &mut App) -> usize {
    handle.0.read(cx).list_item_count()
}

#[cfg(feature = "probe")]
pub fn markdown_probe_handle_scroll_to(
    handle: &MarkdownProbeHandle,
    offset: ListOffset,
    cx: &mut App,
) {
    handle.0.update(cx, |s, _| s.list_scroll_to(offset));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_prefix_closes_bold() {
        assert_eq!(safe_prefix("**wor"), "**wor**");
    }
}
