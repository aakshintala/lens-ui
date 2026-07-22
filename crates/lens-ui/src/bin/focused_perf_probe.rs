//! Real-window O(visible) perf probe — list render calls ≪ resident rows; frame time reported.
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

const LARGE_RESIDENT: usize = 2000;
/// Generous cap: visible rows + overdraw, independent of resident count.
const RENDER_CALL_CAP: usize = 80;
const TARGET_FRAME_MS: f64 = 8.3;
const REGRESSION_FRAME_MS: f64 = 11.1;

#[derive(Default)]
struct FrameTimer {
    samples_us: Vec<u128>,
}

impl FrameTimer {
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

#[derive(Default)]
struct ProbeState {
    failures: Vec<String>,
    frame_timer: FrameTimer,
    last_frame_start: Option<Instant>,
    max_render_calls: usize,
    resident_count: usize,
    delta_frames: usize,
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

fn seed_rows(replica: &Entity<FocusedTranscript>, count: usize, cx: &mut App) {
    let rows: Vec<_> = (0..count)
        .map(|i| {
            (
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

fn drive_live_delta(replica: &Entity<FocusedTranscript>, tag: &str, cx: &mut App) {
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
}

async fn drive_perf_probe(
    weak: WeakEntity<HarnessView>,
    mut wcx: AsyncWindowContext,
    resident: usize,
    exit_ok: Rc<RefCell<bool>>,
) {
    wait_frames(&mut wcx, 3).await;

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        seed_rows(&view.replica, resident, cx);
        view.probe.borrow_mut().resident_count = resident;
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    for i in 0..4 {
        let tag = format!("delta_{resident}_{i}");
        let _ = weak.update_in(&mut wcx, |view, _, cx| {
            view.frame_render_calls.set(0);
            drive_live_delta(&view.replica, &tag, cx);
            cx.notify();
        });
        wait_frames(&mut wcx, 3).await;

        let _ = weak.update_in(&mut wcx, |view, _, _| {
            let calls = view.frame_render_calls.get();
            let mut p = view.probe.borrow_mut();
            p.delta_frames += 1;
            p.max_render_calls = p.max_render_calls.max(calls);
            if calls >= RENDER_CALL_CAP {
                p.failures.push(format!(
                    "render_calls {calls} >= cap {RENDER_CALL_CAP} with resident={resident}"
                ));
            }
            if calls >= resident {
                p.failures.push(format!(
                    "render_calls {calls} scaled with resident count {resident}"
                ));
            }
        });
    }

    let ok = weak
        .update_in(&mut wcx, |view, _, _| {
            let p = view.probe.borrow();
            let mean_ms = p.frame_timer.mean_ms();
            eprintln!(
                "PERF PROBE resident={} max_render_calls={} cap={RENDER_CALL_CAP} \
                 delta_frames={} mean_frame_ms={mean_ms:.2} (target {TARGET_FRAME_MS} / regression {REGRESSION_FRAME_MS})",
                p.resident_count,
                p.max_render_calls,
                p.delta_frames,
            );
            if mean_ms > REGRESSION_FRAME_MS {
                eprintln!(
                    "PERF PROBE NOTE: mean frame time {mean_ms:.2}ms exceeds {REGRESSION_FRAME_MS}ms regression line (soft check)"
                );
            } else if mean_ms > TARGET_FRAME_MS {
                eprintln!(
                    "PERF PROBE NOTE: mean frame time {mean_ms:.2}ms above {TARGET_FRAME_MS}ms target (soft check)"
                );
            }
            if p.failures.is_empty() {
                eprintln!("PERF PROBE: O(visible) render-call gate passed");
            } else {
                eprintln!("PERF PROBE FAILURES: {:?}", p.failures);
            }
            p.failures.is_empty() && p.delta_frames >= 4
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
            let resident = LARGE_RESIDENT;
            cx.spawn_in(window, move |weak, wcx: &mut AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    drive_perf_probe(weak, wcx, resident, exit_ok).await;
                }
            })
            .detach();
        }

        {
            let now = Instant::now();
            let mut p = self.probe.borrow_mut();
            if let Some(start) = p.last_frame_start.take() {
                p.frame_timer.record(now.duration_since(start));
            }
            p.last_frame_start = Some(now);
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
