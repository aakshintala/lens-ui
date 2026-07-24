//! Real-window O(visible) perf probe — small vs large resident render-call parity.
//! `Application::new().run()`; not invokable from `#[gpui::test]` worker threads.

use std::cell::{Cell, RefCell};
use std::process;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::{
    App, Application, AsyncWindowContext, Context, Entity, Render, Styled, WeakEntity, Window, div,
    list, prelude::*, px,
};
use lens_core::domain::ids::{AccId, ItemId, ResponseId};
use lens_core::domain::item::{BlockContext, ContentBlock, ItemKind, MessageAcc, StreamScratch};
use lens_core::domain::scalars::Role;
use lens_core::persist::{RangeRead, ReadRange};
use lens_core::reduce::StreamUpdate;
use lens_ui::fleet::store::ReconcileEpoch;
use lens_ui::focused::{FocusedTranscript, ReaderWorkerHandle, RowKind};

const SMALL_RESIDENT: usize = 30;
const LARGE_RESIDENT: usize = 2000;
/// Generous cap: visible rows + overdraw, independent of resident count.
const RENDER_CALL_CAP: usize = 80;
/// Small vs large per-frame render calls must agree within this band.
const RENDER_CALL_TOLERANCE: usize = 8;
const TARGET_COMPUTE_MS: f64 = 8.3;
const REGRESSION_COMPUTE_MS: f64 = 11.1;
const DELTA_FRAMES_PER_RUN: usize = 4;

#[derive(Default)]
struct DurationSamples {
    samples_us: Vec<u128>,
}

impl DurationSamples {
    fn record(&mut self, d: Duration) {
        self.samples_us.push(d.as_micros());
    }

    fn mean_ms(&self) -> f64 {
        if self.samples_us.is_empty() {
            0.0
        } else {
            let sum: u128 = self.samples_us.iter().sum();
            sum as f64 / self.samples_us.len() as f64 / 1000.0
        }
    }
}

#[derive(Clone, Default)]
struct ResidentRunResult {
    resident: usize,
    max_render_calls: usize,
    mean_compute_ms: f64,
    delta_frames: usize,
}

#[derive(Default)]
struct ProbeState {
    failures: Vec<String>,
    compute_samples: DurationSamples,
    small: Option<ResidentRunResult>,
    large: Option<ResidentRunResult>,
}

struct HarnessView {
    replica: Entity<FocusedTranscript>,
    render_calls: Rc<Cell<usize>>,
    frame_render_calls: Rc<Cell<usize>>,
    probe: Rc<RefCell<ProbeState>>,
    spawned: bool,
    exit_ok: Rc<RefCell<bool>>,
}

async fn wait_frames(wcx: &mut AsyncWindowContext, n: usize) {
    for _ in 0..n {
        let (tx, rx) = mpsc::sync_channel(1);
        wcx.on_next_frame(move |_, _| {
            let _ = tx.send(());
        });
        let _ = wcx.update(|window, _| window.refresh());
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            wcx.background_executor()
                .timer(Duration::from_millis(1))
                .await;
        }
    }
}

fn message_item(id: &str, text: &str, resp: &str) -> lens_core::domain::item::Item {
    lens_core::domain::item::Item {
        id: ItemId::new(id),
        seq: None,
        ctx: BlockContext {
            agent: None,
            depth: 0,
            response_id: Some(ResponseId::new(resp)),
        },
        created_at: 1,
        kind: ItemKind::Message {
            role: Role::Assistant,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some(text.into()),
                data: serde_json::Value::Null,
            }],
        },
    }
}

fn row_with_len(
    ord: i64,
    item: lens_core::domain::item::Item,
) -> (i64, usize, lens_core::domain::item::Item) {
    (ord, 4, item)
}

fn seed_rows(replica: &Entity<FocusedTranscript>, count: usize, cx: &mut App) {
    let rows: Vec<_> = (0..count)
        .map(|i| {
            row_with_len(
                i as i64,
                message_item(
                    &format!("m{i}"),
                    &format!("row {i}"),
                    &format!("resp_{}", i % 8),
                ),
            )
        })
        .collect();
    replica.update(cx, |r, cx| {
        r.apply_read(
            1,
            ReadRange::All,
            RangeRead {
                rows,
                skipped: vec![],
                watermark: Some((count as i64).saturating_sub(1)),
            },
            cx,
        );
        r.fold_detailed(
            StreamUpdate::ActiveResponseChanged(Some(ResponseId::new("resp_live"))),
            cx,
        );
    });
}

