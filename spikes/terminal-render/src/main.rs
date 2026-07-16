//! Spike A — GPUI terminal-grid paint viability harness.
//!
//! Measurement sweep: fixture × grid × strategy, release build, ≥500 frames
//! after 60-frame warmup. Prints p50/p95/p99/max for paint + snapshot.

mod fixtures;
mod measure;
mod paint;

use std::cell::RefCell;
use std::env;
use std::io::Write;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    Application, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
    canvas, point, prelude::*, px, size,
};
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::{Terminal, TerminalOptions};

use measure::{FixtureKind, RunConfig, RunResult, SampleSet};
use paint::{
    CellMetrics, RowShapeCache, Strategy, TextPlacement, paint_grid, per_row_alignment_ok,
};

struct VtEngine {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    cols: u16,
    rows_n: u16,
    frame_n: u64,
    seeded: bool,
    cache: RowShapeCache,
}

impl VtEngine {
    fn new(cols: u16, rows_n: u16) -> Self {
        let terminal = Terminal::new(TerminalOptions {
            cols,
            rows: rows_n,
            max_scrollback: 1000,
        })
        .expect("terminal");
        Self {
            terminal,
            render_state: RenderState::new().expect("render state"),
            rows: RowIterator::new().expect("row iterator"),
            cells: CellIterator::new().expect("cell iterator"),
            cols,
            rows_n,
            frame_n: 0,
            seeded: false,
            cache: RowShapeCache::default(),
        }
    }
}

/// Shared across render → later paint closure (paint runs after render returns).
struct FrameAccum {
    paint: SampleSet,
    snapshot: SampleSet,
    input_to_first_paint_ms: Option<f64>,
    done: bool,
}

struct GridView {
    vt: Rc<RefCell<VtEngine>>,
    metrics: Option<CellMetrics>,
    config: RunConfig,
    accum: Rc<RefCell<FrameAccum>>,
    alignment_ok: bool,
    finished: bool,
    result_slot: Rc<RefCell<Option<RunResult>>>,
}

impl GridView {
    fn new(
        config: RunConfig,
        result_slot: Rc<RefCell<Option<RunResult>>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            vt: Rc::new(RefCell::new(VtEngine::new(config.cols, config.rows))),
            metrics: None,
            config,
            accum: Rc::new(RefCell::new(FrameAccum {
                paint: SampleSet::new(),
                snapshot: SampleSet::new(),
                input_to_first_paint_ms: None,
                done: false,
            })),
            alignment_ok: true,
            finished: false,
            result_slot,
        }
    }

    fn finish(&mut self, cx: &mut gpui::App) {
        if self.finished {
            return;
        }
        self.finished = true;
        let vt = self.vt.borrow();
        let accum = self.accum.borrow();
        let result = RunResult {
            label: format!(
                "{} {}×{} {:?}",
                self.config.fixture.as_str(),
                self.config.cols,
                self.config.rows,
                self.config.strategy
            ),
            paint: accum
                .paint
                .percentiles_after_warmup(self.config.warmup),
            snapshot: accum
                .snapshot
                .percentiles_after_warmup(self.config.warmup),
            input_to_first_paint_ms: accum.input_to_first_paint_ms,
            cache_hits: vt.cache.hits(),
            cache_misses: vt.cache.misses(),
            placement: format!("{:?}", self.config.placement),
            alignment_ok: self.alignment_ok,
        };
        result.print_block();
        // Write before quit — `Application::run` + `cx.quit()` exits the process
        // without returning to `main` on this gpui/macOS path.
        persist_result_line(&result);
        *self.result_slot.borrow_mut() = Some(result);
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        cx.quit();
    }
}

fn results_path() -> String {
    env::var("TERMINAL_RENDER_RESULTS")
        .unwrap_or_else(|_| "spikes/terminal-render/results.tsv".into())
}

