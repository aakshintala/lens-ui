//! Full-snapshot `Frame` painter (Slice 1c).
//!
//! Lifts the spike's per-row/per-cell glyph placement onto Lens-owned `Frame`
//! values (no `RowShapeCache`, no `libghostty_vt` types). `paint_frame` stays
//! `pub` inside the private `render` module — never re-exported at the crate
//! root — so it does not enter the public API (I12).
//!
//! Layers: default-bg fill → per-cell background quads → per-row (or per-cell
//! for wide rows) glyphs → per-cell decoration quads (overline + double/dotted/
//! dashed underline). SGR is resolved per cell (`resolve_cell_paint`): inverse,
//! faint, bold/italic font, single/curly underline (on the `TextRun`), and
//! double/dotted/dashed underline + overline (as quads coloured with
//! `underline_quad_color`, I10a). Invisible cells keep their advance via
//! width-preserving spaces (I10b). `blink` is a steady no-op in 1c.

use std::time::Instant;

use gpui::{
    App, Bounds, Font, Hsla, Pixels, Point, Rgba, ShapedLine, SharedString, TextRun, Window,
};
use gpui::{fill, point, px, size};

use super::metrics::CellMetrics;
use crate::{Frame, FrameCell, FrameRow, Rgb};

/// Selection background (matches the spike's highlight).
const SELECTION_BG: Rgb = Rgb {
    r: 40,
    g: 60,
    b: 120,
};

const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
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

/// Underline styles that render as decoration **quads** (gpui's `TextRun`
/// underline only models single/wavy). Double/dotted/dashed + overline are
/// painted separately, coloured with `ResolvedCellPaint::underline_quad_color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderlineQuadKind {
    None,
    Double,
    Dotted,
    Dashed,
}

/// The resolved paint for one cell after applying SGR (inverse/faint/bold/
/// italic/underline/strikethrough/invisible).
pub struct ResolvedCellPaint {
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub font: Font,
    /// gpui `TextRun` underline (single → straight, curly → wavy). `None` when
    /// the underline is a quad kind or absent.
    pub underline: Option<gpui::UnderlineStyle>,
    pub strikethrough: Option<gpui::StrikethroughStyle>,
    /// Invisible cell: keep the advance, emit no glyph (I10b).
    pub skip_glyph: bool,
    pub overline: bool,
    pub underline_quad_kind: UnderlineQuadKind,
    /// Colour for every decoration quad (overline + double/dotted/dashed) —
    /// `underline_color` if set, else the post-inverse/faint fg (I10a).
    pub underline_quad_color: Rgb,
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

/// Resolve the full SGR set for one cell. `blink` is a steady no-op in 1c.
pub(super) fn resolve_cell_paint(
    cell: &FrameCell,
    default_bg: Rgb,
    metrics: &CellMetrics,
) -> ResolvedCellPaint {
    let _blink_steady = cell.style.blink;

    let mut fg = cell.fg;
    let mut bg = effective_bg(cell);
    if cell.style.inverse {
        let bg_for_swap = bg.unwrap_or(default_bg);
        bg = Some(fg);
        fg = bg_for_swap;
    }
    if cell.style.faint {
        fg = Rgb {
            r: fg.r / 2,
            g: fg.g / 2,
            b: fg.b / 2,
        };
    }

    let font = match (cell.style.bold, cell.style.italic) {
        (true, true) => metrics.bold_italic_font.clone(),
        (true, false) => metrics.bold_font.clone(),
        (false, true) => metrics.italic_font.clone(),
        (false, false) => metrics.font.clone(),
    };

    let underline_quad_color = cell.style.underline_color.unwrap_or(fg);
    let ul_hsla = Some(rgb_to_hsla(underline_quad_color));

    let (underline, underline_quad_kind) = match cell.style.underline {
        crate::UnderlineStyle::None => (None, UnderlineQuadKind::None),
        crate::UnderlineStyle::Single => (
            Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: ul_hsla,
                wavy: false,
            }),
            UnderlineQuadKind::None,
        ),
        crate::UnderlineStyle::Curly => (
            Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: ul_hsla,
                wavy: true,
            }),
            UnderlineQuadKind::None,
        ),
        crate::UnderlineStyle::Double => (None, UnderlineQuadKind::Double),
        crate::UnderlineStyle::Dotted => (None, UnderlineQuadKind::Dotted),
        crate::UnderlineStyle::Dashed => (None, UnderlineQuadKind::Dashed),
    };

    let strikethrough = if cell.style.strikethrough {
        Some(gpui::StrikethroughStyle {
            thickness: px(1.0),
            color: Some(rgb_to_hsla(fg)),
        })
    } else {
        None
    };

    ResolvedCellPaint {
        fg,
        bg,
        font,
        underline,
        strikethrough,
        skip_glyph: cell.style.invisible,
        overline: cell.style.overline,
        underline_quad_kind,
        underline_quad_color,
    }
}