fn drive_live_delta(replica: &Entity<FocusedTranscript>, tag: &str, cx: &mut App) -> Duration {
    let start = Instant::now();
    replica.update(cx, |r, cx| {
        r.fold_detailed(
            StreamUpdate::ScratchChanged(std::sync::Arc::new(StreamScratch {
                open_message: Some(MessageAcc {
                    acc_id: AccId::new(tag),
                    message_id: None,
                    text: format!("streaming {tag}"),
                    block_index: 0,
                }),
                ..Default::default()
            })),
            cx,
        );
    });
    start.elapsed()
}

async fn run_resident_probe(
    weak: &WeakEntity<HarnessView>,
    wcx: &mut AsyncWindowContext,
    resident: usize,
) -> (ResidentRunResult, Vec<String>) {
    let _ = weak.update_in(wcx, |view, _, cx| {
        seed_rows(&view.replica, resident, cx);
        cx.notify();
    });
    wait_frames(wcx, 4).await;

    let mut max_render_calls = 0usize;
    let mut delta_frames = 0usize;
    let mut compute_samples = DurationSamples::default();
    let mut failures = Vec::new();

    for i in 0..DELTA_FRAMES_PER_RUN {
        let tag = format!("delta_{resident}_{i}");
        let compute = weak
            .update_in(wcx, |view, _, cx| {
                view.frame_render_calls.set(0);
                let elapsed = drive_live_delta(&view.replica, &tag, cx);
                cx.notify();
                elapsed
            })
            .unwrap_or(Duration::ZERO);
        compute_samples.record(compute);
        let _ = weak.update_in(wcx, |view, _, _| {
            view.probe.borrow_mut().compute_samples.record(compute);
        });
        wait_frames(wcx, 3).await;

        let calls = weak
            .update_in(wcx, |view, _, _| view.frame_render_calls.get())
            .unwrap_or(0);
        max_render_calls = max_render_calls.max(calls);
        delta_frames += 1;

        if calls == 0 {
            failures.push(format!(
                "resident={resident} delta {i}: zero list render calls (vacuous pass)"
            ));
        }
        if calls >= RENDER_CALL_CAP {
            failures.push(format!(
                "resident={resident} delta {i}: render_calls {calls} >= cap {RENDER_CALL_CAP}"
            ));
        }
        // Only when resident dwarfs visible+overdraw: at SMALL_RESIDENT=30 a healthy
        // O(visible) window can paint 30–79 rows without scaling with resident count.
        if resident > RENDER_CALL_CAP && calls >= resident {
            failures.push(format!(
                "resident={resident} delta {i}: render_calls {calls} scaled with resident count"
            ));
        }
    }

    (
        ResidentRunResult {
            resident,
            max_render_calls,
            mean_compute_ms: compute_samples.mean_ms(),
            delta_frames,
        },
        failures,
    )
}

fn assert_render_parity(
    small: &ResidentRunResult,
    large: &ResidentRunResult,
    failures: &mut Vec<String>,
) {
    if small.max_render_calls == 0 || large.max_render_calls == 0 {
        failures.push(format!(
            "render parity requires >0 calls on both runs (small={}, large={})",
            small.max_render_calls, large.max_render_calls
        ));
        return;
    }
    let diff = small.max_render_calls.abs_diff(large.max_render_calls);
    if diff > RENDER_CALL_TOLERANCE {
        failures.push(format!(
            "render calls must not scale with resident: small={} large={} diff={diff} tolerance={RENDER_CALL_TOLERANCE}",
            small.max_render_calls,
            large.max_render_calls,
        ));
    }
}

