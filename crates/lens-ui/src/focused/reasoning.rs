use gpui::{
    div, prelude::*, px, App, IntoElement, ParentElement, SharedString, Styled, Window,
};
use crate::focused::RowContent;
use crate::md::MarkdownView;

pub type ReasoningExpandFn = Box<dyn Fn(&mut App) + 'static>;

pub enum ReasoningUiState {
    LiveExpanded,
    Collapsed { duration_secs: Option<u32> },
    SummaryExpanded,
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

pub fn render_reasoning(
    content: &RowContent,
    ui_state: ReasoningUiState,
    on_expand: Option<ReasoningExpandFn>,
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
            if let Some(on_expand) = on_expand {
                base.id(SharedString::from(format!(
                    "reasoning-expand-{}",
                    content_key.as_element_id()
                )))
                .cursor_pointer()
                .on_click(move |_, _, cx| on_expand(cx))
                .into_any_element()
            } else {
                base.into_any_element()
            }
        }
        ReasoningUiState::SummaryExpanded | ReasoningUiState::LiveExpanded => {
            let body = if summary.is_empty() { full } else { summary };
            let expand_link = div().child("show full reasoning ↗");
            let expand_link = if let Some(on_expand) = on_expand {
                expand_link
                    .id(SharedString::from(format!(
                        "reasoning-show-full-{}",
                        content_key.as_element_id()
                    )))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| on_expand(cx))
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
                    MarkdownView::new(content_key.as_element_id(), body.clone(), window, cx)
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
}
