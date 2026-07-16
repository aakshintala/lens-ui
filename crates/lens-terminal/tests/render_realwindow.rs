//! Real-window render harness (Slice 1c C1/C5).
//!
//! NOT under `#[gpui::test]`: gpui's test platform installs a `NoopTextSystem`
//! that fakes every font/shape/paint result, so Menlo/paint/perf assertions
//! there are false-green (memory `gpui-test-noop-text-system`). This is a
//! `harness = false` binary that opens a **real** `Application::new().run()`
//! window and drives assertions from the canvas paint callback.
//!
//! `cx.quit()` exits the *process* on the macOS gpui path, so all workloads run
//! in ONE `Application::run`, sequentially. On failure: `std::process::exit(1)`.
//! On full success: `std::process::exit(0)`. xtask executes this on macOS with
//! `--features test-util`.
//!
//! Tasks 2–8 grow the `Phase` state machine (Menlo gate, paint, SGR, perf).
//! Task 1 asserts only that Menlo resolves to the real Menlo family.

use gpui::{
    Application, Bounds, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, canvas, prelude::*, px, size,
};
use lens_terminal::render_test_api::CellMetrics;

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
            |_window, cx| {
                cx.new(|_cx| HarnessView {
                    phase: Phase::ResolveMenlo,
                })
            },
        )
        .expect("open_window");
        cx.activate(true);
    });
}

#[derive(Clone, Copy)]
enum Phase {
    ResolveMenlo,
    // Task 2+: MenloGate, PaintAscii, PaintWideRouting, PaintSgr,
    // PerfAscii200x50, PerfWide200x50, PerfWide400x100, Done.
}

struct HarnessView {
    phase: Phase,
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        match self.phase {
            Phase::ResolveMenlo => canvas(
                |_, _, _| {},
                move |_bounds, _prepaint, window, _cx| {
                    let m = CellMetrics::resolve_menlo(window);
                    let font_id = window.text_system().resolve_font(&m.font);
                    let family = window
                        .text_system()
                        .get_font_for_id(font_id)
                        .map(|f| f.family.to_string())
                        .unwrap_or_default();
                    if family != "Menlo" {
                        eprintln!("resolve_menlo: expected family Menlo, got {family:?}");
                        std::process::exit(1);
                    }
                    // Task 1 terminal exit; T2+ advance the phase machine instead.
                    println!("render_realwindow: ResolveMenlo OK (family=Menlo)");
                    std::process::exit(0);
                },
            )
            .size_full(),
        }
    }
}
