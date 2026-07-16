//! Liftable cell-grid → GPUI paint mapping.
//!
//! Keep this free of harness/timing concerns — it is the kept artifact from
//! the spike. Strategies:
//! - **S1**: per-row `shape_line` every frame (no cache)
//! - **S2**: per-row `shape_line` with content-keyed `ShapedLine` cache
//! - **PerCell**: fallback when per-row shaping misaligns wide/emoji grids

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use gpui::{
    App, Bounds, Font, Hsla, Pixels, Point, Rgba, ShapedLine, SharedString, TextRun, Window, fill,
    font, point, px, size,
};
use libghostty_vt::render::{CellIterator, Dirty, RowIterator, Snapshot};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::RgbColor;

/// How text glyphs are placed on the fixed cell grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextPlacement {
    /// Shape a whole row as one line (S1 / S2).
    PerRow,
    /// Shape/paint each cell at `col * cell_w` (alignment fallback).
    PerCell,
}

/// Paint strategy for the decision sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// Per-row shape, no cache.
    S1,
    /// Per-row shape, cached by row-content hash.
    S2,
}

/// Cell metrics computed once from the window text system.
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub cell_w: Pixels,
    pub cell_h: Pixels,
    pub font_size: Pixels,
    pub font: Font,
    pub bold_font: Font,
}

impl CellMetrics {
    pub fn resolve(window: &Window) -> Self {
        let font_size = px(14.0);
        let base = font(".ZedMono");
        let bold = base.clone().bold();
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
        }
    }
}

/// Per-row `ShapedLine` cache for S2.
#[derive(Default)]
pub struct RowShapeCache {
    entries: HashMap<u64, ShapedLine>,
    hits: u64,
    misses: u64,
}

impl RowShapeCache {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }
}

/// Optional counters returned to the harness (not required for painting).
#[derive(Clone, Copy, Debug, Default)]
pub struct PaintCounters {
    pub rows_painted: u32,
    pub cells_bg: u32,
    pub shapes: u32,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

fn rgb_to_rgba(c: RgbColor) -> Rgba {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    }
}

fn rgb_to_hsla(c: RgbColor) -> Hsla {
    Hsla::from(rgb_to_rgba(c))
}

#[derive(Clone)]
struct CellPaint {
    col: u16,
    grapheme: String,
    fg: RgbColor,
    bg: Option<RgbColor>,
    bold: bool,
    wide: bool,
}

#[derive(Clone)]
struct RowPaint {
    cells: Vec<CellPaint>,
    content_hash: u64,
}

fn collect_rows<'alloc>(
    snapshot: &Snapshot<'alloc, '_>,
    rows: &mut RowIterator<'alloc>,
    cells: &mut CellIterator<'alloc>,
    default_fg: RgbColor,
) -> libghostty_vt::error::Result<Vec<RowPaint>> {
    let mut out = Vec::new();
    let mut row_iter = rows.update(snapshot)?;
    while let Some(row) = row_iter.next() {
        let mut cell_iter = cells.update(&row)?;
        let mut row_cells = Vec::new();
        let mut col: u16 = 0;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        while let Some(cell) = cell_iter.next() {
            let this_col = col;
            col = col.saturating_add(1);

            let wide = cell.raw_cell()?.wide()?;
            if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                continue;
            }

            let graphemes = cell.graphemes()?;
            let fg = cell.fg_color()?.unwrap_or(default_fg);
            let bg = cell.bg_color()?;
            let style = cell.style()?;
            let selected = cell.is_selected()?;
            let bold = style.bold;
            let grapheme: String = if graphemes.is_empty() {
                " ".to_string()
            } else {
                graphemes.iter().collect()
            };

            grapheme.hash(&mut hasher);
            fg.r.hash(&mut hasher);
            fg.g.hash(&mut hasher);
            fg.b.hash(&mut hasher);
            match bg {
                Some(c) => {
                    1u8.hash(&mut hasher);
                    c.r.hash(&mut hasher);
                    c.g.hash(&mut hasher);
                    c.b.hash(&mut hasher);
                }
                None => 0u8.hash(&mut hasher),
            }
            bold.hash(&mut hasher);
            selected.hash(&mut hasher);

            let bg = if selected {
                Some(RgbColor {
                    r: 40,
                    g: 60,
                    b: 120,
                })
            } else {
                bg
            };

            row_cells.push(CellPaint {
                col: this_col,
                grapheme,
                fg,
                bg,
                bold,
                wide: matches!(wide, CellWide::Wide),
            });
        }
        let _ = row.set_dirty(false);
        out.push(RowPaint {
            content_hash: hasher.finish(),
            cells: row_cells,
        });
    }
    let _ = snapshot.set_dirty(Dirty::Clean);
    Ok(out)
}

