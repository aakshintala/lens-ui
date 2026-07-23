use std::cell::RefCell;

thread_local! {
    static SINK: RefCell<Vec<ContentUiEvent>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigateToFile {
    pub path: String,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentUiEvent {
    NavigateToFile(NavigateToFile),
}

pub fn emit_navigate_to_file(path: String, line: Option<u32>, _cx: &mut gpui::App) {
    SINK.with(|s| {
        s.borrow_mut()
            .push(ContentUiEvent::NavigateToFile(NavigateToFile { path, line }));
    });
}

#[cfg(test)]
pub fn take_events() -> Vec<ContentUiEvent> {
    SINK.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    async fn emit_navigate_to_file_records_event(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            emit_navigate_to_file("src/parser.rs".into(), Some(42), cx);
            let events = take_events();
            assert_eq!(
                events,
                vec![ContentUiEvent::NavigateToFile(NavigateToFile {
                    path: "src/parser.rs".into(),
                    line: Some(42),
                })]
            );
        });
    }
}
