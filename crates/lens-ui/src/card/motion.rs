use std::f32::consts::TAU;
use std::time::Duration;

use gpui::{
    Bounds, ContentMask, Hsla, IntoElement, ParentElement, PathBuilder, Styled, Transformation,
    canvas, div, linear_color_stop, linear_gradient, point, px, radians, svg,
};

use super::model::SessionCard;
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

const SPIN_PERIOD_MS: i64 = 2000;

/// Spinner rotation fraction (0..1) at `now_ms` — pure fn of the clock (period 2.0s).
/// i64 modulo BEFORE the f32 cast (epoch-millis overflow — see `sweep_phase`).
pub fn spin_phase(now_ms: i64) -> f32 {
    now_ms.rem_euclid(SPIN_PERIOD_MS) as f32 / SPIN_PERIOD_MS as f32
}

/// True for any wave that runs a per-frame (or per-second) animation.
pub fn wave_animates(wave: Wave) -> bool {
    sweep_period(wave).is_some() || matches!(wave, Wave::Working | Wave::Scheduled)
}

/// Frame cap per wave: 30fps for the sweep/spinner class, 1 Hz for the Scheduled
/// countdown, `None` for still waves. (`demo` feature can override the 30fps value — Task 10.)
pub fn anim_tick_for(wave: Wave) -> Option<Duration> {
    if matches!(wave, Wave::Scheduled) {
        return Some(Duration::from_millis(1000));
    }
    if wave_animates(wave) {
        return Some(Duration::from_millis(anim_tick_ms_fast()));
    }
    None
}

/// The fast-class frame cap in ms. Overridable ONLY under the `demo` feature (Task 10);
/// the shipped build is a hard 33ms (≈30fps).
fn anim_tick_ms_fast() -> u64 {
    #[cfg(feature = "demo")]
    {
        if let Some(ms) = std::env::var("LENS_ANIM_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n >= 1)
        {
            return ms;
        }
    }
    33
}

const RING_PERIOD_MS: i64 = 2400;
/// Expanding-ring phase (0..1) for the loud pair. i64 modulo first (see `sweep_phase`).
pub fn ring_phase(now_ms: i64) -> f32 {
    now_ms.rem_euclid(RING_PERIOD_MS) as f32 / RING_PERIOD_MS as f32
}

// Peak alpha of the moving highlight = status × (24% × amplitude 0.4) — mockup value; soft
// so text stays legible. Tunable in the end-of-build pass.
const SWEEP_PEAK_ALPHA: f32 = 0.096;
// Skew of the band (degrees), matching the mockup's skewX(-14deg).
const SWEEP_SKEW_DEG: f32 = 14.0;
// Band width as a fraction of card width.
const SWEEP_BAND_FRAC: f32 = 0.48;

/// Clipped sweep overlay at `phase` (0..1), drawn as a skewed gradient parallelogram via
/// `canvas` + `paint_path`. No `with_animation` — the card view's timer re-renders us with a
/// fresh clock-derived phase. Two half-bands fake a symmetric feather (gpui gradients are 2-stop).
pub fn render_sweep_overlay(status: Hsla, phase: f32) -> impl IntoElement {
    let peak = status.opacity(SWEEP_PEAK_ALPHA);
    let edge = status.opacity(0.0);

    canvas(
        move |_, _, _| (),
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let h = f32::from(bounds.size.height);
                let card_w = f32::from(bounds.size.width);
                let band_w = card_w * SWEEP_BAND_FRAC;
                let skew = h * SWEEP_SKEW_DEG.to_radians().tan();

                // Center-x of the band as it travels from fully off-left to fully off-right.
                // The parallelogram's true horizontal extent includes the top shear, so the
                // travel endpoints must account for `skew` — otherwise the band pops in as a
                // visible wedge at the phase wrap (1→0) instead of entering off-screen.
                let cx_start = -band_w * 0.5 - skew.max(0.0);
                let cx_end = card_w + band_w * 0.5 - skew.min(0.0);
                let cx = cx_start + phase * (cx_end - cx_start);
                let x = |dx: f32| bounds.origin.x + px(dx);
                let top = bounds.origin.y;
                let bot = bounds.origin.y + px(h);

                // A parallelogram spanning [left..right] at the bottom, sheared right by `skew`
                // at the top. `half` builds one gradient half.
                let mut half = |x0: f32, x1: f32, from: Hsla, to: Hsla| {
                    let mut b = PathBuilder::fill();
                    b.move_to(point(x(x0), bot));
                    b.line_to(point(x(x0 + skew), top));
                    b.line_to(point(x(x1 + skew), top));
                    b.line_to(point(x(x1), bot));
                    b.close();
                    if let Ok(path) = b.build() {
                        window.paint_path(
                            path,
                            linear_gradient(
                                90.0,
                                linear_color_stop(from, 0.0),
                                linear_color_stop(to, 1.0),
                            ),
                        );
                    }
                };

                let left = cx - band_w * 0.5;
                let mid = cx;
                let right = cx + band_w * 0.5;
                half(left, mid, edge, peak);
                half(mid, right, peak, edge);
            });
        },
    )
    .absolute()
    .size_full()
}