async fn drive_perf_probe(
    weak: WeakEntity<HarnessView>,
    mut wcx: AsyncWindowContext,
    exit_ok: Rc<RefCell<bool>>,
) {
    wait_frames(&mut wcx, 3).await;

    let (small, small_failures) = run_resident_probe(&weak, &mut wcx, SMALL_RESIDENT).await;
    let (large, large_failures) = run_resident_probe(&weak, &mut wcx, LARGE_RESIDENT).await;

    let ok = weak
        .update_in(&mut wcx, |view, _, _| {
            let mut p = view.probe.borrow_mut();
            p.failures.extend(small_failures);
            p.failures.extend(large_failures);
            p.small = Some(small.clone());
            p.large = Some(large.clone());
            assert_render_parity(&small, &large, &mut p.failures);
            if small.delta_frames < DELTA_FRAMES_PER_RUN || large.delta_frames < DELTA_FRAMES_PER_RUN
            {
                p.failures.push(format!(
                    "insufficient delta frames (small={}, large={})",
                    small.delta_frames, large.delta_frames
                ));
            }

            let mean_compute_ms = p.compute_samples.mean_ms();
            eprintln!(
                "PERF PROBE small(resident={}) max_render_calls={} mean_compute_ms={:.2} | \
                 large(resident={}) max_render_calls={} | parity_tol={RENDER_CALL_TOLERANCE} cap={RENDER_CALL_CAP}",
                small.resident,
                small.max_render_calls,
                small.mean_compute_ms,
                large.resident,
                large.max_render_calls,
            );
            eprintln!(
                "PERF PROBE mean per-delta COMPUTE ms={mean_compute_ms:.2} \
                 (target {TARGET_COMPUTE_MS} / regression {REGRESSION_COMPUTE_MS})"
            );
            if mean_compute_ms > REGRESSION_COMPUTE_MS {
                eprintln!(
                    "PERF PROBE NOTE: mean compute {mean_compute_ms:.2}ms exceeds {REGRESSION_COMPUTE_MS}ms regression line (soft check)"
                );
            } else if mean_compute_ms > TARGET_COMPUTE_MS {
                eprintln!(
                    "PERF PROBE NOTE: mean compute {mean_compute_ms:.2}ms above {TARGET_COMPUTE_MS}ms target (soft check)"
                );
            }

            if p.failures.is_empty() {
                eprintln!("PERF PROBE: O(visible) render-call parity gate passed");
            } else {
                eprintln!("PERF PROBE FAILURES: {:?}", p.failures);
            }
            p.failures.is_empty()
        })
        .unwrap_or(false);

    *exit_ok.borrow_mut() = ok;
    process::exit(if ok { 0 } else { 1 });
}

fn kind_tag(kind: RowKind) -> &'static str {
    match kind {
        RowKind::SectionChip => "SectionChip",
        RowKind::SectionRail => "SectionRail",
        RowKind::WorkChild => "WorkChild",
        RowKind::Message => "Message",
        RowKind::UserMessage => "UserMessage",
        RowKind::ResourceEvent => "ResourceEvent",
        RowKind::StreamingReasoning => "StreamingReasoning",
        RowKind::StreamingMessage => "StreamingMessage",
        RowKind::ReconnectBreak => "ReconnectBreak",
        RowKind::LoadOlder => "LoadOlder",
    }
}

fn render_probe_row(pres: &lens_ui::focused::RowPresentation, ix: usize) -> gpui::AnyElement {
    div()
        .id(ix)
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .child(
            div()
                .text_xs()
                .text_color(gpui::rgb(0x888888))
                .child(kind_tag(pres.kind)),
        )
        .child(pres.text.clone())
        .when_some(pres.height_hint, |el, h| el.h(px(h)))
        .into_any_element()
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.spawned {
            self.spawned = true;
            let exit_ok = Rc::clone(&self.exit_ok);
            cx.spawn_in(window, move |weak, wcx: &mut AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    drive_perf_probe(weak, wcx, exit_ok).await;
                }
            })
            .detach();
        }

        self.frame_render_calls.set(0);
        self.render_calls.set(0);

        let replica = self.replica.clone();
        let render_calls = Rc::clone(&self.render_calls);
        let frame_render_calls = Rc::clone(&self.frame_render_calls);
        let list_state = replica.read(cx).list_state().clone();

        let list_el = list(list_state, move |ix, _window, app| {
            render_calls.set(render_calls.get() + 1);
            frame_render_calls.set(frame_render_calls.get() + 1);
            let replica = replica.clone();
            let Some(id) = replica.read(app).rows().id_at(ix) else {
                return div().into_any_element();
            };
            let Some(entity) = replica.read(app).rows().entity(id) else {
                return div().into_any_element();
            };
            let pres = entity.read(app).presentation.clone();
            render_probe_row(&pres, ix)
        })
        .size_full();

        div().size_full().child(list_el)
    }
}

fn main() {
    let exit_ok = Rc::new(RefCell::new(false));
    let exit_for_run = Rc::clone(&exit_ok);
    let probe = Rc::new(RefCell::new(ProbeState::default()));

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);

        let (reader, _rx) = ReaderWorkerHandle::new_test();
        let session_id = lens_core::domain::ids::SessionId::new("sess_perf");
        let replica = cx.new(|cx| {
            FocusedTranscript::new_test_no_baseline(
                reader,
                session_id,
                ReconcileEpoch::default(),
                1,
                cx,
            )
        });

        cx.open_window(gpui::WindowOptions::default(), move |_window, cx| {
            cx.new(|_| HarnessView {
                replica: replica.clone(),
                render_calls: Rc::new(Cell::new(0)),
                frame_render_calls: Rc::new(Cell::new(0)),
                probe: Rc::clone(&probe),
                spawned: false,
                exit_ok: Rc::clone(&exit_for_run),
            })
        })
        .expect("open window");
        cx.activate(true);
    });

    process::exit(if *exit_ok.borrow() { 0 } else { 1 });
}