fn persist_result_line(result: &RunResult) {
    let Some(p) = result.paint else {
        return;
    };
    let line = format!(
        "RESULT\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\n",
        result.label,
        result.placement,
        p.p50,
        p.p95,
        p.p99,
        result.snapshot.map(|s| s.p95).unwrap_or(f64::NAN),
        result.input_to_first_paint_ms.unwrap_or(f64::NAN),
        result.cache_hits,
        result.cache_misses,
    );
    eprint!("{line}");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(results_path())
    {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Paint from the previous frame may have filled the accumulator.
        {
            let accum = self.accum.borrow();
            if !self.finished && accum.paint.len() >= self.config.total_frames {
                drop(accum);
                self.finish(cx);
                return canvas(|_, _, _| {}, |_, _, _, _| {}).size_full();
            }
        }

        if self.finished {
            return canvas(|_, _, _| {}, |_, _, _, _| {}).size_full();
        }

        window.request_animation_frame();

        if self.metrics.is_none() {
            let m = CellMetrics::resolve(window);
            self.alignment_ok = per_row_alignment_ok(window, &m);
            eprintln!(
                "alignment probe: {} (placement={:?})",
                if self.alignment_ok {
                    "OK"
                } else {
                    "MISALIGNED"
                },
                self.config.placement
            );
            self.metrics = Some(m);
        }

        let vt = Rc::clone(&self.vt);
        let accum = Rc::clone(&self.accum);
        let metrics = self.metrics.clone().expect("metrics");
        let fixture = self.config.fixture;
        let strategy = self.config.strategy;
        let placement = self.config.placement;
        let total_frames = self.config.total_frames;

        canvas(
            |_bounds, _window, _cx| {},
            move |bounds, _prepaint, window, cx| {
                {
                    let accum = accum.borrow();
                    if accum.done || accum.paint.len() >= total_frames {
                        return;
                    }
                }

                let mut vt = vt.borrow_mut();
                let feed_start = Instant::now();
                let measuring_first = vt.frame_n == 0;

                match fixture {
                    FixtureKind::FullRedraw => {
                        let bytes = fixtures::full_redraw(vt.cols, vt.rows_n);
                        vt.terminal.vt_write(&bytes);
                    }
                    FixtureKind::PartialUpdate => {
                        if !vt.seeded {
                            let bytes = fixtures::full_redraw(vt.cols, vt.rows_n);
                            vt.terminal.vt_write(&bytes);
                            vt.seeded = true;
                        } else {
                            let bytes =
                                fixtures::partial_update(vt.cols, vt.rows_n, vt.frame_n);
                            vt.terminal.vt_write(&bytes);
                        }
                    }
                    FixtureKind::WideAndSgr => {
                        if !vt.seeded {
                            let bytes = fixtures::wide_and_sgr(vt.cols, vt.rows_n);
                            vt.terminal.vt_write(&bytes);
                            vt.seeded = true;
                        }
                    }
                }

                vt.frame_n += 1;

                let VtEngine {
                    terminal,
                    render_state,
                    rows,
                    cells,
                    cache,
                    ..
                } = &mut *vt;

                let snap_t0 = Instant::now();
                let snapshot = render_state.update(terminal).expect("update");
                let snap_dt = snap_t0.elapsed();

                let origin = point(bounds.origin.x + px(4.0), bounds.origin.y + px(4.0));
                let cache_ref = match strategy {
                    Strategy::S2 => Some(cache),
                    Strategy::S1 => None,
                };

                let paint_t0 = Instant::now();
                let _ = paint_grid(
                    &snapshot,
                    rows,
                    cells,
                    origin,
                    &metrics,
                    strategy,
                    placement,
                    cache_ref,
                    window,
                    cx,
                )
                .expect("paint_grid");
                let paint_dt = paint_t0.elapsed();

                let mut accum = accum.borrow_mut();
                accum.snapshot.push(snap_dt);
                accum.paint.push(paint_dt);
                if measuring_first && accum.input_to_first_paint_ms.is_none() {
                    accum.input_to_first_paint_ms =
                        Some(feed_start.elapsed().as_secs_f64() * 1000.0);
                }
                if accum.paint.len() >= total_frames {
                    accum.done = true;
                }
            },
        )
        .size_full()
    }
}

fn parse_args() -> Vec<RunConfig> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut fixture: Option<FixtureKind> = None;
    let mut cols: Option<u16> = None;
    let mut rows: Option<u16> = None;
    let mut strategy = Strategy::S1;
    let mut placement = TextPlacement::PerRow;
    let mut total_frames = 560usize;
    let mut warmup = 60usize;
    let mut sweep: Option<String> = None;

    for a in &args {
        if let Some(v) = a.strip_prefix("--fixture=") {
            fixture = FixtureKind::parse(v);
        } else if let Some(v) = a.strip_prefix("--cols=") {
            cols = v.parse().ok();
        } else if let Some(v) = a.strip_prefix("--rows=") {
            rows = v.parse().ok();
        } else if let Some(v) = a.strip_prefix("--strategy=") {
            strategy = match v {
                "s1" | "S1" => Strategy::S1,
                "s2" | "S2" => Strategy::S2,
                _ => strategy,
            };
        } else if let Some(v) = a.strip_prefix("--placement=") {
            placement = match v {
                "per-row" | "row" => TextPlacement::PerRow,
                "per-cell" | "cell" => TextPlacement::PerCell,
                _ => placement,
            };
        } else if let Some(v) = a.strip_prefix("--frames=") {
            total_frames = v.parse().unwrap_or(total_frames);
        } else if let Some(v) = a.strip_prefix("--warmup=") {
            warmup = v.parse().unwrap_or(warmup);
        } else if let Some(v) = a.strip_prefix("--sweep=") {
            sweep = Some(v.to_string());
        } else if a == "--sweep" {
            sweep = Some("s1".into());
        }
    }

    if let Some(s) = sweep {
        let strat = match s.as_str() {
            "s2" | "S2" => Strategy::S2,
            _ => Strategy::S1,
        };
        return sweep_configs(strat, total_frames, warmup);
    }

    vec![RunConfig {
        fixture: fixture.unwrap_or(FixtureKind::FullRedraw),
        cols: cols.unwrap_or(80),
        rows: rows.unwrap_or(24),
        strategy,
        placement,
        total_frames,
        warmup,
    }]
}

