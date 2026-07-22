//! Real-window scroll-contract probe — four §16 contracts + paused-not-yanked.
//! `Application::new().run()`; not invokable from `#[gpui::test]` worker threads.

use std::cell::RefCell;
use std::process;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{
    App, Application, AsyncWindowContext, Context, Entity, Pixels, Render, Styled, WeakEntity,
    Window, div, prelude::*, px,
};
use lens_core::domain::ids::{AccId, ItemId, ResponseId};
use lens_core::domain::item::{BlockContext, ContentBlock, ItemKind, StreamScratch};
use lens_core::domain::scalars::Role;
use lens_core::persist::{RangeRead, ReadRange};
use lens_core::reduce::{RetireDisposition, StreamUpdate};
use lens_ui::fleet::store::ReconcileEpoch;
use lens_ui::focused::view::{FocusedTranscriptView, FollowMode};
use lens_ui::focused::{FocusedTranscript, ReaderWorkerHandle};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AnchorSnapshot {
    top_item_index: usize,
    sub_offset: Pixels,
}

impl From<gpui::ListOffset> for AnchorSnapshot {
    fn from(offset: gpui::ListOffset) -> Self {
        Self {
            top_item_index: offset.item_ix,
            sub_offset: offset.offset_in_item,
        }
    }
}

#[derive(Default)]
struct ProbeState {
    failures: Vec<String>,
    samples: usize,
    saw_initial_bottom: bool,
    saw_stick_while_following: bool,
    saw_paused_on_scroll: bool,
    saw_pill_while_paused: bool,
    saw_pill_n_three: bool,
    saw_resume_following: bool,
    finalize_anchor_stable: Option<bool>,
    paused_anchor_stable: Option<bool>,
}