/// Paint per-cell background quads (wide cells span two columns). Uses
/// `resolve_cell_paint` so inverse's swapped background shows. Returns the
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
            let Some(bg) = resolve_cell_paint(cell, frame.default_bg, metrics).bg else {
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

/// Whether a row must use per-cell placement. **The single routing SSOT**: a
/// row with any wide cell is placed per-cell (per-row shaping drifts wide/emoji
/// advances off the grid — see `metrics::per_row_alignment_ok`).
pub fn row_needs_per_cell(row: &FrameRow) -> bool {
    row.cells.iter().any(|c| c.wide)
}

/// Push `s` onto `text` + a matching `TextRun`, merging with the previous run
/// when the style matches (only plain, decoration-free runs merge — keeps the
/// merge cheap without requiring `UnderlineStyle: PartialEq`).
fn push_run(
    text: &mut String,
    runs: &mut Vec<TextRun>,
    s: &str,
    font: &Font,
    color: Hsla,
    underline: Option<gpui::UnderlineStyle>,
    strikethrough: Option<gpui::StrikethroughStyle>,
) {
    let start = text.len();
    text.push_str(s);
    let byte_len = text.len() - start;
    let plain = underline.is_none() && strikethrough.is_none();
    if plain
        && let Some(last) = runs.last_mut()
        && last.font == *font
        && last.color == color
        && last.underline.is_none()
        && last.strikethrough.is_none()
        && last.background_color.is_none()
    {
        last.len += byte_len;
    } else {
        runs.push(TextRun {
            len: byte_len,
            font: font.clone(),
            color,
            background_color: None,
            underline,
            strikethrough,
        });
    }
}

/// Assemble a row's `(text, runs)` for per-row shaping. Gap columns and
/// invisible cells become width-preserving spaces so glyphs stay on the grid
/// (I10b). No `Window` needed — shared with the invisible-width test.
fn assemble_row(
    row: &FrameRow,
    default_bg: Rgb,
    metrics: &CellMetrics,
) -> Option<(String, Vec<TextRun>)> {
    if row.cells.is_empty() {
        return None;
    }
    let white = rgb_to_hsla(WHITE);
    let mut text = String::new();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut expected_col = 0u16;

    for cell in &row.cells {
        while expected_col < cell.col {
            push_run(&mut text, &mut runs, " ", &metrics.font, white, None, None);
            expected_col += 1;
        }
        let resolved = resolve_cell_paint(cell, default_bg, metrics);
        let width_cells = if cell.wide { 2u16 } else { 1u16 };
        if resolved.skip_glyph {
            for _ in 0..width_cells {
                push_run(&mut text, &mut runs, " ", &metrics.font, white, None, None);
            }
        } else {
            push_run(
                &mut text,
                &mut runs,
                &cell.grapheme,
                &resolved.font,
                rgb_to_hsla(resolved.fg),
                resolved.underline,
                resolved.strikethrough,
            );
        }
        expected_col = cell.col.saturating_add(width_cells);
    }

    if text.is_empty() || runs.is_empty() {
        None
    } else {
        Some((text, runs))
    }
}

/// Shape a whole row as one line. Never receives a wide cell — those route to
/// `paint_per_cell_row` (`row_needs_per_cell`); a misroute would silently place
/// a wide glyph off-grid, so we `debug_assert` it (debug/test only — the paint
/// path must never panic in release).
pub(super) fn shape_row_line(
    row: &FrameRow,
    default_bg: Rgb,
    metrics: &CellMetrics,
    window: &Window,
) -> Option<ShapedLine> {
    debug_assert!(
        !row.cells.iter().any(|c| c.wide),
        "per-row shaping received a wide cell; wide rows must route to per-cell (row_needs_per_cell)"
    );
    let (text, runs) = assemble_row(row, default_bg, metrics)?;
    Some(
        window
            .text_system()
            .shape_line(SharedString::from(text), metrics.font_size, &runs, None),
    )
}

/// Paint overline + double/dotted/dashed underline quads for one cell, coloured
/// with `underline_quad_color` (I10a).
fn paint_decoration_quads(
    cell: &FrameCell,
    resolved: &ResolvedCellPaint,
    cell_origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
) {
    let width = if cell.wide {
        metrics.cell_w * 2.0
    } else {
        metrics.cell_w
    };
    let color = rgb_to_rgba(resolved.underline_quad_color);
    if resolved.overline {
        window.paint_quad(fill(Bounds::new(cell_origin, size(width, px(1.0))), color));
    }
    let ul_y = cell_origin.y + metrics.cell_h - px(2.0);
    match resolved.underline_quad_kind {
        UnderlineQuadKind::None => {}
        UnderlineQuadKind::Double => {
            window.paint_quad(fill(
                Bounds::new(point(cell_origin.x, ul_y), size(width, px(1.0))),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(cell_origin.x, ul_y - px(2.0)), size(width, px(1.0))),
                color,
            ));
        }
        UnderlineQuadKind::Dotted => {
            let mut x = cell_origin.x;
            while x < cell_origin.x + width {
                let seg = px(2.0).min(cell_origin.x + width - x);
                window.paint_quad(fill(Bounds::new(point(x, ul_y), size(seg, px(1.0))), color));
                x += px(4.0);
            }
        }
        UnderlineQuadKind::Dashed => {
            let mut x = cell_origin.x;
            while x < cell_origin.x + width {
                let seg = px(4.0).min(cell_origin.x + width - x);
                window.paint_quad(fill(Bounds::new(point(x, ul_y), size(seg, px(1.0))), color));
                x += px(8.0);
            }
        }
    }
}

