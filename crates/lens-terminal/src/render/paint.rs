//! Full-snapshot `Frame` painter (Slice 1c).
//!
//! Lifts the spike's per-row glyph placement onto Lens-owned `Frame` values
//! (no `RowShapeCache`, no `libghostty_vt` types). `paint_frame` stays `pub`
//! inside the private `render` module — never re-exported at the crate root —
//! so it does not enter the public API (I12).
//!
//! T3: default-bg fill + per-cell background quads + per-row shaping for every
//! row (bold-aware only; full SGR is T5). T4 adds per-cell routing for wide
//! rows; T5 adds the full SGR resolver + decoration quads.

use std::time::Instant;

use gpui::{App, Bounds, Hsla, Pixels, Point, Rgba, ShapedLine, SharedString, TextRun, Window};
use gpui::{fill, point, size};

use super::metrics::CellMetrics;
use crate::{Frame, FrameCell, FrameRow, Rgb};

/// Selection background (matches the spike's highlight).
const SELECTION_BG: Rgb = Rgb {
    r: 40,
    g: 60,
    b: 120,
};

/// Per-paint statistics surfaced into Inspect + the perf gate. Errors from
/// `ShapedLine::paint` are counted here, never `unwrap`ed on the paint path.
#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub rows_painted: u32,
    pub cells_bg: u32,
    pub shapes: u32,
    pub per_row_rows: u32,
    pub per_cell_rows: u32,
    pub paint_errors: u32,
    pub paint_micros: u64,
}

pub(super) fn rgb_to_rgba(c: Rgb) -> Rgba {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    }
}

pub(super) fn rgb_to_hsla(c: Rgb) -> Hsla {
    Hsla::from(rgb_to_rgba(c))
}

/// Effective background for a cell: the selection colour when selected, else
/// the cell's own background (`None` = default bg, no quad).
pub(super) fn effective_bg(cell: &FrameCell) -> Option<Rgb> {
    if cell.selected {
        Some(SELECTION_BG)
    } else {
        cell.bg
    }
}

/// Paint per-cell background quads (wide cells span two columns). Returns the
/// number of quads painted.
fn paint_backgrounds(
    frame: &Frame,
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
) -> u32 {
    let mut n = 0u32;
    for (row_i, row) in frame.grid.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        for cell in &row.cells {
            let Some(bg) = effective_bg(cell) else {
                continue;
            };
            let x = origin.x + metrics.cell_w * f32::from(cell.col);
            let width = if cell.wide {
                metrics.cell_w * 2.0
            } else {
                metrics.cell_w
            };
            let rect = Bounds::new(point(x, y), size(width, metrics.cell_h));
            window.paint_quad(fill(rect, rgb_to_rgba(bg)));
            n += 1;
        }
    }
    n
}

/// Shape a whole row as one line (T3: bold-aware; full SGR lands in T5).
/// Gap-fills missing columns with spaces so glyphs stay on the grid.
pub(super) fn shape_row_line(
    row: &FrameRow,
    metrics: &CellMetrics,
    window: &Window,
) -> Option<ShapedLine> {
    if row.cells.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut expected_col = 0u16;

    for cell in &row.cells {
        while expected_col < cell.col {
            text.push(' ');
            expected_col += 1;
        }
        let start_len = text.len();
        text.push_str(&cell.grapheme);
        let byte_len = text.len() - start_len;
        let run_font = if cell.style.bold {
            metrics.bold_font.clone()
        } else {
            metrics.font.clone()
        };
        let color = rgb_to_hsla(cell.fg);
        if let Some(last) = runs.last_mut()
            && last.font == run_font
            && last.color == color
            && last.underline.is_none()
            && last.strikethrough.is_none()
            && last.background_color.is_none()
        {
            last.len += byte_len;
        } else {
            runs.push(TextRun {
                len: byte_len,
                font: run_font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        expected_col = cell.col.saturating_add(if cell.wide { 2 } else { 1 });
    }

    if text.is_empty() || runs.is_empty() {
        return None;
    }
    Some(
        window
            .text_system()
            .shape_line(SharedString::from(text), metrics.font_size, &runs, None),
    )
}

/// Shape + paint one row per-row. Returns `(shapes, paint_errors)`.
fn paint_per_row_row(
    row: &FrameRow,
    y: Pixels,
    origin_x: Pixels,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u32, u32) {
    let Some(shaped) = shape_row_line(row, metrics, window) else {
        return (0, 0);
    };
    let errors = u32::from(
        shaped
            .paint(point(origin_x, y), metrics.cell_h, window, cx)
            .is_err(),
    );
    (1, errors)
}

/// Paint a full `Frame` snapshot at `origin`. Emits the default-bg fill, per
/// cell background quads, then per-row glyphs. Never `unwrap`s the paint path:
/// `ShapedLine::paint` errors are counted into `RenderStats::paint_errors`.
pub fn paint_frame(
    frame: &Frame,
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> RenderStats {
    let t0 = Instant::now();

    // Default background over the whole grid area.
    let grid_bounds = Bounds::new(
        origin,
        size(
            metrics.cell_w * f32::from(frame.cols),
            metrics.cell_h * f32::from(frame.rows),
        ),
    );
    window.paint_quad(fill(grid_bounds, rgb_to_rgba(frame.default_bg)));

    let cells_bg = paint_backgrounds(frame, origin, metrics, window);

    let mut shapes = 0u32;
    let mut per_row_rows = 0u32;
    let per_cell_rows = 0u32; // T4 populates this.
    let mut paint_errors = 0u32;

    for (row_i, row) in frame.grid.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        per_row_rows += 1;
        let (s, e) = paint_per_row_row(row, y, origin.x, metrics, window, cx);
        shapes += s;
        paint_errors += e;
    }

    RenderStats {
        rows_painted: frame.grid.len() as u32,
        cells_bg,
        shapes,
        per_row_rows,
        per_cell_rows,
        paint_errors,
        paint_micros: t0.elapsed().as_micros() as u64,
    }
}
