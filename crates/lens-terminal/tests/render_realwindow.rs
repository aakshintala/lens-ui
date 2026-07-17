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
use std::time::{Duration, Instant};

use gpui::{
    Application, Bounds, Context, FocusHandle, IntoElement, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, canvas, point, prelude::*, px, size,
};
use lens_terminal::Frame;
use lens_terminal::render_test_api::{
    CellMetrics, RenderStats, TabRenderState, ascii_frame, dense_wide_emoji_frame, menlo_gate_ok,
    mixed_ascii_wide_frame, paint_frame, pathological_wide_emoji_frame, sgr_frame,
};

/// Fail-closed paint p95 budgets, calibrated to measured **release** p95 (the
/// gate runs `--release`; debug is ~5.4× slower and unrepresentative — see
/// docs/plans/2026-07-16-terminal-slice-1c-perf-resolution.md). Each budget sits
/// above the observed p95 tail with headroom for a load transient, yet low
/// enough to trip on a ~2× regression. The 120fps product line (8.3ms) is the
/// ceiling all of these clear.
///
/// Observed release p95 (this hardware): ascii ~0.9–1.3ms, wide-200×50
/// ~3.2–3.7ms, wide-400×100 ~4.8–6.2ms, pathological ~2.9–3.3ms (the upper end
/// is under full-gate system load, which adds ~30% to the heavy per-cell phases
/// — budgets carry margin for it so the gate does not flap).
const BUDGET_ASCII_MS: f64 = 3.0;
const BUDGET_WIDE_200_MS: f64 = 5.5;
/// 400×100: the absolute 8.3ms target was re-scoped to Slice 4, but release
/// already meets it — this budget locks in sub-8.3 (no longer a 20ms interim)
/// while clearing the ~6.2ms observed under gate load with margin.
const BUDGET_WIDE_400_MS: f64 = 8.0;
/// Pathological (100%-wide, 50%-emoji) regression guard — a tripwire against
/// gross per-cell degradation, not a target.
const BUDGET_PATHOLOGICAL_MS: f64 = 6.0;
const WARMUP: usize = 60;
const MEASURE: usize = 120;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    ResolveMenlo,
    MenloGate,
    PaintAscii,
    PaintWideRouting,
    PaintSgr,
    PerfAscii200x50,
    PerfWide200x50,
    PerfWide400x100,
    PerfPathological200x50,
    Done,
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    metrics: Rc<RefCell<Option<CellMetrics>>>,
    focus: FocusHandle,
    state: TabRenderState,
    /// Which paint/perf phase's frame is currently loaded (guards the two-frame
    /// dance + one-time perf-frame build).
    setup_phase: Option<Phase>,
    /// Per-paint durations for the current perf phase (warmup + measure).
    samples: Rc<RefCell<Vec<Duration>>>,
    /// The perf phase's frame, built once on phase entry.
    perf_frame: Rc<RefCell<Option<Arc<Frame>>>>,
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            phase: Rc::new(RefCell::new(Phase::ResolveMenlo)),
            metrics: Rc::new(RefCell::new(None)),
            focus: cx.focus_handle(),
            state: TabRenderState::new(),
            setup_phase: None,
            samples: Rc::new(RefCell::new(Vec::new())),
            perf_frame: Rc::new(RefCell::new(None)),
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
                    *self.phase.borrow_mut() = Phase::PerfAscii200x50;
                }
                self.state
                    .render_element(&self.focus, "harness", "PaintSgr", window, cx)
                    .into_any_element()
            }
            Phase::PerfAscii200x50 => self
                .run_perf_phase(
                    phase,
                    || ascii_frame(200, 50, 'a'),
                    BUDGET_ASCII_MS,
                    Phase::PerfWide200x50,
                )
                .into_any_element(),
            Phase::PerfWide200x50 => self
                .run_perf_phase(
                    phase,
                    || dense_wide_emoji_frame(200, 50),
                    BUDGET_WIDE_200_MS,
                    Phase::PerfWide400x100,
                )
                .into_any_element(),
            Phase::PerfWide400x100 => self
                .run_perf_phase(
                    phase,
                    || dense_wide_emoji_frame(400, 100),
                    BUDGET_WIDE_400_MS,
                    Phase::PerfPathological200x50,
                )
                .into_any_element(),
            // Regression guard only (not representative). Generous ceiling.
            Phase::PerfPathological200x50 => self
                .run_perf_phase(
                    phase,
                    || pathological_wide_emoji_frame(200, 50),
                    BUDGET_PATHOLOGICAL_MS,
                    Phase::Done,
                )
                .into_any_element(),
        }
    }
}

fn percentile_ms(samples: &[Duration], p: f64) -> f64 {
    let mut v: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

impl HarnessView {
    /// Time `paint_frame` end-to-end in the real window across WARMUP+MEASURE
    /// paints, then fail-close on the measured p95 vs `budget_ms`. This — not
    /// the spike's paint-closure-CPU number — is the perf verdict (C1/C5). The
    /// frame is built once on phase entry; only the `paint_frame` call is timed.
    fn run_perf_phase(
        &mut self,
        phase: Phase,
        make_frame: impl FnOnce() -> Frame,
        budget_ms: f64,
        next: Phase,
    ) -> impl IntoElement {
        if self.setup_phase != Some(phase) {
            *self.perf_frame.borrow_mut() = Some(Arc::new(make_frame()));
            self.samples.borrow_mut().clear();
            self.setup_phase = Some(phase);
        }
        let metrics_cell = Rc::clone(&self.metrics);
        let samples_cell = Rc::clone(&self.samples);
        let frame_cell = Rc::clone(&self.perf_frame);
        let phase_cell = Rc::clone(&self.phase);
        canvas(
            |_, _, _| {},
            move |bounds, _prepaint, window, cx| {
                let m = metrics_cell.borrow();
                let Some(metrics) = m.as_ref() else {
                    return;
                };
                let f = frame_cell.borrow();
                let Some(frame) = f.as_ref() else {
                    return;
                };
                let t0 = Instant::now();
                let _stats = paint_frame(
                    frame,
                    point(bounds.origin.x, bounds.origin.y),
                    metrics,
                    window,
                    cx,
                );
                let dt = t0.elapsed();

                let mut samples = samples_cell.borrow_mut();
                samples.push(dt);
                if samples.len() >= WARMUP + MEASURE {
                    let p95 = percentile_ms(&samples[WARMUP..], 0.95);
                    eprintln!("SMOKE {phase:?} p95_ms={p95:.3} budget_ms={budget_ms}");
                    if p95 > budget_ms {
                        fail(&format!(
                            "{phase:?} p95 {p95:.3}ms > budget {budget_ms}ms (release-calibrated; investigate a regression before raising — see the perf-resolution plan)"
                        ));
                    }
                    drop(samples);
                    *phase_cell.borrow_mut() = next;
                }
            },
        )
        .size_full()
    }

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