fn paint_backgrounds(
    rows: &[RowPaint],
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
) -> u32 {
    let mut n = 0u32;
    for (row_i, row) in rows.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        for cell in &row.cells {
            let Some(bg) = cell.bg else {
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

fn shape_row_line(row: &RowPaint, metrics: &CellMetrics, window: &Window) -> Option<ShapedLine> {
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
        let run_font = if cell.bold {
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
    Some(window.text_system().shape_line(
        SharedString::from(text),
        metrics.font_size,
        &runs,
        None,
    ))
}

fn paint_per_row(
    rows: &[RowPaint],
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    strategy: Strategy,
    mut cache: Option<&mut RowShapeCache>,
    window: &mut Window,
    cx: &mut App,
) -> (u32, u64, u64) {
    let mut shapes = 0u32;
    let mut hits = 0u64;
    let mut misses = 0u64;
    for (row_i, row) in rows.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        let origin_pt = point(origin.x, y);

        let shaped = match (strategy, cache.as_deref_mut()) {
            (Strategy::S2, Some(cache)) => {
                if let Some(hit) = cache.entries.get(&row.content_hash) {
                    cache.hits += 1;
                    hits += 1;
                    hit.clone()
                } else {
                    cache.misses += 1;
                    misses += 1;
                    let Some(line) = shape_row_line(row, metrics, window) else {
                        continue;
                    };
                    shapes += 1;
                    cache.entries.insert(row.content_hash, line.clone());
                    line
                }
            }
            _ => {
                let Some(line) = shape_row_line(row, metrics, window) else {
                    continue;
                };
                shapes += 1;
                line
            }
        };

        let _ = shaped.paint(origin_pt, metrics.cell_h, window, cx);
    }
    (shapes, hits, misses)
}

fn paint_per_cell(
    rows: &[RowPaint],
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> u32 {
    let mut shapes = 0u32;
    for (row_i, row) in rows.iter().enumerate() {
        let y = origin.y + metrics.cell_h * (row_i as f32);
        for cell in &row.cells {
            if cell.grapheme == " " {
                continue;
            }
            let x = origin.x + metrics.cell_w * f32::from(cell.col);
            let run_font = if cell.bold {
                metrics.bold_font.clone()
            } else {
                metrics.font.clone()
            };
            let color = rgb_to_hsla(cell.fg);
            let text = SharedString::from(cell.grapheme.clone());
            let run = TextRun {
                len: text.len(),
                font: run_font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped =
                window
                    .text_system()
                    .shape_line(text, metrics.font_size, &[run], None);
            shapes += 1;
            let _ = shaped.paint(point(x, y), metrics.cell_h, window, cx);
        }
    }
    shapes
}

/// Probe whether per-row `shape_line` keeps wide/emoji on the monospace grid.
///
/// Returns `true` when a wide CJK + emoji sample's shaped advances stay within
/// ~0.75px of the expected `col * cell_w`.
pub fn per_row_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool {
    let sample = "a日b😀c";
    let run = TextRun {
        len: sample.len(),
        font: metrics.font.clone(),
        color: rgb_to_hsla(RgbColor {
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
    let idx_cjk = sample.find('日').unwrap();
    let idx_emoji = sample.find('😀').unwrap();
    let x_cjk = shaped.x_for_index(idx_cjk);
    let x_emoji = shaped.x_for_index(idx_emoji);
    let tol = px(0.75);
    let cjk_ok = (x_cjk - metrics.cell_w).abs() <= tol;
    // a(1) + 日(2) + b(1) = 4 cells before emoji
    let emoji_ok = (x_emoji - metrics.cell_w * 4.0).abs() <= tol;
    cjk_ok && emoji_ok
}

/// Paint a full terminal snapshot into the window.
///
/// Liftable mapping: iterate Ghostty render rows/cells, emit background quads
/// for non-default backgrounds, and place glyphs via strategy / placement.
pub fn paint_grid<'alloc>(
    snapshot: &Snapshot<'alloc, '_>,
    rows: &mut RowIterator<'alloc>,
    cells: &mut CellIterator<'alloc>,
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    strategy: Strategy,
    placement: TextPlacement,
    cache: Option<&mut RowShapeCache>,
    window: &mut Window,
    cx: &mut App,
) -> libghostty_vt::error::Result<PaintCounters> {
    let colors = snapshot.colors()?;
    let default_bg = colors.background;
    let default_fg = colors.foreground;

    let (ncols, nrows) = (snapshot.cols()?, snapshot.rows()?);
    let grid_bounds = Bounds::new(
        origin,
        size(
            metrics.cell_w * f32::from(ncols),
            metrics.cell_h * f32::from(nrows),
        ),
    );
    window.paint_quad(fill(grid_bounds, rgb_to_rgba(default_bg)));

    let row_data = collect_rows(snapshot, rows, cells, default_fg)?;
    let cells_bg = paint_backgrounds(&row_data, origin, metrics, window);

    let (shapes, hits, misses) = match placement {
        TextPlacement::PerRow => {
            paint_per_row(&row_data, origin, metrics, strategy, cache, window, cx)
        }
        TextPlacement::PerCell => {
            let shapes = paint_per_cell(&row_data, origin, metrics, window, cx);
            (shapes, 0, 0)
        }
    };

    Ok(PaintCounters {
        rows_painted: row_data.len() as u32,
        cells_bg,
        shapes,
        cache_hits: hits,
        cache_misses: misses,
    })
}
