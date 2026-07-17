use std::time::Duration;

use gpui::{Hsla, IntoElement, ParentElement, Styled, div, linear_color_stop, linear_gradient, px};

use super::model::{CARD_WIDTH_PX, SessionCard};
use super::wave::Wave;

/// Sweep period for waves that use the attention band (§3 motion sheet).
pub fn sweep_period(wave: Wave) -> Option<Duration> {
    match wave {
        Wave::NeedsInput | Wave::Failed => Some(Duration::from_secs_f32(1.0)),
        Wave::AwaitingReview | Wave::Ready => Some(Duration::from_secs_f32(1.5)),
        Wave::Working | Wave::Scheduled | Wave::Slept | Wave::Neutral => None,
    }
}

/// Sweep phase (0..1) for `wave` at `now_ms` — pure fn of the clock (approach ②,
/// deterministically testable). `None` for non-sweep waves.
pub fn sweep_phase(wave: Wave, now_ms: i64) -> Option<f32> {
    // Modulo in i64 FIRST: epoch-millis (~1.8e12) exceeds f32's ~24-bit mantissa, so
    // `now_ms as f32` quantizes to ~131k-ms steps and the phase would freeze. The small
    // remainder casts exactly.
    let period_ms = (sweep_period(wave)?.as_secs_f32() * 1000.0) as i64;
    Some(now_ms.rem_euclid(period_ms) as f32 / period_ms as f32)
}

/// True for any wave that runs a per-frame animation (drives the 30fps timer).
pub fn wave_animates(wave: Wave) -> bool {
    sweep_period(wave).is_some()
}

const RING_PERIOD_MS: i64 = 2400;
/// Expanding-ring phase (0..1) for the loud pair. i64 modulo first (see `sweep_phase`).
pub fn ring_phase(now_ms: i64) -> f32 {
    now_ms.rem_euclid(RING_PERIOD_MS) as f32 / RING_PERIOD_MS as f32
}

// Peak alpha of the moving highlight — soft so text stays legible.
const SWEEP_ALPHA: f32 = 0.20;
// Gradient angle: 90° = horizontal feather across the band width (the diagonal lives
// in the band shape once we move to a canvas path; div version stays vertical).
const SWEEP_ANGLE: f32 = 90.0;

/// Clipped sweep overlay positioned at `phase` (0..1). No `with_animation` — the card
/// view's 30fps timer re-renders us with a fresh clock-derived phase (frame-capped,
/// occlusion-cheap). Two half-bands fake a two-sided feather (gpui gradients are 2-stop).
pub fn render_sweep_overlay(status_color: Hsla, phase: f32) -> impl IntoElement {
    let band_w = CARD_WIDTH_PX * 0.42;
    let card_w = CARD_WIDTH_PX;
    let peak = status_color.opacity(SWEEP_ALPHA);
    let edge = status_color.opacity(0.0);
    let left = -band_w + phase * (card_w + band_w);

    div().absolute().size_full().overflow_hidden().child(
        div()
            .absolute()
            .top_0()
            .h_full()
            .w(px(band_w))
            .left(px(left))
            .flex()
            .flex_row()
            .child(div().h_full().flex_1().bg(linear_gradient(
                SWEEP_ANGLE,
                linear_color_stop(edge, 0.0),
                linear_color_stop(peak, 1.0),
            )))
            .child(div().h_full().flex_1().bg(linear_gradient(
                SWEEP_ANGLE,
                linear_color_stop(peak, 0.0),
                linear_color_stop(edge, 1.0),
            ))),
    )
}

/// Working-state indicator — STATIC placeholder for the spike (a rotating spinner needs
/// a bundled SVG asset, which the crate does not ship; deferred to the build's asset
/// task). Rendered as a static ring so Working doesn't run its own animation loop.
pub fn render_working_spinner(color: Hsla) -> impl IntoElement {
    div()
        .size(px(22.0))
        .rounded_full()
        .border_2()
        .border_color(color.opacity(0.6))
}

