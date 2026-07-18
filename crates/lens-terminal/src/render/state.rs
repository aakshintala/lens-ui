//! Shared render state (Slice 1c I6).
//!
//! `TabRenderState` owns `latest_frame` + `cell_metrics` + the **exact** canvas
//! element builder used by both `TerminalTab::render` and the real-window test
//! host — one implementation, no duplicated paint path. Slice 1d only swaps the
//! *source* of `latest_frame` (engine wake sampler); it must not touch this.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, Div, ElementInputHandler, Entity, FocusHandle, Pixels, Window, canvas, div, point,
    prelude::*, size,
};

use super::inspect::RenderInspectShared;
use super::metrics::CellMetrics;
use super::paint::{RenderStats, paint_frame, paint_preedit_overlay};
use crate::Frame;
use crate::TerminalTab;

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
    /// sampler is the production writer; harness hosts call this directly.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_frame(&mut self, frame: Arc<Frame>) {
        self.latest_frame = Some(frame);
    }

    #[cfg(not(any(test, feature = "test-util")))]
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
    ///
    /// When `input` is `Some`, registers keydown/keyup + [`Window::handle_input`]
    /// for the production terminal path. Paint-only harnesses pass `None`.
    pub fn render_element(
        &mut self,
        focus: &FocusHandle,
        placeholder_title: &str,
        lifecycle_dbg: &str,
        input: Option<(Option<&str>, Entity<TerminalTab>)>,
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
        let preedit_owned = input.as_ref().and_then(|(p, _)| (*p).map(str::to_owned));
        let focus_for_input = focus.clone();
        let input_tab = input.as_ref().map(|(_, tab)| tab.clone());

        let mut el = div().track_focus(focus).size_full();
        if let Some(tab) = input_tab.clone() {
            let tab_down = tab.clone();
            let tab_up = tab.clone();
            let tab_scroll = tab;
            el = el
                .on_key_down(move |event, window, cx| {
                    tab_down.update(cx, |tab, cx| tab.handle_key_down(event, window, cx));
                })
                .on_key_up(move |event, window, cx| {
                    tab_up.update(cx, |tab, cx| tab.handle_key_up(event, window, cx));
                })
                .on_scroll_wheel(move |event, _window, cx| {
                    tab_scroll.update(cx, |tab, cx| tab.handle_scroll_wheel(event, cx));
                });
        }
        match frame {
            None => {
                if let Some(tab) = input_tab {
                    let focus = focus_for_input.clone();
                    let metrics = metrics.clone();
                    el = el.child(canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            let ime_bounds = default_ime_bounds(bounds, None, metrics.as_ref());
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(ime_bounds, tab),
                                cx,
                            );
                        },
                    ));
                }
                el.child(placeholder)
            }
            Some(frame) => {
                let input_tab_paint = input_tab.clone();
                el.child(canvas(
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
                        if let (Some(preedit), Some(cursor)) =
                            (preedit_owned.as_deref(), frame.cursor)
                        {
                            let _ = paint_preedit_overlay(
                                preedit,
                                cursor,
                                point(bounds.origin.x, bounds.origin.y),
                                metrics,
                                window,
                                cx,
                            );
                        }
                        if let Some(tab) = input_tab_paint {
                            let ime_bounds =
                                default_ime_bounds(bounds, Some(&frame), Some(metrics));
                            window.handle_input(
                                &focus_for_input,
                                ElementInputHandler::new(ime_bounds, tab),
                                cx,
                            );
                        }
                        inspect.record_paint(&stats);
                        *stats_slot.borrow_mut() = Some(stats);
                    },
                ))
            }
        }
    }
}

/// IME anchor when no cursor is available yet (pre-first-frame).
fn default_ime_bounds(
    element_bounds: Bounds<Pixels>,
    frame: Option<&Frame>,
    metrics: Option<&CellMetrics>,
) -> Bounds<Pixels> {
    if let (Some(frame), Some(metrics)) = (frame, metrics)
        && let Some(cursor) = frame.cursor
    {
        let x = element_bounds.origin.x + metrics.cell_w * f32::from(cursor.col);
        let y = element_bounds.origin.y + metrics.cell_h * f32::from(cursor.row);
        return Bounds::new(point(x, y), size(metrics.cell_w, metrics.cell_h));
    }
    if let Some(metrics) = metrics {
        Bounds::new(element_bounds.origin, size(metrics.cell_w, metrics.cell_h))
    } else {
        element_bounds
    }
}

impl Default for TabRenderState {
    fn default() -> Self {
        Self::new()
    }
}
