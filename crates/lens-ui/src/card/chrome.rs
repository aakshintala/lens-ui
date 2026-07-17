use gpui::{
    App, Div, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString, Stateful, Styled,
    Window, div, linear_color_stop, linear_gradient, prelude::*, px, relative, svg,
};
use lens_core::domain::usage::Cost;

use crate::theme::ActiveLensTheme;

use super::model::{ConnectionOverlay, RepoRef, SessionCard};
use super::motion::{
    countdown_fraction, format_wake_countdown, pulse_alpha, render_countdown_ring,
    render_sweep_overlay, render_working_spinner, wave_icon_path, wave_status_line,
};
use super::wave::Wave;

/// Lucide glyphs for the repo row (bundled in `assets/icons/`), tinted at the call site.
const FOLDER_ICON: &str = "icons/folder.svg";
const GIT_BRANCH_ICON: &str = "icons/git-branch.svg";

/// The `·+N` badge when a card spans multiple repos (N = extras beyond the primary shown inline).
fn repos_overflow_badge(count: usize) -> Option<String> {
    (count > 1).then(|| format!("·+{}", count - 1))
}

/// One `folder name git-branch branch` entry: tinted Lucide glyphs (in `icon_color`) with the
/// name/branch text inheriting the parent foreground. Shared by the inline row and the tooltip.
fn repo_entry(name: &str, branch: &str, icon_color: Hsla) -> Div {
    let glyph = |path: &'static str| {
        svg()
            .path(path)
            .w(px(13.0))
            .h(px(13.0))
            .flex_shrink_0()
            .text_color(icon_color)
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .overflow_hidden()
        .child(glyph(FOLDER_ICON))
        .child(ellipsize_line(name.to_string()))
        .child(glyph(GIT_BRANCH_ICON))
        .child(ellipsize_line(branch.to_string()))
}

