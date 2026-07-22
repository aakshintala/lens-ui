use crate::focused::{FocusedTranscript, view::mount_focused_transcript_view};
use gpui::{
    AnyView, App, Context, Entity, FocusHandle, InteractiveElement, IntoElement, Render,
    SharedString, Window, div, prelude::*,
};

pub trait ContentTab {}

pub struct TabHandle {
    pub view: AnyView,
    pub title: SharedString,
    pub focus_handle: FocusHandle,
}

impl TabHandle {
    pub fn set_title(&mut self, title: SharedString) {
        self.title = title;
    }
}

pub struct PlaceholderTab {
    focus_handle: FocusHandle,
}

impl ContentTab for PlaceholderTab {}

impl PlaceholderTab {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Render for PlaceholderTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .child("Working area (placeholder)")
    }
}

pub fn focused_transcript_tab(replica: Entity<FocusedTranscript>, cx: &mut App) -> TabHandle {
    let (view, focus_handle) = mount_focused_transcript_view(replica, cx);
    TabHandle {
        view: view.into(),
        title: SharedString::from("chat"),
        focus_handle,
    }
}

pub fn placeholder_tab(cx: &mut App) -> TabHandle {
    let entity = cx.new(PlaceholderTab::new);
    let focus_handle = entity.read(cx).focus_handle.clone();
    TabHandle {
        view: entity.into(),
        title: SharedString::from("Placeholder"),
        focus_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn tab_handle_title_is_updatable(cx: &mut gpui::TestAppContext) {
        let mut handle = cx.update(placeholder_tab);
        assert_eq!(handle.title.as_ref(), "Placeholder");
        handle.set_title(SharedString::from("Updated"));
        assert_eq!(handle.title.as_ref(), "Updated");
    }
}
