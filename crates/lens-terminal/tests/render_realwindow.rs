//! Real-window render harness (Slice 1c C1/C5).
//!
//! NOT under `#[gpui::test]`: gpui's test platform installs a `NoopTextSystem`
//! that fakes every font/shape/paint result, so Menlo/paint/perf assertions
//! there are false-green (memory `gpui-test-noop-text-system`). This is a
//! `harness = false` binary that opens a **real** `Application::new().run()`
//! window and drives assertions from the canvas paint callback.
//!
//! `cx.quit()` exits the *process* on the macOS gpui path, so all workloads run
//! in ONE `Application::run`, sequentially. Phase state lives in `Rc<RefCell>`
//! cells (gate/perf phases advance from inside the canvas paint closure, which
//! only sees `&mut App`); paint phases use a two-frame dance — frame A sets the
//! frame, the canvas paints it, frame B reads `TabRenderState::last_stats()`.
//! `setup_phase` guards frame A from reading the *previous* phase's stale stats.
//! On failure: `std::process::exit(1)`; on success: `std::process::exit(0)`.
//! xtask runs this on macOS with `--features test-util`.
//!
//! Tasks 5–8 grow the `Phase` machine (SGR, perf).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Application, Bounds, Context, FocusHandle, IntoElement, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, canvas, prelude::*, px, size,
};
use lens_terminal::Frame;
use lens_terminal::render_test_api::{
    CellMetrics, RenderStats, TabRenderState, ascii_frame, menlo_gate_ok, mixed_ascii_wide_frame,
    sgr_frame,
};

fn main() {
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal render_realwindow".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(800.0), px(600.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_window, cx| cx.new(HarnessView::new),
        )
        .expect("open_window");
        cx.activate(true);
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    ResolveMenlo,
    MenloGate,
    PaintAscii,
    PaintWideRouting,
    PaintSgr,
    Done,
    // Tasks 8: PerfAscii200x50, PerfWide200x50, PerfWide400x100.
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    metrics: Rc<RefCell<Option<CellMetrics>>>,
    focus: FocusHandle,
    state: TabRenderState,
    /// Which paint phase's frame is currently loaded (guards the two-frame
    /// dance from reading the previous phase's stats).
    setup_phase: Option<Phase>,
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            phase: Rc::new(RefCell::new(Phase::ResolveMenlo)),
            metrics: Rc::new(RefCell::new(None)),
            focus: cx.focus_handle(),
            state: TabRenderState::new(),
            setup_phase: None,
        }
    }

    /// On the first frame of a paint phase, load its frame and return `None`.
    /// On later frames, return the stats from painting it.
    fn paint_phase_stats(
        &mut self,
        phase: Phase,
        make_frame: impl FnOnce() -> Frame,
    ) -> Option<RenderStats> {
        if self.setup_phase != Some(phase) {
            self.state.set_frame(Arc::new(make_frame()));
            self.setup_phase = Some(phase);
            None
        } else {
            self.state.last_stats()
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("render_realwindow FAIL: {msg}");
    std::process::exit(1);
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        let phase = *self.phase.borrow();
        match phase {
            Phase::ResolveMenlo | Phase::MenloGate | Phase::Done => {
                self.gate_canvas(phase).into_any_element()
            }
            Phase::PaintAscii => {
                if let Some(stats) = self.paint_phase_stats(phase, || ascii_frame(40, 10, 'A')) {
                    if stats.rows_painted != 10 || stats.paint_errors != 0 || stats.shapes < 1 {
                        fail(&format!("PaintAscii stats bad: {stats:?}"));
                    }
                    println!("render_realwindow: PaintAscii OK ({stats:?})");
                    *self.phase.borrow_mut() = Phase::PaintWideRouting;
                }
                self.state
                    .render_element(&self.focus, "harness", "PaintAscii", window, cx)
                    .into_any_element()
            }
            Phase::PaintWideRouting => {
                if let Some(stats) = self.paint_phase_stats(phase, || mixed_ascii_wide_frame(20, 2))
                {
                    if stats.per_row_rows != 1
                        || stats.per_cell_rows != 1
                        || stats.paint_errors != 0
                    {
                        fail(&format!("PaintWideRouting stats bad: {stats:?}"));
                    }
                    println!("render_realwindow: PaintWideRouting OK ({stats:?})");
                    *self.phase.borrow_mut() = Phase::PaintSgr;
                }
                self.state
                    .render_element(&self.focus, "harness", "PaintWideRouting", window, cx)
                    .into_any_element()
            }
            Phase::PaintSgr => {
                if let Some(stats) = self.paint_phase_stats(phase, sgr_frame) {
                    if stats.rows_painted != 1 || stats.paint_errors != 0 || stats.shapes < 1 {
                        fail(&format!("PaintSgr stats bad: {stats:?}"));
                    }
                    println!("render_realwindow: PaintSgr OK ({stats:?})");
                    *self.phase.borrow_mut() = Phase::Done;
                }
                self.state
                    .render_element(&self.focus, "harness", "PaintSgr", window, cx)
                    .into_any_element()
            }
        }
    }
}

impl HarnessView {
    /// Manual canvas for the non-paint gate phases (assertions run in the paint
    /// closure, which only sees `&mut App`, so state is shared via `Rc`).
    fn gate_canvas(&self, phase: Phase) -> impl IntoElement {
        let phase_cell = Rc::clone(&self.phase);
        let metrics_cell = Rc::clone(&self.metrics);
        canvas(
            |_, _, _| {},
            move |_bounds, _prepaint, window, _cx| match phase {
                Phase::ResolveMenlo => {
                    *metrics_cell.borrow_mut() = Some(CellMetrics::resolve_menlo(window));
                    *phase_cell.borrow_mut() = Phase::MenloGate;
                }
                Phase::MenloGate => {
                    let m = metrics_cell.borrow();
                    let metrics = m.as_ref().expect("metrics resolved");
                    let gate = menlo_gate_ok(window, metrics);
                    if !gate.ok {
                        fail(&format!(
                            "Menlo gate: {} — reopen lens-fonts; do not soft-pass",
                            gate.reason
                        ));
                    }
                    println!("render_realwindow: MenloGate OK");
                    *phase_cell.borrow_mut() = Phase::PaintAscii;
                }
                Phase::Done => {
                    println!("render_realwindow: all phases OK");
                    std::process::exit(0);
                }
                _ => {}
            },
        )
        .size_full()
    }
}
