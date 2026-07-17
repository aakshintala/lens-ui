//! Shared render state (Slice 1c I6).
//!
//! `TabRenderState` owns `latest_frame` + `cell_metrics` + the **exact** canvas
//! element builder used by both `TerminalTab::render` and the real-window test
//! host — one implementation, no duplicated paint path. Slice 1d only swaps the
//! *source* of `latest_frame` (engine wake sampler); it must not touch this.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Div, FocusHandle, Window, canvas, div, point, prelude::*};

use super::inspect::RenderInspectShared;
use super::metrics::CellMetrics;
use super::paint::{RenderStats, paint_frame};
use crate::Frame;

pub struct TabRenderState {
    pub latest_frame: Option<Arc<Frame>>,
    pub cell_metrics: Option<CellMetrics>,
    pub inspect: RenderInspectShared,
    /// Written every paint; read only via `last_stats` (the test/harness stats
    /// surface). In the normal 1c build there is no reader yet, so suppress
    /// dead-code there — the production stats surface is the Inspect ring.
    #[cfg_attr(not(any(test, feature = "test-util")), allow(dead_code))]
    stats_slot: Rc<RefCell<Option<RenderStats>>>,
}

impl TabRenderState {
    pub fn new() -> Self {
        Self {
            latest_frame: None,
            cell_metrics: None,
            inspect: RenderInspectShared::new(),
            stats_slot: Rc::new(RefCell::new(None)),
        }
    }

    /// Replace the frame to paint on the next render. Slice 1d's engine wake
    /// sampler is the production writer; `set_frame_for_test` delegates here.
    pub(crate) fn set_frame(&mut self, frame: Arc<Frame>) {
        self.latest_frame = Some(frame);
    }

    /// Stats from the most recent completed paint (written by the canvas
    /// closure). `None` until the first paint runs. Test/inspect surface only.
    #[cfg(any(test, feature = "test-util"))]
    pub fn last_stats(&self) -> Option<RenderStats> {
        self.stats_slot.borrow().clone()
    }

    /// Build the focus-tracked div + canvas (or a placeholder when no frame has
    /// arrived). The **one** canvas builder — shared by `TerminalTab::render`
    /// and the real-window harness host.
    pub fn render_element(
        &mut self,
        focus: &FocusHandle,
        placeholder_title: &str,
        lifecycle_dbg: &str,
        window: &mut Window,
        _cx: &mut App,
    ) -> Div {
        if self.cell_metrics.is_none() {
            self.cell_metrics = Some(CellMetrics::resolve_menlo(window));
        }
        let metrics = self.cell_metrics.clone();
        let frame = self.latest_frame.clone();
        let inspect = self.inspect.clone();
        let stats_slot = Rc::clone(&self.stats_slot);
        let placeholder = format!("{placeholder_title} — {lifecycle_dbg}");

        let el = div().track_focus(focus).size_full();
        match frame {
            None => el.child(placeholder),
            Some(frame) => el.child(canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let Some(metrics) = metrics.as_ref() else {
                        return;
                    };
                    let stats = paint_frame(
                        &frame,
                        point(bounds.origin.x, bounds.origin.y),
                        metrics,
                        window,
                        cx,
                    );
                    inspect.record_paint(&stats);
                    *stats_slot.borrow_mut() = Some(stats);
                },
            )),
        }
    }
}

impl Default for TabRenderState {
    fn default() -> Self {
        Self::new()
    }
}
