//! Real-window staged-finalize probe — must run on the main thread (`cargo run -p lens-ui --bin focused_finalize_probe`).
//! `Application::new().run()`; not invokable from `#[gpui::test]` worker threads.

use std::cell::RefCell;
use std::process;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{
    App, Application, AsyncWindowContext, Context, Entity, EntityId, ListAlignment, ListOffset,
    ListState, Render, Styled, WeakEntity, Window, div, list, prelude::*, px,
};
use lens_core::domain::ids::{AccId, ItemId, ResponseId};
use lens_core::domain::item::{BlockContext, ContentBlock, ItemKind, StreamScratch};
use lens_core::domain::scalars::Role;
use lens_core::persist::{RangeRead, ReadRange};
use lens_core::reduce::{RetireDisposition, StreamUpdate};
use lens_ui::fleet::store::ReconcileEpoch;
use lens_ui::focused::{FocusedTranscript, ReaderWorkerHandle, RowId};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AnchorSnapshot {
    top_item_index: usize,
    sub_offset: gpui::Pixels,
}

impl From<ListOffset> for AnchorSnapshot {
    fn from(offset: ListOffset) -> Self {
        Self {
            top_item_index: offset.item_ix,
            sub_offset: offset.offset_in_item,
        }
    }
}

#[derive(Default)]
struct ProbeState {
    samples: usize,
    failures: Vec<String>,
    target_entity: Option<EntityId>,
    min_row_count: usize,
}

