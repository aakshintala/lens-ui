use gpui::{AnyView, App, Context, IntoElement, Render, SharedString, Window, div, prelude::*};

pub trait ContentTab {}

pub struct TabHandle {
    pub view: AnyView,
    pub title: SharedString,
}

impl TabHandle {
    pub fn set_title(&mut self, title: SharedString) {
        self.title = title;
    }
}

pub struct PlaceholderTab;

impl ContentTab for PlaceholderTab {}

impl Render for PlaceholderTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Working area (placeholder)")
    }
}

pub fn placeholder_tab(cx: &mut App) -> TabHandle {
    let view = cx.new(|_| PlaceholderTab);
    TabHandle {
        view: view.into(),
        title: SharedString::from("Placeholder"),
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
