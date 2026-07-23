use gpui::{
    div, prelude::*, px, App, IntoElement, ParentElement, SharedString, Styled, Window,
};
use crate::focused::RowContent;
use crate::md::MarkdownView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReasoningExpand {
    #[default]
    Collapsed,
    Summary,
    Full,
}

pub type ReasoningSetExpandFn = Box<dyn Fn(ReasoningExpand, &mut App) + 'static>;

pub enum ReasoningUiState {
    Collapsed { duration_secs: Option<u32> },
    SummaryExpanded,
    FullExpanded,
    Encrypted { duration_secs: Option<u32> },
}

pub fn reasoning_collapsed_label(encrypted: bool, duration_secs: Option<u32>) -> String {
    if encrypted {
        match duration_secs {
            Some(s) => format!("🔒 thought for {s}s · reasoning hidden"),
            None => "🔒 thought · reasoning hidden".into(),
        }
    } else if let Some(s) = duration_secs {
        format!("💭 thought for {s}s")
    } else {
        "💭 thought".into()
    }
}

/// Body text for expanded reasoning rows — summary (or full if empty) vs full chain-of-thought.
fn expanded_body<'a>(
    state: ReasoningUiState,
    summary: &'a str,
    full: &'a str,
) -> &'a str {
    match state {
        ReasoningUiState::FullExpanded => full,
        ReasoningUiState::SummaryExpanded => {
            if summary.is_empty() {
                full
            } else {
                summary
            }
        }
        _ => summary,
    }
}

pub fn render_reasoning(
    content: &RowContent,
    ui_state: ReasoningUiState,
    on_set_expand: Option<ReasoningSetExpandFn>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let RowContent::Reasoning {
        summary,
        full,
        encrypted,
        duration_secs,
        content_key,
        live,
    } = content
    else {
        return div().into_any_element();
    };

    if *encrypted {
        return div()
            .child(reasoning_collapsed_label(true, *duration_secs))
            .into_any_element();
    }

    if *live {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().child("💭 thinking…"))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "reason-live-{}",
                        content_key.as_element_id()
                    )))
                    .max_h(px(120.))
                    .overflow_hidden()
                    .child(
                        MarkdownView::new(content_key.as_element_id(), full.clone(), window, cx)
                            .scrollable(true)
                            .selectable(true)
                            .into_inner(),
                    ),
            )
            .into_any_element();
    }

    match ui_state {
        ReasoningUiState::Collapsed { duration_secs } => {
            let label = reasoning_collapsed_label(false, duration_secs);
            let base = div().child(label);
            if let Some(on_set_expand) = on_set_expand {
                base.id(SharedString::from(format!(
                    "reasoning-expand-{}",
                    content_key.as_element_id()
                )))
                .cursor_pointer()
                .on_click(move |_, _, cx| on_set_expand(ReasoningExpand::Summary, cx))
                .into_any_element()
            } else {
                base.into_any_element()
            }
        }
        ReasoningUiState::SummaryExpanded => {
            let body = expanded_body(ui_state, summary, full);
            let expand_link = div().child("show full reasoning ↗");
            let expand_link = if let Some(on_set_expand) = on_set_expand {
                expand_link
                    .id(SharedString::from(format!(
                        "reasoning-show-full-{}",
                        content_key.as_element_id()
                    )))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| on_set_expand(ReasoningExpand::Full, cx))
            } else {
                expand_link.id(SharedString::from(format!(
                    "reasoning-show-full-{}",
                    content_key.as_element_id()
                )))
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().child(reasoning_collapsed_label(false, *duration_secs)))
                .child(expand_link)
                .child(
                    MarkdownView::new(content_key.as_element_id(), body.to_owned(), window, cx)
                        .scrollable(false)
                        .selectable(true)
                        .into_inner(),
                )
                .into_any_element()
        }
        ReasoningUiState::FullExpanded => {
            let body = expanded_body(ui_state, summary, full);
            let collapse_link = div().child("show summary ↖");
            let collapse_link = if let Some(on_set_expand) = on_set_expand {
                collapse_link
                    .id(SharedString::from(format!(
                        "reasoning-show-summary-{}",
                        content_key.as_element_id()
                    )))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| on_set_expand(ReasoningExpand::Summary, cx))
            } else {
                collapse_link.id(SharedString::from(format!(
                    "reasoning-show-summary-{}",
                    content_key.as_element_id()
                )))
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().child(reasoning_collapsed_label(false, *duration_secs)))
                .child(collapse_link)
                .child(
                    MarkdownView::new(content_key.as_element_id(), body.to_owned(), window, cx)
                        .scrollable(false)
                        .selectable(true)
                        .into_inner(),
                )
                .into_any_element()
        }
        ReasoningUiState::Encrypted { duration_secs } => div()
            .child(reasoning_collapsed_label(true, duration_secs))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focused::ContentKey;

    #[test]
    fn encrypted_label_includes_duration() {
        let label = reasoning_collapsed_label(true, Some(3));
        assert_eq!(label, "🔒 thought for 3s · reasoning hidden");
    }

    #[test]
    fn collapsed_label_without_duration() {
        let label = reasoning_collapsed_label(false, None);
        assert_eq!(label, "💭 thought");
    }

    #[test]
    fn collapsed_label_with_duration_from_row_content() {
        let content = RowContent::Reasoning {
            summary: "sum".into(),
            full: "full".into(),
            encrypted: false,
            duration_secs: Some(4),
            content_key: ContentKey::from_label("r1"),
            live: false,
        };
        let label = match &content {
            RowContent::Reasoning {
                duration_secs, encrypted, ..
            } => reasoning_collapsed_label(*encrypted, *duration_secs),
            _ => panic!("expected reasoning"),
        };
        assert!(label.contains("4s"), "label should contain durable duration_secs: {label}");
    }

    #[test]
    fn expand_advances_collapsed_summary_full() {
        assert_eq!(ReasoningExpand::default(), ReasoningExpand::Collapsed);
        assert_ne!(ReasoningExpand::Collapsed, ReasoningExpand::Summary);
        assert_ne!(ReasoningExpand::Summary, ReasoningExpand::Full);
    }

    #[test]
    fn full_expanded_renders_full_not_summary() {
        assert_eq!(
            expanded_body(
                ReasoningUiState::FullExpanded,
                "short",
                "LONG"
            ),
            "LONG"
        );
        assert_eq!(
            expanded_body(
                ReasoningUiState::SummaryExpanded,
                "short",
                "LONG"
            ),
            "short"
        );
        assert_eq!(
            expanded_body(
                ReasoningUiState::SummaryExpanded,
                "",
                "LONG"
            ),
            "LONG"
        );
    }
}