/// Rotating Lucide `loader-circle`, tinted to the working color, angle from the clock.
/// Rotation is render-only (no layout/hitbox effect) — pivots around the element center.
pub fn render_working_spinner(status: Hsla, now_ms: i64) -> impl IntoElement {
    svg()
        .path("icons/loader-circle.svg")
        .w(px(22.0))
        .h(px(22.0))
        .text_color(status)
        .with_transformation(Transformation::rotate(radians(spin_phase(now_ms) * TAU)))
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

/// Remaining fraction (0..1) of a scheduled wake window; `None` if either bound is missing.
pub fn countdown_fraction(
    started_at: Option<i64>,
    wake_at: Option<i64>,
    now_ms: i64,
) -> Option<f32> {
    let (start, wake) = (started_at?, wake_at?);
    let span = wake.saturating_sub(start);
    if span <= 0 {
        return Some(0.0);
    }
    let remaining = wake.saturating_sub(now_ms).max(0) as f32 / span as f32;
    Some(remaining.clamp(0.0, 1.0))
}

/// Live countdown label. `remaining_ms <= 0` → "waking…".
pub fn format_wake_countdown(remaining_ms: i64) -> String {
    if remaining_ms <= 0 {
        return "waking…".into();
    }
    let secs = remaining_ms / 1000 + (remaining_ms % 1000 != 0) as i64;
    if secs >= 60 {
        format!("wakes in {}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("wakes in {secs}s")
    }
}

/// Depleting arc around the 44px tile, drawn full→empty as `fraction` goes 1→0.
/// `canvas` + `arc_to` stroke (gpui has no conic-gradient). NOT in the 30fps loop — 1 Hz.
pub fn render_countdown_ring(status: Hsla, fraction: f32) -> impl IntoElement {
    let frac = fraction.clamp(0.0, 1.0);
    canvas(
        move |_, _, _| (),
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            if frac <= 0.0 {
                return;
            }
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let stroke = px(2.0);
                let center = bounds.center();
                let radius = (bounds.size.width.min(bounds.size.height) - stroke) / 2.0;
                let sweep_deg = frac * 360.0;
                // 0° = top, clockwise.
                let polar = |deg: f32| {
                    let r = deg.to_radians();
                    point(center.x + radius * r.sin(), center.y - radius * r.cos())
                };
                let mut b = PathBuilder::stroke(stroke);
                b.move_to(polar(0.0));
                if sweep_deg >= 359.9 {
                    // full ring = two half-arcs (single arc_to can't close 360°).
                    b.arc_to(point(radius, radius), px(0.0), true, true, polar(180.0));
                    b.arc_to(point(radius, radius), px(0.0), true, true, polar(0.0));
                } else {
                    b.arc_to(
                        point(radius, radius),
                        px(0.0),
                        sweep_deg > 180.0,
                        true,
                        polar(sweep_deg),
                    );
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, status);
                }
            });
        },
    )
    .absolute()
    .inset(px(-4.0))
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
    fn spin_phase_advances_and_wraps() {
        assert_eq!(spin_phase(0), 0.0);
        assert!((spin_phase(1000) - 0.5).abs() < 1e-4, "half period");
        assert!(spin_phase(2000).abs() < 1e-4, "wraps at 2s");
    }

    #[test]
    fn spin_phase_survives_epoch_millis() {
        // i64-modulo-before-cast: a real epoch value must not quantize to a frozen phase.
        let a = spin_phase(1_700_000_000_123);
        let b = spin_phase(1_700_000_000_123 + 500);
        assert!((a - b).abs() > 1e-3, "phase must advance at epoch scale");
    }

    #[test]
    fn sweep_phase_is_a_clock_ratio() {
        // 1.0s period for the loud pair; midpoint at 500ms.
        assert!((sweep_phase(Wave::NeedsInput, 500).unwrap() - 0.5).abs() < 1e-4);
        assert_eq!(sweep_phase(Wave::NeedsInput, 0), Some(0.0));
        // 1.5s for the soft pair.
        assert!((sweep_phase(Wave::Ready, 750).unwrap() - 0.5).abs() < 1e-4);
        // non-sweep waves → None.
        assert_eq!(sweep_phase(Wave::Working, 123), None);
        assert_eq!(sweep_phase(Wave::Slept, 123), None);
    }

    #[test]
    fn sweep_phase_survives_epoch_millis() {
        let a = sweep_phase(Wave::NeedsInput, 1_700_000_000_123).unwrap();
        let b = sweep_phase(Wave::NeedsInput, 1_700_000_000_123 + 100).unwrap();
        assert!((a - b).abs() > 1e-3, "phase must advance at epoch scale");
    }

    #[test]
    fn working_animates_now() {
        assert!(wave_animates(Wave::Working));
        assert!(!wave_animates(Wave::Slept));
        assert!(!wave_animates(Wave::Neutral));
    }

    #[test]
    fn anim_tick_cadence_per_wave() {
        use std::time::Duration;
        assert_eq!(
            anim_tick_for(Wave::NeedsInput),
            Some(Duration::from_millis(33))
        );
        assert_eq!(
            anim_tick_for(Wave::Working),
            Some(Duration::from_millis(33))
        );
        assert_eq!(
            anim_tick_for(Wave::Scheduled),
            Some(Duration::from_millis(1000))
        );
        assert_eq!(anim_tick_for(Wave::Slept), None);
        assert_eq!(anim_tick_for(Wave::Neutral), None);
    }

    #[test]
    fn countdown_fraction_depletes() {
        let start = 10_000;
        let wake = 10_000 + 180_000; // 3m window
        assert_eq!(
            countdown_fraction(Some(start), Some(wake), start),
            Some(1.0)
        );
        assert_eq!(countdown_fraction(Some(start), Some(wake), wake), Some(0.0));
        let mid = countdown_fraction(Some(start), Some(wake), start + 90_000).unwrap();
        assert!((mid - 0.5).abs() < 1e-3);
        // past wake clamps to 0; missing bound → None.
        assert_eq!(
            countdown_fraction(Some(start), Some(wake), wake + 5_000),
            Some(0.0)
        );
        assert_eq!(countdown_fraction(None, Some(wake), start), None);
    }

    #[test]
    fn format_wake_countdown_shapes() {
        assert_eq!(format_wake_countdown(179_000), "wakes in 2m 59s");
        assert_eq!(format_wake_countdown(45_000), "wakes in 45s");
        assert_eq!(format_wake_countdown(0), "waking…");
        assert_eq!(format_wake_countdown(-1), "waking…");
    }

    #[test]
    fn countdown_fraction_extreme_bounds_no_panic() {
        // Non-positive span from adversarial bounds must clamp to 0, not overflow.
        assert_eq!(
            countdown_fraction(Some(i64::MAX), Some(i64::MIN), 0),
            Some(0.0)
        );
        // Huge valid span does not panic.
        assert_eq!(
            countdown_fraction(Some(i64::MIN), Some(i64::MAX), i64::MAX),
            Some(0.0)
        );
    }

    #[test]
    fn format_wake_countdown_no_overflow_at_i64_max() {
        // Must not panic (debug) / wrap (release) at extreme remaining.
        let s = format_wake_countdown(i64::MAX);
        assert!(s.starts_with("wakes in "), "got {s}");
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