struct HarnessView {
    replica: Entity<FocusedTranscript>,
    transcript: Entity<FocusedTranscriptView>,
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

fn at_bottom(list_state: &gpui::ListState, count: usize) -> bool {
    let a = AnchorSnapshot::from(list_state.logical_scroll_top());
    a.top_item_index == count && a.sub_offset == px(0.)
}

fn message_item(id: &str, text: &str) -> lens_core::domain::item::Item {
    lens_core::domain::item::Item {
        id: ItemId::new(id),
        seq: None,
        ctx: BlockContext {
            agent: None,
            depth: 0,
            response_id: Some(ResponseId::new("resp_a")),
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
                message_item(&format!("m{i}"), &format!("row {i}")),
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
    });
}

fn append_row(
    replica: &Entity<FocusedTranscript>,
    id: &str,
    text: &str,
    ordinal: i64,
    cx: &mut App,
) {
    replica.update(cx, |r, cx| {
        r.apply_read(
            1,
            ReadRange::Delta {
                after: ordinal.saturating_sub(1),
                through: ordinal,
            },
            RangeRead {
                rows: vec![(ordinal, message_item(id, text))],
                skipped: vec![],
                watermark: Some(ordinal),
            },
            cx,
        );
    });
}

async fn drive_scroll_probe(
    weak: WeakEntity<HarnessView>,
    mut wcx: AsyncWindowContext,
    exit_ok: Rc<RefCell<bool>>,
) {
    wait_frames(&mut wcx, 3).await;

    // Seed baseline rows (view mount already reset list).
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        seed_rows(&view.replica, 8, cx);
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Contract 1: append while following stays pinned.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        append_row(&view.replica, "m8", "appended while following", 8, cx);
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Scroll up → pause.
    let anchor_before_pause = weak
        .update_in(&mut wcx, |view, _, cx| {
            let list = view.replica.read(cx).list_state();
            list.scroll_by(px(-400.));
            AnchorSnapshot::from(list.logical_scroll_top())
        })
        .unwrap();
    wait_frames(&mut wcx, 4).await;

    // Contract 2 + paused-not-yanked: append while paused.
    let anchor_before_paused_appends = weak
        .update_in(&mut wcx, |view, _, cx| {
            AnchorSnapshot::from(view.replica.read(cx).list_state().logical_scroll_top())
        })
        .unwrap();
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        append_row(&view.replica, "m9", "while paused 1", 9, cx);
        append_row(&view.replica, "m10", "while paused 2", 10, cx);
        append_row(&view.replica, "m11", "while paused 3", 11, cx);
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;
    let anchor_after_paused_appends = weak
        .update_in(&mut wcx, |view, _, cx| {
            AnchorSnapshot::from(view.replica.read(cx).list_state().logical_scroll_top())
        })
        .unwrap();

    // Pill click → bottom + resume.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.transcript
            .update(cx, |v, cx| v.jump_to_latest_for_test(cx));
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Contract 3: finalize anchor stable.
    let acc = AccId::new("acc_scroll_fin");
    let item_id = ItemId::new("msg_scroll_fin");
    let anchor_before_finalize = weak
        .update_in(&mut wcx, |view, _, cx| {
            view.replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ScratchChanged(std::sync::Arc::new(StreamScratch {
                        open_message: Some(lens_core::domain::item::MessageAcc {
                            acc_id: acc.clone(),
                            message_id: None,
                            text: "streaming".into(),
                            block_index: 0,
                        }),
                        ..Default::default()
                    })),
                    cx,
                );
            });
            let list = view.replica.read(cx).list_state();
            list.scroll_by(px(-200.));
            AnchorSnapshot::from(list.logical_scroll_top())
        })
        .unwrap();

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.fold_detailed(
                StreamUpdate::Retired {
                    acc_id: acc.clone(),
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
        cx.notify();
    });
    wait_frames(&mut wcx, 3).await;

    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.apply_read(
                1,
                ReadRange::Delta {
                    after: 11,
                    through: 12,
                },
                RangeRead {
                    rows: vec![(12, message_item("msg_scroll_fin", "finalized"))],
                    skipped: vec![],
                    watermark: Some(12),
                },
                cx,
            );
        });
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    let anchor_after_finalize = weak
        .update_in(&mut wcx, |view, _, cx| {
            AnchorSnapshot::from(view.replica.read(cx).list_state().logical_scroll_top())
        })
        .unwrap();

    let ok = weak
        .update_in(&mut wcx, |view, _, _| {
            let mut p = view.probe.borrow_mut();
            p.finalize_anchor_stable = Some(anchor_before_finalize == anchor_after_finalize);
            p.paused_anchor_stable =
                Some(anchor_before_paused_appends == anchor_after_paused_appends);
            if !p.saw_initial_bottom {
                p.failures
                    .push("contract 4: new session did not land at bottom".into());
            }
            if !p.saw_stick_while_following {
                p.failures
                    .push("contract 1: stick-to-bottom while following failed".into());
            }
            if !p.saw_paused_on_scroll {
                p.failures
                    .push("contract 1: scroll-up did not pause auto-follow".into());
            }
            if !p.saw_pill_while_paused {
                p.failures
                    .push("contract 2: pill not visible while paused".into());
            }
            if !p.saw_pill_n_three {
                p.failures.push("contract 2: pill N != 3".into());
            }
            if !p.saw_resume_following {
                p.failures
                    .push("contract 2: jump-to-latest did not resume Following".into());
            }
            if p.finalize_anchor_stable != Some(true) {
                p.failures.push(format!(
                    "contract 3: anchor jumped on finalize {:?} -> {:?}",
                    anchor_before_finalize, anchor_after_finalize
                ));
            }
            if p.paused_anchor_stable != Some(true) {
                p.failures.push(format!(
                    "paused-not-yanked: anchor shifted {:?} -> {:?}",
                    anchor_before_paused_appends, anchor_after_paused_appends
                ));
            }
            if anchor_before_pause == anchor_after_paused_appends {
                p.failures.push("scroll-up had no effect on anchor".into());
            }
            p.failures.is_empty() && p.samples >= 8
        })
        .unwrap_or(false);

    if !ok {
        let _ = weak.update_in(&mut wcx, |view, _, _| {
            let p = view.probe.borrow();
            eprintln!("SCROLL PROBE FAILURES: {:?}", p.failures);
            eprintln!("samples={}", p.samples);
        });
    }
    *exit_ok.borrow_mut() = ok;
    let _ = wcx.update(|_, cx| cx.quit());
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.spawned {
            self.spawned = true;
            let exit_ok = Rc::clone(&self.exit_ok);
            cx.spawn_in(window, move |weak, wcx: &mut AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    drive_scroll_probe(weak, wcx, exit_ok).await;
                }
            })
            .detach();
        }

        let count = self.replica.read(cx).rows().len();
        let list = self.replica.read(cx).list_state();
        let transcript = self.transcript.read(cx);
        let follow = transcript.follow_mode();
        let pill_n = transcript.new_since_pause_for_test(cx);
        let pill_visible = transcript.pill_visible_for_test(cx);

        {
            let mut p = self.probe.borrow_mut();
            if count >= 8 && at_bottom(list, count) {
                p.saw_initial_bottom = true;
            }
            if count >= 9 && follow == FollowMode::Following && at_bottom(list, count) {
                p.saw_stick_while_following = true;
            }
            if follow == FollowMode::Paused {
                p.saw_paused_on_scroll = true;
            }
            if pill_visible && follow == FollowMode::Paused {
                p.saw_pill_while_paused = true;
            }
            if pill_n == 3 {
                p.saw_pill_n_three = true;
            }
            if follow == FollowMode::Following && at_bottom(list, count) && count >= 12 {
                p.saw_resume_following = true;
            }
            p.samples += 1;
        }

        div().size_full().child(self.transcript.clone())
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
        let session_id = lens_core::domain::ids::SessionId::new("sess_scroll");
        let replica = cx.new(|cx| {
            FocusedTranscript::new_test_no_baseline(
                reader,
                session_id,
                ReconcileEpoch::default(),
                1,
                cx,
            )
        });
        let transcript = cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx));

        cx.open_window(gpui::WindowOptions::default(), move |_window, cx| {
            cx.new(|_| HarnessView {
                replica: replica.clone(),
                transcript: transcript.clone(),
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