fn sweep_configs(strategy: Strategy, total_frames: usize, warmup: usize) -> Vec<RunConfig> {
    let grids: &[(u16, u16)] = &[(80, 24), (200, 50), (400, 100)];
    let fixtures = [
        FixtureKind::FullRedraw,
        FixtureKind::PartialUpdate,
        FixtureKind::WideAndSgr,
    ];
    let mut out = Vec::new();
    for &(cols, rows) in grids {
        for &fixture in &fixtures {
            let placement = match fixture {
                FixtureKind::WideAndSgr => TextPlacement::PerCell,
                _ => TextPlacement::PerRow,
            };
            out.push(RunConfig {
                fixture,
                cols,
                rows,
                strategy,
                placement,
                total_frames,
                warmup,
            });
        }
    }
    out
}

fn config_to_args(cfg: &RunConfig) -> Vec<String> {
    vec![
        format!("--fixture={}", cfg.fixture.as_str()),
        format!("--cols={}", cfg.cols),
        format!("--rows={}", cfg.rows),
        format!(
            "--strategy={}",
            match cfg.strategy {
                Strategy::S1 => "s1",
                Strategy::S2 => "s2",
            }
        ),
        format!(
            "--placement={}",
            match cfg.placement {
                TextPlacement::PerRow => "per-row",
                TextPlacement::PerCell => "per-cell",
            }
        ),
        format!("--frames={}", cfg.total_frames),
        format!("--warmup={}", cfg.warmup),
    ]
}

fn run_one(config: RunConfig) {
    let slot = Rc::new(RefCell::new(None));
    let cfg = config.clone();
    eprintln!(
        "RUN {} {}×{} {:?} placement={:?} frames={}",
        cfg.fixture.as_str(),
        cfg.cols,
        cfg.rows,
        cfg.strategy,
        cfg.placement,
        cfg.total_frames
    );
    Application::new().run(move |cx| {
        let slot = Rc::clone(&slot);
        let cfg = config.clone();
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(
                        format!(
                            "terminal-render {} {}×{}",
                            cfg.fixture.as_str(),
                            cfg.cols,
                            cfg.rows
                        )
                        .into(),
                    ),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(1100.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| GridView::new(cfg, slot, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
    // Unreachable on macOS gpui quit path; kept for non-quit platforms.
    let _ = slot;
}

fn main() {
    let configs = parse_args();
    let path = results_path();

    // Parent of a multi-run sweep: spawn one process per config (gpui quit exits).
    if configs.len() > 1 && env::var_os("TERMINAL_RENDER_CHILD").is_none() {
        let _ = std::fs::remove_file(&path);
        println!("# terminal-render spike measurements");
        println!("# host: {}  build: release", std::env::consts::ARCH);
        println!("# spawning {} child runs → {path}", configs.len());
        println!();
        let exe = env::current_exe().expect("current_exe");
        for cfg in &configs {
            let args = config_to_args(cfg);
            let status = Command::new(&exe)
                .args(&args)
                .env("TERMINAL_RENDER_CHILD", "1")
                .env("TERMINAL_RENDER_RESULTS", &path)
                .status()
                .expect("spawn child");
            if !status.success() {
                eprintln!("child failed for {:?}: {status}", cfg.fixture);
            }
        }
        println!("## Aggregated RESULT lines");
        if let Ok(body) = std::fs::read_to_string(&path) {
            print!("{body}");
        } else {
            println!("(no results file)");
        }
        return;
    }

    println!("# terminal-render spike measurements");
    println!("# host: {}  build: release", std::env::consts::ARCH);
    println!();
    // Single config (child or direct invocation).
    run_one(configs.into_iter().next().expect("config"));
}