/// Paint the decoration quads (overline / quad underlines) for every cell in a
/// row. Shared by both placement paths.
fn paint_row_decorations(
    row: &FrameRow,
    y: Pixels,
    origin_x: Pixels,
    default_bg: Rgb,
    metrics: &CellMetrics,
    window: &mut Window,
) {
    for cell in &row.cells {
        let resolved = resolve_cell_paint(cell, default_bg, metrics);
        if resolved.overline || resolved.underline_quad_kind != UnderlineQuadKind::None {
            let x = origin_x + metrics.cell_w * f32::from(cell.col);
            paint_decoration_quads(cell, &resolved, point(x, y), metrics, window);
        }
    }
}

/// Shape + paint one row per-row, then its decoration quads. Returns
/// `(shapes, paint_errors)`.
fn paint_per_row_row(
    row: &FrameRow,
    y: Pixels,
    origin_x: Pixels,
    default_bg: Rgb,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u32, u32) {
    let mut errors = 0u32;
    let mut shapes = 0u32;
    if let Some(shaped) = shape_row_line(row, default_bg, metrics, window) {
        shapes = 1;
        errors += u32::from(
            shaped
                .paint(point(origin_x, y), metrics.cell_h, window, cx)
                .is_err(),
        );
    }
    paint_row_decorations(row, y, origin_x, default_bg, metrics, window);
    (shapes, errors)
}

/// Shape + paint each cell at its exact `col * cell_w`, then decoration quads.
/// Used for rows containing wide/emoji cells. Skips blank + invisible cells'
/// glyphs (I10b). Returns `(shapes, paint_errors)`.
fn paint_per_cell_row(
    row: &FrameRow,
    y: Pixels,
    origin_x: Pixels,
    default_bg: Rgb,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u32, u32) {
    let mut shapes = 0u32;
    let mut errors = 0u32;
    for cell in &row.cells {
        let resolved = resolve_cell_paint(cell, default_bg, metrics);
        let x = origin_x + metrics.cell_w * f32::from(cell.col);
        if !resolved.skip_glyph && cell.grapheme != " " {
            let text = SharedString::from(cell.grapheme.clone());
            let run = TextRun {
                len: text.len(),
                font: resolved.font.clone(),
                color: rgb_to_hsla(resolved.fg),
                background_color: None,
                underline: resolved.underline,
                strikethrough: resolved.strikethrough,
            };
            let shaped = window
                .text_system()
                .shape_line(text, metrics.font_size, &[run], None);
            shapes += 1;
            errors += u32::from(
                shaped
                    .paint(point(x, y), metrics.cell_h, window, cx)
                    .is_err(),
            );
        }
        if resolved.overline || resolved.underline_quad_kind != UnderlineQuadKind::None {
            paint_decoration_quads(cell, &resolved, point(x, y), metrics, window);
        }
    }
    (shapes, errors)
}

