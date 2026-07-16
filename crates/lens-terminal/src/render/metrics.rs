//! Cell metrics + the fail-closed Menlo hardware gate.
//!
//! `resolve_menlo` is on the real paint path; the gate + alignment probes
//! (`menlo_gate_ok`, `per_row_alignment_ok`, box-drawing probe) are asserted
//! only inside the real-window harness, so they are `test-util`-gated to avoid
//! dead-code in the normal library build.

use gpui::{Font, Pixels, Window, font, px};

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

/// Fail-closed Menlo gate (T2): resolves-to-Menlo, uniform advance, and the
/// per-row + box-drawing alignment probes. Stub until Task 2.
#[cfg(any(test, feature = "test-util"))]
pub fn menlo_gate_ok(_window: &Window, _metrics: &CellMetrics) -> MenloGateResult {
    MenloGateResult {
        ok: false,
        reason: "not implemented",
    }
}

/// Probe that per-row `shape_line` keeps wide/emoji on the monospace grid
/// (checks the cell after the emoji too). Stub until Task 2.
#[cfg(any(test, feature = "test-util"))]
pub fn per_row_alignment_ok(_window: &Window, _metrics: &CellMetrics) -> bool {
    false
}
