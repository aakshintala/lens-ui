//! Job-A sustained multi-tab streaming perf gate (Slice 3).
//!
//! Sibling of `render_realwindow.rs` — same rationale for `harness = false` +
//! real `Application::run` (gpui's test `NoopTextSystem` false-greens paint/perf
//! assertions; memory `gpui-test-noop-text-system`). Where `render_realwindow`
//! paints ONE static frame, this drives N live engines fed a sustained
//! synthetic dense-wide/emoji stream from a background thread, paints the
//! VISIBLE tab every frame (PerCell path), and fail-closes on BOTH the paint
//! p95 (main thread) and the engine build p95 (from `EngineInspect`) under
//! load. It also flips a hidden tab visible mid-run and asserts hidden tabs
//! suppress builds. Process ΔRSS is recorded informationally (not asserted).
//!
//! Default run is a SHORT burst so it fits the macOS `xtask gate`. Set
//! `LENS_STREAM_SOAK=1` for a longer soak at slice acceptance.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    Application, Bounds, Context, FocusHandle, IntoElement, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, canvas, point, prelude::*, px, size,
};
use lens_terminal::render_test_api::{CellMetrics, TabRenderState, paint_frame};
use lens_terminal::{EngineConfig, EngineHandle};

const TAB_COUNT: usize = 4;
const COLS: u16 = 200;
const ROWS: u16 = 50;
const BUILD_P95_MS: f64 = 3.0;
const PAINT_P95_MS: f64 = 5.5;
const WARMUP: usize = 60;

fn measure_frames() -> usize {
    if std::env::var("LENS_STREAM_SOAK").ok().as_deref() == Some("1") {
        1200
    } else {
        240
    }
}

