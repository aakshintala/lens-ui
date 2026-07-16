use gpui::{
    App, AppContext, Context, Div, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::usage::Cost;

use super::model::{ConnectionOverlay, RepoRef, SessionCard};
use super::wave::Wave;

pub fn format_repos_row(repos: &[RepoRef]) -> String {
    if repos.is_empty() {
        return "—".into();
    }
    let primary = &repos[0];
    let branch = primary.branch.as_deref().unwrap_or("—");
    let mut row = format!("📁 {} ⑂ {}", primary.name, branch);
    if repos.len() > 1 {
        row.push_str(&format!(" ·+{}", repos.len() - 1));
    }
    row
}

pub fn format_repos_tooltip(repos: &[RepoRef]) -> String {
    if repos.is_empty() {
        return "—".into();
    }
    repos
        .iter()
        .map(|r| {
            let branch = r.branch.as_deref().unwrap_or("—");
            format!("📁 {} ⑂ {}", r.name, branch)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_label(status: SessionStatusValue) -> &'static str {
    match status {
        SessionStatusValue::Idle => "IDLE",
        SessionStatusValue::Launching => "LAUNCHING",
        SessionStatusValue::Running => "RUNNING",
        SessionStatusValue::Waiting => "WAITING",
        SessionStatusValue::Failed => "FAILED",
        SessionStatusValue::Unknown => "UNKNOWN",
    }
}

fn format_harness_model(card: &SessionCard) -> String {
    let harness = card.harness.as_deref().unwrap_or("—");
    let model = card
        .model_override
        .as_deref()
        .or(card.llm_model.as_deref())
        .unwrap_or("—");
    format!("{harness} · {model}")
}

fn format_spend(cost: &Cost) -> String {
    match cost.total_cost_usd {
        Some(usd) => format!("~${usd:.2}"),
        None => "—".into(),
    }
}

fn format_ctx_pct(context_window: Option<u64>, last_total_tokens: Option<u64>) -> String {
    match (context_window, last_total_tokens) {
        (Some(w), Some(t)) if w > 0 => format!("{}%", (t.saturating_mul(100) / w).min(100)),
        _ => "—".into(),
    }
}

fn host_label(card: &SessionCard) -> String {
    card.host_id
        .as_ref()
        .map(|h| h.as_str().to_string())
        .or_else(|| card.workspace.clone())
        .or_else(|| card.agent_name.clone())
        .unwrap_or_else(|| "—".into())
}

pub fn wave_border_color(wave: Wave) -> Hsla {
    match wave {
        Wave::NeedsInput => gpui::rgb(0xf59e0b),
        Wave::Ready => gpui::rgb(0x22c55e),
        Wave::Working => gpui::rgb(0x3b82f6),
        Wave::Failed => gpui::rgb(0xef4444),
        Wave::Slept => gpui::rgb(0x6b7280),
        Wave::Neutral => gpui::rgb(0x374151),
    }
    .into()
}

/// Short state label for the colored status pill.
fn wave_label(wave: Wave, status: SessionStatusValue) -> &'static str {
    match wave {
        Wave::NeedsInput => "NEEDS INPUT",
        Wave::Ready => "READY",
        Wave::Working => "WORKING",
        Wave::Failed => "FAILED",
        Wave::Slept => "SLEPT",
        Wave::Neutral => status_label(status),
    }
}

/// Contrasting text color for the filled pill (dark on bright waves, light on grey).
fn pill_text_color(wave: Wave) -> Hsla {
    match wave {
        Wave::Neutral | Wave::Slept => gpui::rgb(0xe5e7eb).into(),
        _ => gpui::rgb(0x0b1220).into(),
    }
}

fn ellipsize_line(text: impl Into<SharedString>) -> Div {
    div().overflow_hidden().text_ellipsis().child(text.into())
}

struct ReposTooltip(String);

impl Render for ReposTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}

/// Card chrome inside the fixed 280×148 tile (§4.4 — reserved slots, no collapse).
pub fn render_card_chrome(
    card: &SessionCard,
    wave: Wave,
    kebab_open: bool,
    on_kebab_toggle: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_sleep: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_send: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_retry: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let title = card.title.clone().unwrap_or_else(|| "—".into());
    let harness_model = format_harness_model(card);
    let repos_row = format_repos_row(&card.repos);
    let repos_for_tooltip = card.repos.clone();
    let spend = format_spend(&card.cumulative_cost);
    let ctx_pct = format_ctx_pct(card.context_window, card.last_total_tokens);
    let host = host_label(card);

    let activity = if wave == Wave::Failed {
        "Retry".into()
    } else {
        card.activity_summary.clone()
    };

    let border = wave_border_color(wave);
    let mut root = div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .p_2()
        .gap_1()
        .rounded_md()
        .border_2()
        .border_color(border)
        .overflow_hidden();

    // Header: a filled state pill (wave color + label) + title + kebab.
    let mut header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_shrink_0()
                .px_2()
                .py(px(1.0))
                .rounded_full()
                .bg(border)
                .text_color(pill_text_color(wave))
                .text_xs()
                .child(wave_label(wave, card.status)),
        )
        .child(div().flex_grow().overflow_hidden().child(ellipsize_line(title)))
        .child(
            div()
                .id("card-kebab")
                .cursor_pointer()
                .on_click(on_kebab_toggle)
                .child("⋮"),
        );

    if kebab_open {
        header = header.child(
            div()
                .absolute()
                .top(px(20.0))
                .right(px(4.0))
                .flex()
                .flex_col()
                .bg(gpui::rgb(0x1f2937))
                .rounded_md()
                .p_1()
                .gap_1()
                .child(
                    div()
                        .id("card-kebab-sleep")
                        .cursor_pointer()
                        .on_click(on_sleep)
                        .child("Sleep"),
                )
                .child(
                    div()
                        .id("card-kebab-send")
                        .cursor_pointer()
                        .on_click(on_send)
                        .child("Send"),
                ),
        );
    }

    root = root
        .child(header)
        .child(
            ellipsize_line(harness_model)
                .text_xs()
                .text_color(gpui::rgb(0x9ca3af)),
        )
        .child({
            let mut activity_slot = div()
                .id("card-activity")
                .h(px(16.0))
                .flex_shrink_0()
                .overflow_hidden()
                .child(ellipsize_line(if activity.is_empty() {
                    SharedString::from(" ")
                } else {
                    SharedString::from(activity.clone())
                }));
            if wave == Wave::Failed {
                activity_slot = activity_slot.cursor_pointer().on_click(on_retry);
            }
            activity_slot
        })
        .child(
            ellipsize_line(repos_row)
                .id("card-repos")
                .text_xs()
                .tooltip({
                    move |_, cx| {
                        let tip = format_repos_tooltip(&repos_for_tooltip);
                        cx.new(|_| ReposTooltip(tip)).into()
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_xs()
                .text_color(gpui::rgb(0x9ca3af))
                .child(ellipsize_line(host).max_w(px(80.0)))
                .child(ellipsize_line(spend))
                .child(ellipsize_line(ctx_pct)),
        );

    if card.connection_overlay != ConnectionOverlay::Connected {
        let label = match card.connection_overlay {
            ConnectionOverlay::Reconnecting => "Reconnecting…",
            ConnectionOverlay::Disconnected => "Disconnected",
            ConnectionOverlay::Connected => "",
        };
        root = root.child(
            div()
                .absolute()
                .inset_0()
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.55))
                .flex()
                .items_center()
                .justify_center()
                .text_color(gpui::rgb(0xf3f4f6))
                .child(label),
        );
    }

    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_render_one_row_with_overflow_badge() {
        let row = format_repos_row(&[
            RepoRef {
                name: "a".into(),
                branch: Some("main".into()),
            },
            RepoRef {
                name: "b".into(),
                branch: None,
            },
            RepoRef {
                name: "c".into(),
                branch: None,
            },
        ]);
        assert!(row.contains("·+2"), "overflow badge: {row}");
        assert!(!row.contains('\n'));
    }

    #[test]
    fn repos_empty_shows_dash() {
        assert_eq!(format_repos_row(&[]), "—");
    }
}
