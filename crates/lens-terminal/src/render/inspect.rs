//! Render Inspect ring (Slice 1c). Zero-cost when disabled: `record_paint`
//! returns before touching the ring. Mirrors the engine Inspect shape
//! (`engine/inspect.rs`) but UI-thread-only, so it uses `Rc<RefCell>` rather
//! than atomics. The Inspect snapshot is the **production** paint-stats surface
//! (the harness reads exact last stats via `TabRenderState::last_stats`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde::Serialize;

use super::paint::RenderStats;

const RING_CAP: usize = 32;

/// One recorded paint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenderInspectEvent {
    pub kind: RenderInspectEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RenderInspectEventKind {
    FramePainted {
        micros: u64,
        rows_painted: u32,
        cells_bg: u32,
        shapes: u32,
        per_row_rows: u32,
        per_cell_rows: u32,
        paint_errors: u32,
    },
}

/// Serializable snapshot of the render Inspect state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenderInspect {
    pub enabled: bool,
    pub frames_painted: u64,
    pub last_paint_micros: u64,
    pub last_rows_painted: u32,
    pub last_cells_bg: u32,
    pub last_shapes: u32,
    pub last_per_row_rows: u32,
    pub last_per_cell_rows: u32,
    pub last_paint_errors: u32,
    pub recent: Vec<RenderInspectEvent>,
}

#[derive(Default)]
struct Inner {
    enabled: bool,
    frames_painted: u64,
    last_paint_micros: u64,
    last_rows_painted: u32,
    last_cells_bg: u32,
    last_shapes: u32,
    last_per_row_rows: u32,
    last_per_cell_rows: u32,
    last_paint_errors: u32,
    recent: VecDeque<RenderInspectEvent>,
}

/// Cheaply-cloneable handle the canvas paint closure records into.
#[derive(Clone)]
pub struct RenderInspectShared {
    inner: Rc<RefCell<Inner>>,
}

impl RenderInspectShared {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner::default())),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = self.inner.borrow_mut();
        inner.enabled = enabled;
        if !enabled {
            inner.recent.clear();
        }
    }

    /// Record a completed paint. Returns immediately (zero-cost) when disabled.
    pub fn record_paint(&self, stats: &RenderStats) {
        let mut inner = self.inner.borrow_mut();
        if !inner.enabled {
            return;
        }
        inner.frames_painted += 1;
        inner.last_paint_micros = stats.paint_micros;
        inner.last_rows_painted = stats.rows_painted;
        inner.last_cells_bg = stats.cells_bg;
        inner.last_shapes = stats.shapes;
        inner.last_per_row_rows = stats.per_row_rows;
        inner.last_per_cell_rows = stats.per_cell_rows;
        inner.last_paint_errors = stats.paint_errors;
        inner.recent.push_back(RenderInspectEvent {
            kind: RenderInspectEventKind::FramePainted {
                micros: stats.paint_micros,
                rows_painted: stats.rows_painted,
                cells_bg: stats.cells_bg,
                shapes: stats.shapes,
                per_row_rows: stats.per_row_rows,
                per_cell_rows: stats.per_cell_rows,
                paint_errors: stats.paint_errors,
            },
        });
        while inner.recent.len() > RING_CAP {
            inner.recent.pop_front();
        }
    }

    pub fn snapshot(&self) -> RenderInspect {
        let inner = self.inner.borrow();
        RenderInspect {
            enabled: inner.enabled,
            frames_painted: inner.frames_painted,
            last_paint_micros: inner.last_paint_micros,
            last_rows_painted: inner.last_rows_painted,
            last_cells_bg: inner.last_cells_bg,
            last_shapes: inner.last_shapes,
            last_per_row_rows: inner.last_per_row_rows,
            last_per_cell_rows: inner.last_per_cell_rows,
            last_paint_errors: inner.last_paint_errors,
            recent: inner.recent.iter().cloned().collect(),
        }
    }
}

impl Default for RenderInspectShared {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(micros: u64) -> RenderStats {
        RenderStats {
            rows_painted: 10,
            cells_bg: 3,
            shapes: 10,
            per_row_rows: 9,
            per_cell_rows: 1,
            paint_errors: 0,
            paint_micros: micros,
        }
    }

    #[test]
    fn render_inspect_ring_empty_when_disabled_and_records_when_enabled() {
        let shared = RenderInspectShared::new();

        // Disabled: no-op.
        shared.record_paint(&stats(123));
        assert!(shared.snapshot().recent.is_empty());
        assert_eq!(shared.snapshot().frames_painted, 0);

        // Enabled: records.
        shared.set_enabled(true);
        shared.record_paint(&stats(123));
        let snap = shared.snapshot();
        assert_eq!(snap.frames_painted, 1);
        assert_eq!(snap.last_paint_micros, 123);
        assert_eq!(snap.last_rows_painted, 10);
        assert!(matches!(
            snap.recent[0].kind,
            RenderInspectEventKind::FramePainted { micros: 123, .. }
        ));

        // Disable clears the ring.
        shared.set_enabled(false);
        assert!(shared.snapshot().recent.is_empty());
    }

    #[test]
    fn render_inspect_ring_bounded_to_cap() {
        let shared = RenderInspectShared::new();
        shared.set_enabled(true);
        for i in 0..(RING_CAP as u64 + 10) {
            shared.record_paint(&stats(i));
        }
        let snap = shared.snapshot();
        assert_eq!(snap.recent.len(), RING_CAP);
        assert_eq!(snap.frames_painted, RING_CAP as u64 + 10);
    }
}