fn rss_bytes() -> u64 {
    let pid = std::process::id();
    match std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .map(|kib| kib * 1024)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// One CRLF-terminated line of dense wide/emoji content (exercises PerCell).
fn dense_line() -> Vec<u8> {
    let mut s = String::new();
    while s.chars().count() < COLS as usize {
        s.push_str("日本語😀AB");
    }
    let mut b = s.into_bytes();
    b.extend_from_slice(b"\r\n");
    b
}

fn fail(msg: &str) -> ! {
    eprintln!("stream_perf_realwindow FAIL: {msg}");
    std::process::exit(1);
}

fn percentile_ms(samples: &[Duration], p: f64) -> f64 {
    let mut v: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn main() {
    // Spawn N engines. Tab 0 starts VISIBLE and painted; tabs 1..N start hidden
    // (streamed but not built) and tab 1 is flipped visible mid-run.
    let cfg = EngineConfig {
        cols: COLS,
        rows: ROWS,
        // BYTE budget (~10 MiB, production-ish). Under sustained streaming the
        // byte cap binds and old rows drop — realistic for a perf test.
        max_scrollback: 10_000_000,
        cell_w_px: 8,
        cell_h_px: 16,
    };
    let engines: Vec<Arc<EngineHandle>> = (0..TAB_COUNT)
        .map(|_| Arc::new(EngineHandle::spawn(cfg)))
        .collect();
    for (i, e) in engines.iter().enumerate() {
        e.set_inspect_enabled(true);
        // Only tab 0 visible initially.
        let _ = e.set_visible(i == 0);
    }

    // Background feeder: stream dense lines into every engine continuously until
    // `stop` flips. Retries on backpressure.
    let stop = Arc::new(AtomicBool::new(false));
    let feeder_engines: Vec<Arc<EngineHandle>> = engines.iter().map(Arc::clone).collect();
    let feeder_stop = Arc::clone(&stop);
    let feeder = std::thread::spawn(move || {
        while !feeder_stop.load(Ordering::Relaxed) {
            for e in &feeder_engines {
                let line = dense_line();
                let _ = e.feed(line); // drop on Full — sustained pressure is the point
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });

    let rss_start = rss_bytes();
    let measure = measure_frames();

    Application::new().run(move |cx| {
        let engines = engines.clone();
        let stop = Arc::clone(&stop);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal stream_perf_realwindow".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_window, cx| cx.new(|cx| StreamView::new(engines, stop, rss_start, measure, cx)),
        )
        .expect("open_window");
        cx.activate(true);
    });

    let _ = feeder.join();
}

struct StreamView {
    engines: Vec<Arc<EngineHandle>>,
    stop: Arc<AtomicBool>,
    #[allow(dead_code)]
    focus: FocusHandle,
    state: TabRenderState,
    metrics: Rc<RefCell<Option<CellMetrics>>>,
    paint_samples: Rc<RefCell<Vec<Duration>>>,
    /// Max `last_build_micros` seen per frame across visible engines.
    build_samples: Rc<RefCell<Vec<Duration>>>,
    frame_idx: Rc<RefCell<usize>>,
    flipped: Rc<RefCell<bool>>,
    hidden_frames_at_start: Rc<RefCell<Option<u64>>>,
    rss_start: u64,
    measure: usize,
}

impl StreamView {
    fn new(
        engines: Vec<Arc<EngineHandle>>,
        stop: Arc<AtomicBool>,
        rss_start: u64,
        measure: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            engines,
            stop,
            focus: cx.focus_handle(),
            state: TabRenderState::new(),
            metrics: Rc::new(RefCell::new(None)),
            paint_samples: Rc::new(RefCell::new(Vec::new())),
            build_samples: Rc::new(RefCell::new(Vec::new())),
            frame_idx: Rc::new(RefCell::new(0)),
            flipped: Rc::new(RefCell::new(false)),
            hidden_frames_at_start: Rc::new(RefCell::new(None)),
            rss_start,
            measure,
        }
    }
}

impl Render for StreamView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        // Resolve Menlo metrics once (first frame) via a manual canvas, then
        // paint the visible tab's latest frame and sample p95s.
        if self.metrics.borrow().is_none() {
            let metrics_cell = Rc::clone(&self.metrics);
            return canvas(
                |_, _, _| {},
                move |_bounds, _prepaint, window, _cx| {
                    *metrics_cell.borrow_mut() = Some(CellMetrics::resolve_menlo(window));
                },
            )
            .size_full()
            .into_any_element();
        }

        let idx = *self.frame_idx.borrow();

        // Record hidden tab 1's build count at the first measured frame so we can
        // assert it stayed flat while hidden.
        if idx == WARMUP {
            *self.hidden_frames_at_start.borrow_mut() =
                Some(self.engines[1].inspect().frames_built);
        }

        // Flip tab 1 visible at the two-thirds mark to exercise the show path.
        let flip_at = WARMUP + (self.measure * 2) / 3;
        if idx == flip_at && !*self.flipped.borrow() {
            // Assert the hidden tab suppressed builds up to now.
            if let Some(start) = *self.hidden_frames_at_start.borrow() {
                let now = self.engines[1].inspect().frames_built;
                if now != start {
                    fail(&format!(
                        "hidden tab 1 built frames while hidden: {start} -> {now}"
                    ));
                }
            }
            let _ = self.engines[1].set_visible(true);
            *self.flipped.borrow_mut() = true;
        }

        // Sample the max build time across engines this frame.
        let max_build_us = self
            .engines
            .iter()
            .map(|e| e.inspect().last_build_micros)
            .max()
            .unwrap_or(0);
        if idx >= WARMUP {
            self.build_samples
                .borrow_mut()
                .push(Duration::from_micros(max_build_us));
        }

        // Load the visible tab-0 frame for painting.
        if let Some(frame) = self.engines[0].latest_frame() {
            self.state.set_frame(frame);
        }

        let metrics_cell = Rc::clone(&self.metrics);
        let paint_cell = Rc::clone(&self.paint_samples);
        let frame_idx = Rc::clone(&self.frame_idx);
        let build_samples = Rc::clone(&self.build_samples);
        let stop = Arc::clone(&self.stop);
        let measure = self.measure;
        let rss_start = self.rss_start;

        // Paint via a timed canvas (PerCell path), then advance/finish.
        // `TabRenderState::latest_frame()` is already public via render_test_api —
        // no production-surface touch needed.
        let frame_for_paint = self.state.latest_frame();
        canvas(
            |_, _, _| {},
            move |bounds, _prepaint, window, cx| {
                let m = metrics_cell.borrow();
                let Some(metrics) = m.as_ref() else { return };
                let Some(frame) = frame_for_paint.as_ref() else {
                    // No frame yet; still advance so we don't deadlock.
                    *frame_idx.borrow_mut() += 1;
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
                let i = *frame_idx.borrow();
                if i >= WARMUP {
                    paint_cell.borrow_mut().push(dt);
                }
                *frame_idx.borrow_mut() = i + 1;

                if i >= WARMUP + measure {
                    let paints = paint_cell.borrow();
                    let builds = build_samples.borrow();
                    let paint_p95 = percentile_ms(&paints, 0.95);
                    let build_p95 = percentile_ms(&builds, 0.95);
                    let rss_end = rss_bytes();
                    let d_rss = rss_end as i64 - rss_start as i64;
                    eprintln!(
                        "STREAM paint_p95_ms={paint_p95:.3} (budget {PAINT_P95_MS}) \
                         build_p95_ms={build_p95:.3} (budget {BUILD_P95_MS}) \
                         delta_rss_bytes={d_rss}"
                    );
                    stop.store(true, Ordering::Relaxed);
                    if paint_p95 > PAINT_P95_MS {
                        fail(&format!(
                            "paint p95 {paint_p95:.3}ms > budget {PAINT_P95_MS}ms"
                        ));
                    }
                    if build_p95 > BUILD_P95_MS {
                        fail(&format!(
                            "build p95 {build_p95:.3}ms > budget {BUILD_P95_MS}ms"
                        ));
                    }
                    println!("stream_perf_realwindow: all budgets OK");
                    std::process::exit(0);
                }
            },
        )
        .size_full()
        .into_any_element()
    }
}