/// Paint a full `Frame` snapshot at `origin`. Emits the default-bg fill, per
/// cell background quads, then per-row/per-cell glyphs + decorations. Never
/// `unwrap`s the paint path: `ShapedLine::paint` errors are counted into
/// `RenderStats::paint_errors`.
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
    let mut per_cell_rows = 0u32;
    let mut paint_errors = 0u32;

    for (row_i, row) in frame.grid.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        let (s, e) = if row_needs_per_cell(row) {
            per_cell_rows += 1;
            paint_per_cell_row(row, y, origin.x, frame.default_bg, metrics, window, cx)
        } else {
            per_row_rows += 1;
            paint_per_row_row(row, y, origin.x, frame.default_bg, metrics, window, cx)
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellStyle;

    fn dummy_metrics_fonts() -> CellMetrics {
        let base = gpui::font("Menlo");
        CellMetrics {
            cell_w: px(8.0),
            cell_h: px(16.0),
            font_size: px(14.0),
            font: base.clone(),
            bold_font: base.clone().bold(),
            italic_font: base.clone().italic(),
            bold_italic_font: base.bold().italic(),
        }
    }

    fn cell(col: u16, grapheme: &str, wide: bool) -> FrameCell {
        FrameCell {
            col,
            grapheme: grapheme.to_owned(),
            fg: WHITE,
            bg: None,
            wide,
            selected: false,
            style: CellStyle::default(),
        }
    }

    fn assemble_row_text_for_test(row: &FrameRow) -> String {
        assemble_row(row, Rgb { r: 0, g: 0, b: 0 }, &dummy_metrics_fonts())
            .map(|(t, _)| t)
            .unwrap_or_default()
    }

    #[test]
    fn row_needs_per_cell_detects_wide() {
        let narrow = FrameRow {
            cells: vec![cell(0, "a", false)],
        };
        let wide = FrameRow {
            cells: vec![cell(0, "日", true)],
        };
        assert!(!row_needs_per_cell(&narrow));
        assert!(row_needs_per_cell(&wide));
    }

    #[test]
    fn resolve_maps_italic_bold_inverse_faint_invisible() {
        let m = dummy_metrics_fonts();
        let default_bg = Rgb {
            r: 10,
            g: 10,
            b: 10,
        };

        // bold + italic → bold_italic font.
        let mut c = cell(0, "x", false);
        c.style.bold = true;
        c.style.italic = true;
        assert_eq!(
            resolve_cell_paint(&c, default_bg, &m).font,
            m.bold_italic_font
        );

        // inverse swaps fg/bg (bg None → default_bg becomes fg).
        let mut c = cell(0, "x", false);
        c.fg = Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        c.style.inverse = true;
        let r = resolve_cell_paint(&c, default_bg, &m);
        assert_eq!(r.fg, default_bg);
        assert_eq!(
            r.bg,
            Some(Rgb {
                r: 200,
                g: 100,
                b: 50
            })
        );

        // faint halves fg.
        let mut c = cell(0, "x", false);
        c.fg = Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        c.style.faint = true;
        assert_eq!(
            resolve_cell_paint(&c, default_bg, &m).fg,
            Rgb {
                r: 100,
                g: 50,
                b: 25
            }
        );

        // invisible → skip_glyph.
        let mut c = cell(0, "x", false);
        c.style.invisible = true;
        assert!(resolve_cell_paint(&c, default_bg, &m).skip_glyph);
    }

    #[test]
    fn resolve_curly_underline_sets_textrun_wavy() {
        let m = dummy_metrics_fonts();
        let mut c = cell(0, "u", false);
        c.style.underline = crate::UnderlineStyle::Curly;
        let r = resolve_cell_paint(&c, Rgb { r: 0, g: 0, b: 0 }, &m);
        let ul = r.underline.expect("curly → TextRun underline");
        assert!(ul.wavy);
        assert_eq!(r.underline_quad_kind, UnderlineQuadKind::None);
    }

    #[test]
    fn resolve_double_underline_sets_quad_kind_and_quad_color() {
        let m = dummy_metrics_fonts();
        let mut c = cell(0, "u", false);
        c.style.underline = crate::UnderlineStyle::Double;
        c.style.underline_color = Some(Rgb { r: 0, g: 255, b: 0 });
        let r = resolve_cell_paint(&c, Rgb { r: 0, g: 0, b: 0 }, &m);
        assert!(r.underline.is_none());
        assert_eq!(r.underline_quad_kind, UnderlineQuadKind::Double);
        assert_eq!(r.underline_quad_color, Rgb { r: 0, g: 255, b: 0 });
    }

    #[test]
    fn blink_is_steady_noop() {
        let m = dummy_metrics_fonts();
        let mut c = cell(0, "b", false);
        c.style.blink = true;
        // blink alone never hides the glyph.
        assert!(!resolve_cell_paint(&c, Rgb { r: 0, g: 0, b: 0 }, &m).skip_glyph);
    }

    #[test]
    fn shape_row_invisible_preserves_width() {
        let mut invisible_b = cell(1, "B", false);
        invisible_b.style.invisible = true;
        let row = FrameRow {
            cells: vec![cell(0, "A", false), invisible_b, cell(2, "C", false)],
        };
        // Space where invisible B was — advance preserved.
        assert_eq!(assemble_row_text_for_test(&row), "A C");
    }
}