struct HarnessView {
    replica: Entity<FocusedTranscript>,
    list_state: ListState,
    probe: Rc<RefCell<ProbeState>>,
    acc_id: AccId,
    item_id: ItemId,
    finalized: bool,
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

fn assistant_item(id: &str, text: &str, resp: &str) -> lens_core::domain::item::Item {
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

async fn drive_finalize_probe(
    weak: WeakEntity<HarnessView>,
    mut wcx: AsyncWindowContext,
    acc_id: AccId,
    item_id: ItemId,
    exit_ok: Rc<RefCell<bool>>,
) {
    wait_frames(&mut wcx, 3).await;

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.fold_detailed(
                StreamUpdate::ActiveResponseChanged(Some(ResponseId::new("resp_a"))),
                cx,
            );
        });
        view.replica.update(cx, |r, cx| {
            r.fold_detailed(
                StreamUpdate::ScratchChanged(std::sync::Arc::new(StreamScratch {
                    open_message: Some(lens_core::domain::item::MessageAcc {
                        acc_id: acc_id.clone(),
                        message_id: None,
                        text: "streaming text".into(),
                        block_index: 0,
                    }),
                    ..Default::default()
                })),
                cx,
            );
        });
        let count = view.replica.read(cx).rows().len();
        view.list_state.reset(count);
        cx.notify();
    });
    wait_frames(&mut wcx, 3).await;

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.fold_detailed(
                StreamUpdate::Retired {
                    acc_id: acc_id.clone(),
                    disposition: RetireDisposition::Finalizing {
                        item_id: item_id.clone(),
                    },
                },
                cx,
            );
            r.fold_detailed(
                StreamUpdate::ScratchChanged(std::sync::Arc::new(StreamScratch::default())),
                cx,
            );
        });
        let count = view.replica.read(cx).rows().len();
        view.list_state
            .splice(0..view.list_state.item_count(), count);
        cx.notify();
    });
    wait_frames(&mut wcx, 3).await;

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.finalized = true;
        view.replica.update(cx, |r, cx| {
            r.apply_read(
                1,
                ReadRange::Delta {
                    after: -1,
                    through: 0,
                },
                RangeRead {
                    rows: vec![(0, assistant_item("msg_local_0", "hi", "resp_a"))],
                    skipped: vec![],
                    watermark: Some(0),
                },
                cx,
            );
        });
        let count = view.replica.read(cx).rows().len();
        view.list_state
            .splice(0..view.list_state.item_count(), count);
        cx.notify();
    });
    wait_frames(&mut wcx, 3).await;

    let ok = weak
        .update_in(&mut wcx, |view, _, _| {
            let p = view.probe.borrow();
            p.failures.is_empty()
                && p.samples >= 4
                && p.target_entity.is_some()
                && p.min_row_count > 0
        })
        .unwrap_or(false);

    if !ok {
        let _ = weak.update_in(&mut wcx, |view, _, _| {
            let p = view.probe.borrow();
            eprintln!("REAL-WINDOW FAILURES: {:?}", p.failures);
            eprintln!("samples={} min_rows={}", p.samples, p.min_row_count);
        });
    }
    *exit_ok.borrow_mut() = ok;
    let _ = wcx.update(|_, cx| cx.quit());
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.spawned {
            self.spawned = true;
            let acc = self.acc_id.clone();
            let item = self.item_id.clone();
            let exit_ok = Rc::clone(&self.exit_ok);
            cx.spawn_in(window, move |weak, wcx: &mut AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    drive_finalize_probe(weak, wcx, acc, item, exit_ok).await;
                }
            })
            .detach();
        }

        let count = self.replica.read(cx).rows().len();
        let tail_id = RowId::StreamTail(self.acc_id.clone());
        let sibling_id = RowId::Sibling(self.item_id.clone());
        let row_ix = if self.finalized {
            (0..count)
                .find(|&ix| {
                    self.replica.read(cx).rows().id_at(ix) == Some(&sibling_id)
                        || self.replica.read(cx).rows().id_at(ix) == Some(&tail_id)
                })
                .unwrap_or(count.saturating_sub(1))
        } else {
            (0..count)
                .find(|&ix| self.replica.read(cx).rows().id_at(ix) == Some(&tail_id))
                .unwrap_or(count.saturating_sub(1))
        };

        let entity_id = self.replica.read(cx).rows().entity_id_at(row_ix, cx);
        let text = self
            .replica
            .read(cx)
            .rows()
            .id_at(row_ix)
            .and_then(|id| self.replica.read(cx).rows().entity(id))
            .map(|e| e.read(cx).presentation.text.clone())
            .unwrap_or_default();
        let anchor = AnchorSnapshot::from(self.list_state.logical_scroll_top());

        {
            let mut p = self.probe.borrow_mut();
            if let Some(target) = p.target_entity {
                if entity_id != Some(target) {
                    p.failures
                        .push(format!("entity id changed {:?} -> {:?}", target, entity_id));
                }
            } else if let Some(eid) = entity_id {
                p.target_entity = Some(eid);
            }
            let min_before = p.min_row_count;
            if count < min_before {
                p.failures
                    .push(format!("row count dipped {min_before} -> {count}"));
            }
            p.min_row_count = p.min_row_count.max(count);
            if count > 0 && (anchor.top_item_index != count || anchor.sub_offset != px(0.)) {
                p.failures.push(format!(
                    "bottom pin lost: anchor=({}, {:?}) count={count}",
                    anchor.top_item_index, anchor.sub_offset
                ));
            }
            if !text.is_empty() && text != "streaming text" && text != "hi" {
                p.failures.push(format!("unexpected content: {text:?}"));
            }
            p.samples += 1;
        }

        let list_state = self.list_state.clone();
        let replica = self.replica.clone();
        div().size_full().child(
            list(list_state, move |ix, _window, cx| {
                let replica = replica.clone();
                let Some(id) = replica.read(cx).rows().id_at(ix) else {
                    return div().into_any_element();
                };
                let Some(entity) = replica.read(cx).rows().entity(id) else {
                    return div().into_any_element();
                };
                let text = entity.read(cx).presentation.text.clone();
                div().id(ix).child(text).into_any_element()
            })
            .size_full(),
        )
    }
}

fn main() {
    let exit_ok = Rc::new(RefCell::new(false));
    let exit_for_run = Rc::clone(&exit_ok);
    let probe = Rc::new(RefCell::new(ProbeState::default()));

    let acc_id = AccId::new("acc_rw");
    let item_id = ItemId::new("msg_local_0");

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);

        let (reader, _rx) = ReaderWorkerHandle::new_test();
        let session_id = lens_core::domain::ids::SessionId::new("sess_rw");
        let replica = cx.new(|cx| {
            FocusedTranscript::new_test_no_baseline(
                reader,
                session_id,
                ReconcileEpoch::default(),
                1,
                cx,
            )
        });

        let list_state = ListState::new(0, ListAlignment::Bottom, px(200.));
        cx.open_window(gpui::WindowOptions::default(), move |_window, cx| {
            cx.new(|_| HarnessView {
                replica: replica.clone(),
                list_state: list_state.clone(),
                probe: Rc::clone(&probe),
                acc_id: acc_id.clone(),
                item_id: item_id.clone(),
                finalized: false,
                spawned: false,
                exit_ok: Rc::clone(&exit_for_run),
            })
        })
        .expect("open window");
        cx.activate(true);
    });

    process::exit(if *exit_ok.borrow() { 0 } else { 1 });
}