/// Expanding ring outside the card clip (NeedsInput / Failed), positioned at `phase`.
pub fn render_expanding_ring(wave: Wave, status_color: Hsla, phase: f32) -> impl IntoElement {
    let mut slot = div().absolute().size_full();
    if !matches!(wave, Wave::NeedsInput | Wave::Failed) {
        return slot;
    }
    let inset = -2.0 + phase * -10.0;
    let opacity = 0.9 * (1.0 - phase);
    slot = slot.child(
        div()
            .absolute()
            .inset(px(inset))
            .rounded_md()
            .border_2()
            .border_color(status_color)
            .opacity(opacity),
    );
    slot
}

/// Lucide glyph asset path for the tile, tinted at the call site via `text_color`.
/// `None` for `Working` — it renders the rotating spinner instead (see `render_working_spinner`).
pub fn wave_icon_path(wave: Wave) -> Option<&'static str> {
    Some(match wave {
        Wave::NeedsInput => "icons/bell.svg",
        Wave::Failed => "icons/triangle-alert.svg",
        Wave::AwaitingReview => "icons/circle-dot.svg",
        Wave::Scheduled => "icons/alarm-clock.svg",
        Wave::Ready => "icons/check.svg",
        Wave::Slept => "icons/moon.svg",
        Wave::Neutral => "icons/coffee.svg",
        Wave::Working => return None,
    })
}

/// Short status line beside the tile glyph.
pub fn wave_status_line(wave: Wave, card: &SessionCard) -> &'static str {
    use lens_core::domain::scalars::SessionStatusValue;
    match wave {
        Wave::NeedsInput => "NEEDS INPUT",
        Wave::Ready => "READY",
        Wave::Working => "WORKING",
        Wave::Failed => "FAILED",
        Wave::Slept => "SLEPT",
        Wave::AwaitingReview => "AWAITING REVIEW",
        Wave::Scheduled => "SCHEDULED",
        Wave::Neutral => match card.status {
            SessionStatusValue::Idle => "IDLE",
            SessionStatusValue::Launching => "LAUNCHING",
            SessionStatusValue::Running => "RUNNING",
            SessionStatusValue::Waiting => "WAITING",
            SessionStatusValue::Failed => "FAILED",
            SessionStatusValue::Unknown => "UNKNOWN",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_icon_path_maps_every_glyph_wave() {
        assert_eq!(wave_icon_path(Wave::NeedsInput), Some("icons/bell.svg"));
        assert_eq!(
            wave_icon_path(Wave::Failed),
            Some("icons/triangle-alert.svg")
        );
        assert_eq!(
            wave_icon_path(Wave::AwaitingReview),
            Some("icons/circle-dot.svg")
        );
        assert_eq!(
            wave_icon_path(Wave::Scheduled),
            Some("icons/alarm-clock.svg")
        );
        assert_eq!(wave_icon_path(Wave::Ready), Some("icons/check.svg"));
        assert_eq!(wave_icon_path(Wave::Slept), Some("icons/moon.svg"));
        assert_eq!(wave_icon_path(Wave::Neutral), Some("icons/coffee.svg"));
    }

    #[test]
    fn working_has_no_static_glyph() {
        assert_eq!(wave_icon_path(Wave::Working), None);
    }

    #[test]
    fn every_glyph_path_is_a_bundled_asset() {
        for wave in [
            Wave::NeedsInput,
            Wave::Failed,
            Wave::AwaitingReview,
            Wave::Scheduled,
            Wave::Ready,
            Wave::Slept,
            Wave::Neutral,
        ] {
            let path = wave_icon_path(wave).unwrap();
            assert!(
                crate::assets::ICON_PATHS.contains(&path),
                "{wave:?} → {path} not in ICON_PATHS"
            );
        }
    }
}
