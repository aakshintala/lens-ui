//! Full-snapshot `Frame` painter (Slice 1c).
//!
//! Lifts the spike's per-row/per-cell glyph placement onto Lens-owned `Frame`
//! values (no `RowShapeCache`, no `libghostty_vt` types). `paint_frame` stays
//! `pub` inside the private `render` module — never re-exported at the crate
//! root — so it does not enter the public API (I12).

use gpui::{App, Pixels, Point, Window};

use super::metrics::CellMetrics;
use crate::Frame;

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

/// Paint a full `Frame` snapshot at `origin`. Stub until Task 3.
pub fn paint_frame(
    _frame: &Frame,
    _origin: Point<Pixels>,
    _metrics: &CellMetrics,
    _window: &mut Window,
    _cx: &mut App,
) -> RenderStats {
    RenderStats::default()
}