/// Inline repo row: the primary repo as an icon entry, plus a `·+N` overflow badge.
fn render_repos_row(repos: &[RepoRef], icon_color: Hsla) -> Div {
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .overflow_hidden();
    let Some(primary) = repos.first() else {
        return row.child("—");
    };
    let branch = primary.branch.as_deref().unwrap_or("—");
    let mut row = row.child(repo_entry(&primary.name, branch, icon_color));
    if let Some(badge) = repos_overflow_badge(repos.len()) {
        row = row.child(badge);
    }
    row
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

fn render_icon_tile(wave: Wave, status: Hsla, now_ms: i64, countdown: Option<f32>) -> Div {
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
    if wave == Wave::Scheduled
        && let Some(frac) = countdown
    {
        tile = tile.child(render_countdown_ring(status, frac));
    }
    if let Some(path) = wave_icon_path(wave) {
        tile = tile.child(svg().path(path).w(px(21.0)).h(px(21.0)).text_color(status));
    } else {
        tile = tile.child(render_working_spinner(status, now_ms));
    }
    tile
}

fn ellipsize_line(text: impl Into<SharedString>) -> Div {
    // `overflow_hidden` (required for the ellipsis) clips ascenders when the line box is
    // tighter than the glyph box — gpui's `text_*` helpers set font-size but not
    // line-height. A comfortable relative line-height (× font-size) keeps every row legible.
    div()
        .overflow_hidden()
        .text_ellipsis()
        .line_height(relative(1.4))
        .child(text.into())
}

/// Monospace family for the "live machine data" lines (activity / error / countdown). A macOS
/// system font resolved via gpui's CoreText provider — not bundled.
const MONO: &str = "Menlo";

/// Ellipsized monospace line — the live/ephemeral type treatment. Flex child that truncates
/// (`min_w(0)` overrides the default auto min-width so the ellipsis engages inside the row).
fn mono_line(text: impl Into<SharedString>) -> Div {
    ellipsize_line(text)
        .font_family(MONO)
        .text_size(px(11.5))
        .flex_grow()
        .min_w(px(0.0))
}

/// The live/ephemeral activity row, styled per wave (spec §11): Working (or any live tool/todo) →
/// pulsing status dot + mono; Failed → pulsing ✕ + mono error; Scheduled → status-colored mono
/// countdown; other waves carry no live text → reserved blank slot (STATUS eyebrow carries it).
/// The dot/✕ pulse rides the card's existing re-render (Working/Failed animate).
fn render_activity(wave: Wave, text: &str, status: Hsla, now_ms: i64, dim: bool) -> Stateful<Div> {
    let row = div()
        .id("card-activity")
        .min_h(px(16.0))
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .overflow_hidden()
        .when(dim, |d| d.opacity(0.42));
    if text.is_empty() {
        return row;
    }
    let pulse = pulse_alpha(now_ms);
    match wave {
        Wave::Failed => row
            .child(
                div()
                    .flex_none()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(status)
                    .opacity(pulse)
                    .child("✕"),
            )
            .child(mono_line(text.to_string())),
        Wave::Scheduled => row.child(mono_line(text.to_string()).text_color(status)),
        _ => row
            .child(
                div()
                    .flex_none()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(status)
                    .opacity(pulse),
            )
            .child(mono_line(text.to_string())),
    }
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
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
        .child(label)
}

/// Per-wave wash over the card body (status-colored fill behind the content).
/// - Sweep waves → `Gradient` (pairs with the moving sweep = the "wave effect").
/// - Working/Scheduled → `Flat` uniform tint (a static gradient reads dull with no sweep).
/// - Neutral/Idle → `Flat` but very faint.
/// - Slept → `None` (dim + colored outline only).
enum Wash {
    None,
    Flat(f32),
    Gradient { peak: f32, spread: f32 },
}

// pane-ui-match intensities (tunable end-of-pass). Gradient spreads full-body (1.0), not a corner.
const WASH_GRADIENT_ALPHA: f32 = 0.24;
const WASH_GRADIENT_SPREAD: f32 = 1.0;
const WASH_FLAT_ALPHA: f32 = 0.14;
const WASH_FAINT_ALPHA: f32 = 0.08;

fn wave_wash(wave: Wave) -> Wash {
    match wave {
        Wave::NeedsInput | Wave::Failed | Wave::AwaitingReview | Wave::Ready => Wash::Gradient {
            peak: wash_tunable("LENS_CARD_WASH", WASH_GRADIENT_ALPHA),
            spread: wash_tunable("LENS_CARD_WASH_SPREAD", WASH_GRADIENT_SPREAD),
        },
        Wave::Working | Wave::Scheduled => {
            Wash::Flat(wash_tunable("LENS_CARD_WASH_FLAT", WASH_FLAT_ALPHA))
        }
        Wave::Neutral => Wash::Flat(wash_tunable("LENS_CARD_WASH_FAINT", WASH_FAINT_ALPHA)),
        Wave::Slept => Wash::None,
    }
}

/// Apply the wave's wash as the card background. `status` is the wave's status color.
fn apply_wash(root: Div, wave: Wave, status: Hsla) -> Div {
    match wave_wash(wave) {
        Wash::None => root,
        Wash::Flat(a) => root.bg(status.opacity(a)),
        Wash::Gradient { peak, spread } => root.bg(linear_gradient(
            135.0,
            linear_color_stop(status.opacity(peak), 0.0),
            linear_color_stop(status.opacity(0.0), spread),
        )),
    }
}

/// Demo-only wash-intensity override (`_var`); the shipped build uses `default`.
fn wash_tunable(_var: &str, default: f32) -> f32 {
    #[cfg(feature = "demo")]
    {
        if let Some(v) = std::env::var(_var)
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| (0.0..=1.0).contains(v))
        {
            return v;
        }
    }
    default
}

