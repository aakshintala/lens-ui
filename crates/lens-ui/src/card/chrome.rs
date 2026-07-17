use gpui::{
    App, AppContext, Context, Div, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Stateful, Styled, Window, div, prelude::*, px, relative, svg,
};
use lens_core::domain::usage::Cost;

use crate::theme::ActiveLensTheme;

use super::model::{ConnectionOverlay, RepoRef, SessionCard};
use super::motion::{
    render_sweep_overlay, render_working_spinner, wave_icon_path, wave_status_line,
};
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

/// Fill fraction (0.0..1.0) for the context-window progress bar.
fn ctx_fraction(context_window: Option<u64>, last_total_tokens: Option<u64>) -> f32 {
    match (context_window, last_total_tokens) {
        (Some(w), Some(t)) if w > 0 => (t as f32 / w as f32).clamp(0.0, 1.0),
        _ => 0.0,
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

fn render_icon_tile(wave: Wave, status: Hsla, now_ms: i64) -> Div {
    // Faint status-tinted surface (mockup: color-mix(status 14%, bg2)) — NOT a solid fill.
    let mut tile = div()
        .flex_shrink_0()
        .w(px(44.0))
        .h(px(44.0))
        .rounded(px(11.0))
        .bg(status.opacity(0.14))
        .border_1()
        .border_color(status.opacity(0.30))
        .flex()
        .items_center()
        .justify_center();
    if let Some(path) = wave_icon_path(wave) {
        tile = tile.child(svg().path(path).w(px(21.0)).h(px(21.0)).text_color(status));
    } else {
        tile = tile.child(render_working_spinner(status, now_ms));
    }
    tile
}

fn ellipsize_line(text: impl Into<SharedString>) -> Div {
    div().overflow_hidden().text_ellipsis().child(text.into())
}

/// A top-right pill button (Wake / Retry). `on_click` is the wired handler.
fn action_button(
    id: &'static str,
    label: &'static str,
    accent: Hsla,
    fg: Hsla,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .rounded(px(7.0))
        .px_2()
        .py(px(4.0))
        .text_xs()
        .text_color(fg)
        .bg(accent.opacity(0.30))
        .border_1()
        .border_color(accent.opacity(0.55))
        .on_click(on_click)
        .child(label)
}

struct ReposTooltip(String);

impl Render for ReposTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}

/// Card chrome inside the fixed 280×148 tile (§4.4 — reserved slots, no collapse).
// Single internal call site (card/view.rs); the five `on_*` handlers are distinct captured closures
// that don't bundle cleanly, and `cx` is needed for theme tokens — a struct here would only add noise.
#[allow(clippy::too_many_arguments)]
pub fn render_card_chrome(
    card: &SessionCard,
    wave: Wave,
    kebab_open: bool,
    sweep_phase: Option<f32>,
    now_ms: i64,
    cx: &App,
    on_kebab_toggle: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_wake: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_sleep: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_send: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    on_retry: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let t = cx.lens_theme();
    let border = wave.status_color(t);
    let popover = t.base.popover;
    let muted_fg = t.base.muted_foreground;
    let overlay_fg = t.base.foreground;
    let overlay_scrim = t.base.overlay.opacity(0.55);

    let title = card.title.clone().unwrap_or_else(|| "—".into());
    let harness_model = format_harness_model(card);
    let repos_row = format_repos_row(&card.repos);
    let repos_for_tooltip = card.repos.clone();
    let spend = format_spend(&card.cumulative_cost);
    let ctx_pct = format_ctx_pct(card.context_window, card.last_total_tokens);
    let ctx_frac = ctx_fraction(card.context_window, card.last_total_tokens);
    let pbar_track = gpui::white().opacity(0.06);
    let host = host_label(card);

    let dim = wave == Wave::Slept;
    let activity = if wave == Wave::Failed {
        card.last_task_error
            .as_ref()
            .map(|e| format!("✕ {}", e.message))
            .unwrap_or_else(|| "failed".into())
    } else {
        card.activity_summary.clone()
    };

    // Slept → bright Wake; Failed → Retry. Both sit top-right, full-opacity even when dimmed.
    let action: Option<Stateful<Div>> = match wave {
        Wave::Slept => Some(action_button(
            "card-wake",
            "Wake",
            t.status.slept,
            overlay_fg,
            on_wake,
        )),
        Wave::Failed => Some(action_button(
            "card-retry",
            "Retry",
            t.status.failed,
            overlay_fg,
            on_retry,
        )),
        _ => None,
    };

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

    // Header: 44px icon-tile + stacked status / title + kebab.
    let mut header = div()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .child(render_icon_tile(wave, border, now_ms).when(dim, |t| t.opacity(0.42)))
        .child(
            div()
                .flex_grow()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(border)
                        .child(wave_status_line(wave, card)),
                )
                .child(ellipsize_line(title))
                .when(dim, |c| c.opacity(0.42)),
        )
        .children(action)
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
                .bg(popover)
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
                .text_color(muted_fg)
                .when(dim, |d| d.opacity(0.42)),
        )
        .child(
            div()
                .id("card-activity")
                .h(px(16.0))
                .flex_shrink_0()
                .overflow_hidden()
                .child(ellipsize_line(if activity.is_empty() {
                    SharedString::from(" ")
                } else {
                    SharedString::from(activity.clone())
                }))
                .when(dim, |d| d.opacity(0.42)),
        )
        .child(
            ellipsize_line(repos_row)
                .id("card-repos")
                .text_xs()
                .tooltip({
                    move |_, cx| {
                        let tip = format_repos_tooltip(&repos_for_tooltip);
                        cx.new(|_| ReposTooltip(tip)).into()
                    }
                })
                .when(dim, |d| d.opacity(0.42)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_xs()
                .text_color(muted_fg)
                .child(ellipsize_line(host).max_w(px(80.0)))
                .child(ellipsize_line(spend))
                .child(ellipsize_line(ctx_pct))
                .when(dim, |d| d.opacity(0.42)),
        );

    root = root.child(
        div()
            .h(px(4.0))
            .w_full()
            .rounded(px(2.0))
            .overflow_hidden()
            .bg(pbar_track)
            .child(div().h_full().w(relative(ctx_frac)).bg(border)),
    );

    if let Some(phase) = sweep_phase {
        root = root.child(render_sweep_overlay(border, phase));
    }

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
                .bg(overlay_scrim)
                .flex()
                .items_center()
                .justify_center()
                .text_color(overlay_fg)
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

    #[test]
    fn ctx_fraction_clamps_and_ratios() {
        assert_eq!(ctx_fraction(Some(200_000), Some(50_000)), 0.25);
        assert_eq!(
            ctx_fraction(Some(100), Some(250)),
            1.0,
            "over-full clamps to 1"
        );
        assert_eq!(ctx_fraction(None, Some(10)), 0.0, "no window → 0");
        assert_eq!(
            ctx_fraction(Some(0), Some(10)),
            0.0,
            "zero window → 0 (no div-by-0)"
        );
        assert_eq!(ctx_fraction(Some(200_000), None), 0.0, "no tokens → 0");
    }
}
