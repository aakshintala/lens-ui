//! Cell metrics + the fail-closed Menlo hardware gate.
//!
//! `resolve_menlo` is on the real paint path; the gate + alignment probes
//! (`menlo_gate_ok`, `per_row_alignment_ok`, box-drawing probe) are asserted
//! only inside the real-window harness, so they are `test-util`-gated to avoid
//! dead-code in the normal library build.

use gpui::{Font, Pixels, Window, font, px};
#[cfg(any(test, feature = "test-util"))]
use gpui::{Hsla, Rgba, SharedString, TextRun};

/// Menlo cell metrics resolved once from the window text system.
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub cell_w: Pixels,
    pub cell_h: Pixels,
    pub font_size: Pixels,
    pub font: Font,
    pub bold_font: Font,
    pub italic_font: Font,
    pub bold_italic_font: Font,
}

impl CellMetrics {
    /// Resolve Menlo + its bold/italic variants and the monospace cell advance.
    ///
    /// **Real text system only** — under gpui's `NoopTextSystem` (`#[gpui::test]`)
    /// `ch_advance` fabricates a value. Call from a real-window canvas paint.
    pub fn resolve_menlo(window: &Window) -> Self {
        let font_size = px(14.0);
        let base = font("Menlo");
        let bold = base.clone().bold();
        let italic = base.clone().italic();
        let bold_italic = base.clone().bold().italic();
        let font_id = window.text_system().resolve_font(&base);
        let cell_w = window
            .text_system()
            .ch_advance(font_id, font_size)
            .unwrap_or(px(8.4));
        let cell_h = window.line_height();
        Self {
            cell_w,
            cell_h,
            font_size,
            font: base,
            bold_font: bold,
            italic_font: italic,
            bold_italic_font: bold_italic,
        }
    }
}

/// Outcome of the fail-closed Menlo gate. `reason` is a stable static string
/// surfaced to stderr on failure.
#[cfg(any(test, feature = "test-util"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenloGateResult {
    pub ok: bool,
    pub reason: &'static str,
}

#[cfg(any(test, feature = "test-util"))]
fn rgb_to_hsla(c: crate::Rgb) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    })
}

/// Probe that per-row `shape_line` keeps text on the monospace grid **and
/// re-synchronises to it after a wide glyph**.
///
/// Shapes `"a日b😀c"` and asserts the CJK `日` (col 1) and the cell *after* the
/// emoji (`c`, col 6) land within 0.75px of their expected `col * cell_w`.
///
/// It deliberately does **not** assert the emoji's own left edge (col 4): a
/// wide glyph's advance under per-row shaping drifts (the CJK fallback glyph is
/// slightly under 2 cells), and the render path never places a wide/emoji cell
/// via per-row — any row containing a wide cell routes to per-cell placement
/// (`row_needs_per_cell`). The meaningful hardware invariant is that the line
/// *returns to grid* after the wide run, which `c` at col 6 proves. That
/// wide-rows-→-per-cell routing is guarded separately in the paint path.
#[cfg(any(test, feature = "test-util"))]
pub fn per_row_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool {
    let sample = "a日b😀c";
    let run = TextRun {
        len: sample.len(),
        font: metrics.font.clone(),
        color: rgb_to_hsla(crate::Rgb {
            r: 255,
            g: 255,
            b: 255,
        }),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(sample),
        metrics.font_size,
        &[run],
        None,
    );
    let tol = px(0.75);
    let x_cjk = shaped.x_for_index(sample.find('日').unwrap());
    let x_after = shaped.x_for_index(sample.find('c').unwrap());
    (x_cjk - metrics.cell_w).abs() <= tol && (x_after - metrics.cell_w * 6.0).abs() <= tol
}

/// Probe that box-drawing glyphs stay on the grid (`"┌─┐"` at cols 0,1,2).
#[cfg(any(test, feature = "test-util"))]
fn box_drawing_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool {
    let sample = "┌─┐";
    let run = TextRun {
        len: sample.len(),
        font: metrics.font.clone(),
        color: rgb_to_hsla(crate::Rgb {
            r: 255,
            g: 255,
            b: 255,
        }),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(sample),
        metrics.font_size,
        &[run],
        None,
    );
    let tol = px(0.75);
    let mut ok = true;
    let mut byte = 0usize;
    for (col, ch) in sample.chars().enumerate() {
        let x = shaped.x_for_index(byte);
        ok &= (x - metrics.cell_w * col as f32).abs() <= tol;
        byte += ch.len_utf8();
    }
    ok
}

/// Fail-closed Menlo gate: family resolves to real Menlo, `'0'`/`'i'` advances
/// agree (monospace), and both alignment probes pass. `ok == false` on the dev
/// machine means STOP and reopen `lens-fonts` (do not soft-pass).
#[cfg(any(test, feature = "test-util"))]
pub fn menlo_gate_ok(window: &Window, metrics: &CellMetrics) -> MenloGateResult {
    let font_id = window.text_system().resolve_font(&metrics.font);
    let Some(resolved) = window.text_system().get_font_for_id(font_id) else {
        return MenloGateResult {
            ok: false,
            reason: "get_font_for_id returned None",
        };
    };
    if resolved.family.as_ref() != "Menlo" {
        return MenloGateResult {
            ok: false,
            reason: "resolved font family is not Menlo (fallback?)",
        };
    }
    let adv0 = window
        .text_system()
        .ch_advance(font_id, metrics.font_size)
        .unwrap_or(px(0.0));
    let adv_i = window
        .text_system()
        .advance(font_id, metrics.font_size, 'i')
        .map(|s| s.width)
        .unwrap_or(px(0.0));
    if (adv0 - adv_i).abs() > px(0.5) {
        return MenloGateResult {
            ok: false,
            reason: "Menlo advances for '0' and 'i' diverge",
        };
    }
    if !per_row_alignment_ok(window, metrics) {
        return MenloGateResult {
            ok: false,
            reason: "post-emoji / CJK per-row alignment probe failed",
        };
    }
    if !box_drawing_alignment_ok(window, metrics) {
        return MenloGateResult {
            ok: false,
            reason: "box-drawing alignment probe failed",
        };
    }
    MenloGateResult {
        ok: true,
        reason: "ok",
    }
}