/// Card chrome inside the fixed 280×160 tile (§4.4 — reserved slots, no collapse).
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

    let has_title = card.title.is_some();
    let title = card.title.clone().unwrap_or_else(|| "—".into());
    let harness_model = format_harness_model(card);
    let repos_for_tooltip = card.repos.clone();
    let spend = format_spend(&card.cumulative_cost);
    let ctx_pct = format_ctx_pct(card.context_window, card.last_total_tokens);
    let ctx_frac = ctx_fraction(card.context_window, card.last_total_tokens);
    let pbar_track = gpui::white().opacity(0.06);
    // Context-window bar: colored by utilization, not the card's wave color —
    // green ≤50%, amber ≤75%, red above. A budget signal independent of status.
    let pbar_fill = if ctx_frac <= 0.50 {
        t.base.success
    } else if ctx_frac <= 0.75 {
        t.base.warning
    } else {
        t.base.danger
    };
    let host = host_label(card);

    let dim = wave == Wave::Slept;
    let activity = if wave == Wave::Failed {
        // Bare message — the ✕ marker is rendered (and pulsed) separately in `render_activity`.
        card.last_task_error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "failed".into())
    } else {
        card.activity_summary.clone()
    };

    let countdown = countdown_fraction(card.scheduled_started_at, card.scheduled_wake_at, now_ms);
    // Scheduled activity line = the live countdown (overrides activity_summary).
    let activity = if wave == Wave::Scheduled {
        card.scheduled_wake_at
            .map(|w| format_wake_countdown(w.saturating_sub(now_ms)))
            .unwrap_or_else(|| activity.clone())
    } else {
        activity
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
        // Breathing room between the card's horizontal rows (header / activity / repos / meta).
        .gap(px(6.0))
        .rounded_md()
        .border_2()
        .border_color(border)
        .overflow_hidden();
    // Status-colored wash behind the content, per-wave (gradient / flat / none). The gradient
    // is a 135° top-left→bottom-right linear approximation of the SSOT corner radial (gpui has
    // no radial); pane-ui-match uses full spread so it covers the body, not just the corner.
    root = apply_wash(root, wave, border);

    // Header: 44px icon-tile + stacked status / title + kebab.
    let mut header = div()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .child(
            // Tile + title stack as one group so the fixed 44px tile centers against the
            // (now 3-line) stack; action/kebab stay top-aligned in the outer header.
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_grow()
                // No overflow_hidden here: the countdown ring's `inset(-4)` canvas extends past
                // the tile and must not be clipped. Text truncation is handled by the title
                // column's own overflow_hidden below.
                .child(
                    render_icon_tile(wave, border, now_ms, countdown)
                        .when(dim, |t| t.opacity(0.42)),
                )
                .child(
                    div()
                        .flex_grow()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(border)
                                .child(wave_status_line(wave, card)),
                        )
                        .child({
                            let title_tip = title.clone();
                            ellipsize_line(title)
                                .id("card-title")
                                // Ellipsized single line → reveal the full title on hover.
                                .when(has_title, move |el| {
                                    el.tooltip(move |window, cx| {
                                        let full = title_tip.clone();
                                        gpui_component::tooltip::Tooltip::element(move |_, _| {
                                            div().child(full.clone())
                                        })
                                        .build(window, cx)
                                    })
                                })
                        })
                        // Harness · model aligns under the title (mockup `.model` sits in the header meta).
                        // Smallest tier: title (base) > status (text_xs) > harness·model (10px).
                        .child(
                            ellipsize_line(harness_model)
                                .text_size(px(10.0))
                                .text_color(muted_fg),
                        )
                        .when(dim, |c| c.opacity(0.42)),
                ),
        )
        .children(action)
        .child(
            div()
                .id("card-kebab")
                .cursor_pointer()
                .on_click(move |ev, window, cx| {
                    cx.stop_propagation();
                    on_kebab_toggle(ev, window, cx);
                })
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
        .child(render_activity(wave, &activity, border, now_ms, dim))
        .child(
            render_repos_row(&card.repos, muted_fg)
                .id("card-repos")
                .text_xs()
                // Tooltip only earns its keep when repos overflow (the `·+N` badge hides the
                // rest); for a single repo it just duplicates the visible line.
                .when(repos_for_tooltip.len() > 1, |el| {
                    el.tooltip(move |window, cx| {
                        // The component's themed popover box (bg/border/shadow) wraps one icon
                        // entry per repo — same glyphs as the inline row.
                        let repos = repos_for_tooltip.clone();
                        gpui_component::tooltip::Tooltip::element(move |_, _| {
                            let mut col = div().flex().flex_col().gap(px(3.0)).text_xs();
                            for r in &repos {
                                let branch = r.branch.as_deref().unwrap_or("—");
                                col = col.child(repo_entry(&r.name, branch, muted_fg));
                            }
                            col
                        })
                        .build(window, cx)
                    })
                })
                .when(dim, |d| d.opacity(0.42)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .text_xs()
                .text_color(muted_fg)
                // Host label as a pill (SSOT `.hostpill`): rounded, faint surface + border.
                .child(
                    ellipsize_line(host)
                        .max_w(px(80.0))
                        .px(px(6.0))
                        .py(px(1.0))
                        .rounded(px(6.0))
                        .bg(t.base.muted)
                        .border_1()
                        .border_color(t.base.border)
                        .text_color(t.base.foreground),
                )
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
            .child(div().h_full().w(relative(ctx_frac)).bg(pbar_fill))
            .when(dim, |d| d.opacity(0.42)),
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
    fn overflow_badge_counts_extras_beyond_primary() {
        // 3 repos → primary shown inline, `·+2` for the two extras.
        assert_eq!(repos_overflow_badge(3), Some("·+2".into()));
        assert_eq!(repos_overflow_badge(2), Some("·+1".into()));
    }

    #[test]
    fn overflow_badge_absent_for_zero_or_one_repo() {
        assert_eq!(repos_overflow_badge(1), None, "single repo: no badge");
        assert_eq!(repos_overflow_badge(0), None, "no repos: no badge");
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
