//! Real-window render harness (Slice 1c C1/C5).
//!
//! NOT under `#[gpui::test]`: gpui's test platform installs a `NoopTextSystem`
//! that fakes every font/shape/paint result, so Menlo/paint/perf assertions
//! there are false-green (memory `gpui-test-noop-text-system`). This is a
//! `harness = false` binary that opens a **real** `Application::new().run()`
//! window and drives assertions from the canvas paint callback.
//!
//! `cx.quit()` exits the *process* on the macOS gpui path, so all workloads run
//! in ONE `Application::run`, sequentially. The canvas paint closure only sees
//! `&mut App` (not the entity), so the phase machine + shared state live in
//! `Rc<RefCell<_>>` cells the closure mutates; `render()` re-dispatches on the
//! next animation frame. On failure: `std::process::exit(1)`. On full success:
//! `std::process::exit(0)`. xtask executes this on macOS with `test-util`.
//!
//! Tasks 3–8 grow the `Phase` machine (paint, SGR, perf).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Application, Bounds, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, canvas, prelude::*, px, size,
};
use lens_terminal::render_test_api::{CellMetrics, menlo_gate_ok};

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
            |_window, _cx_win| _cx_win.new(|_cx| HarnessView::new()),
        )
        .expect("open_window");
        cx.activate(true);
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    ResolveMenlo,
    MenloGate,
    Done,
    // Tasks 3+: PaintAscii, PaintWideRouting, PaintSgr,
    // PerfAscii200x50, PerfWide200x50, PerfWide400x100.
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    metrics: Rc<RefCell<Option<CellMetrics>>>,
}

impl HarnessView {
    fn new() -> Self {
        Self {
            phase: Rc::new(RefCell::new(Phase::ResolveMenlo)),
            metrics: Rc::new(RefCell::new(None)),
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("render_realwindow FAIL: {msg}");
    std::process::exit(1);
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        let phase = *self.phase.borrow();
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
                    *phase_cell.borrow_mut() = Phase::Done;
                }
                Phase::Done => {
                    println!("render_realwindow: all phases OK");
                    std::process::exit(0);
                }
            },
        )
        .size_full()
    }
}
