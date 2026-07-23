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
    finalize_anchor_stable: Option<bool>,
    paused_anchor_stable: Option<bool>,
    prepend_anchor_stable: Option<bool>,
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

/// Baseline rows. Must overflow the viewport so a Bottom-aligned scroll leaves
/// the bottom (`logical_scroll_top` stays `Some` → `is_scrolled = true`).
/// A viewport that content does not fill resets `logical_scroll_top` to `None`
/// on every paint, so no scroll ever registers as paused.
const SEED: usize = 40;

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

fn row_with_len(
    ord: i64,
    item: lens_core::domain::item::Item,
) -> (i64, usize, lens_core::domain::item::Item) {
    (ord, 4, item)
}

/// Row carrying an explicit resident byte length — used to drive resident_bytes
/// toward RESIDENT_CAP_BYTES so a backward page forces hi-side eviction (C2).
fn row_with_len_bytes(
    ord: i64,
    bytes: usize,
    item: lens_core::domain::item::Item,
) -> (i64, usize, lens_core::domain::item::Item) {
    (ord, bytes, item)
}

fn seed_rows(replica: &Entity<FocusedTranscript>, count: usize, cx: &mut App) {
    let rows: Vec<_> = (0..count)
        .map(|i| {
            row_with_len(
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
                rows: vec![row_with_len(ordinal, message_item(id, text))],
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

    // Seed baseline rows (view mount already reset list). SEED rows overflow
    // the viewport so scrolling registers as a real departure from the bottom.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        seed_rows(&view.replica, SEED, cx);
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Contract 1: append while following stays pinned.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        append_row(
            &view.replica,
            &format!("m{SEED}"),
            "appended while following",
            SEED as i64,
            cx,
        );
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Scroll up (real paint anchor move). gpui walls off synthetic
    // ScrollWheelEvent injection, so this cannot fire the view's scroll handler
    // (follow-mode pause is unit-tested in view.rs::on_scroll_event_*). What
    // the real window uniquely proves is that a live append while scrolled up
    // does NOT yank the anchor to the bottom.
    let anchor_before_pause = weak
        .update_in(&mut wcx, |view, _, cx| {
            let list = view.replica.read(cx).list_state();
            list.scroll_by(px(-400.));
            AnchorSnapshot::from(list.logical_scroll_top())
        })
        .unwrap();
    wait_frames(&mut wcx, 4).await;

    // paused-not-yanked: append while scrolled up.
    let anchor_before_paused_appends = weak
        .update_in(&mut wcx, |view, _, cx| {
            AnchorSnapshot::from(view.replica.read(cx).list_state().logical_scroll_top())
        })
        .unwrap();
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        append_row(
            &view.replica,
            &format!("m{}", SEED + 1),
            "while paused 1",
            SEED as i64 + 1,
            cx,
        );
        append_row(
            &view.replica,
            &format!("m{}", SEED + 2),
            "while paused 2",
            SEED as i64 + 2,
            cx,
        );
        append_row(
            &view.replica,
            &format!("m{}", SEED + 3),
            "while paused 3",
            SEED as i64 + 3,
            cx,
        );
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;
    let anchor_after_paused_appends = weak
        .update_in(&mut wcx, |view, _, cx| {
            AnchorSnapshot::from(view.replica.read(cx).list_state().logical_scroll_top())
        })
        .unwrap();

    // Contract 3: finalize anchor stable (still scrolled up from above).
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
                    after: SEED as i64 + 3,
                    through: SEED as i64 + 4,
                },
                RangeRead {
                    rows: vec![row_with_len(
                        SEED as i64 + 4,
                        message_item("msg_scroll_fin", "finalized"),
                    )],
                    skipped: vec![],
                    watermark: Some(SEED as i64 + 4),
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

    // Prepend-anchor contract: backward page must not yank visible content.
    //
    // Seed a LARGE tail (200 rows) so that when we scroll up, the anchor lands
    // with enough content BELOW it to fill the viewport. Otherwise a
    // Bottom-aligned list snaps `logical_scroll_top` back to `None` (bottom)
    // during layout whenever the rows below the anchor cannot fill the viewport
    // (gpui list.rs `ListAlignment::Bottom => logical_scroll_top = None`), and
    // the prepend contract cannot be observed at all. We also read the
    // pre-prepend anchor AFTER a settle frame so it reflects the laid-out
    // (post-snap) position, comparable to the post-prepend read.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        let tail_rows: Vec<_> = (50..250)
            .map(|i| {
                row_with_len(
                    i,
                    message_item(&format!("tail{i}"), &format!("tail row {i}")),
                )
            })
            .collect();
        view.replica.update(cx, |r, cx| {
            r.apply_read(
                1,
                ReadRange::All,
                RangeRead {
                    rows: tail_rows,
                    skipped: vec![],
                    watermark: Some(249),
                },
                cx,
            );
        });
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Scroll far up, then settle a frame so the anchor is a stable laid-out
    // position (not the raw pre-layout `scroll_by` result).
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.read(cx).list_state().scroll_by(px(-3000.));
    });
    wait_frames(&mut wcx, 4).await;

    let (anchor_before_prepend, row_count_before, top_row_before) = weak
        .update_in(&mut wcx, |view, _, cx| {
            let replica = view.replica.read(cx);
            let a = AnchorSnapshot::from(replica.list_state().logical_scroll_top());
            let top = replica
                .rows()
                .order()
                .get(a.top_item_index)
                .map(|id| format!("{id:?}"));
            (a, replica.rows().order().len(), top)
        })
        .unwrap();
    // Guard against a degenerate setup: if the anchor snapped to the bottom
    // sentinel (item_ix == count), we are not genuinely scrolled up and the
    // prepend contract would pass vacuously. Surface it as a failure.
    let scrolled_up_before = anchor_before_prepend.top_item_index < row_count_before;

    let resident_lo = weak
        .update_in(&mut wcx, |view, _, cx| view.replica.read(cx).resident_lo())
        .unwrap();

    let prepend_rows: Vec<_> = (40..49)
        .map(|i| row_with_len(i, message_item(&format!("pre{i}"), &format!("pre row {i}"))))
        .collect();
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.apply_read(
                1,
                ReadRange::Backward {
                    before: resident_lo,
                    byte_budget: 4 * 1024 * 1024,
                },
                RangeRead {
                    rows: prepend_rows,
                    skipped: vec![],
                    watermark: Some(249),
                },
                cx,
            );
        });
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    let (anchor_after_prepend, top_row_after) = weak
        .update_in(&mut wcx, |view, _, cx| {
            let replica = view.replica.read(cx);
            let a = AnchorSnapshot::from(replica.list_state().logical_scroll_top());
            let top = replica
                .rows()
                .order()
                .get(a.top_item_index)
                .map(|id| format!("{id:?}"));
            (a, top)
        })
        .unwrap();
    // The row at the top of the viewport must be identical before and after the
    // prepend — the arithmetic-independent statement of "content did not yank".
    let prepend_identity_ok = top_row_before.is_some() && top_row_before == top_row_after;

    // ── C2: backward prepend + hi-side eviction must not yank visible content ──
    // The prepend section above stays under RESIDENT_CAP_BYTES (tiny rows), so it
    // never exercises the eviction path. Here we seed LARGE-byte rows so a backward
    // page tips resident over the 24 MB cap; paused (`following=false`) eviction then
    // drops rows from the HI (newest) side while the anchor sits near the top. The
    // arithmetic is pinned headless; this proves the two-splice survives REAL layout:
    // the RowId at the top of the viewport must be identical before and after.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            r.set_following(false, cx);
            let rows: Vec<_> = (1000..1200)
                .map(|i| {
                    row_with_len_bytes(
                        i,
                        110 * 1024,
                        message_item(&format!("big{i}"), &format!("big row {i}")),
                    )
                })
                .collect();
            r.apply_read(
                1,
                ReadRange::All,
                RangeRead {
                    rows,
                    skipped: vec![],
                    watermark: Some(1199),
                },
                cx,
            );
        });
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    // Scroll up and settle so the anchor is a stable, genuinely-scrolled-up position.
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.read(cx).list_state().scroll_by(px(-4000.));
    });
    wait_frames(&mut wcx, 4).await;

    let (c2_anchor_before, c2_top_before, c2_count_before) = weak
        .update_in(&mut wcx, |view, _, cx| {
            let r = view.replica.read(cx);
            let a = AnchorSnapshot::from(r.list_state().logical_scroll_top());
            let top = r
                .rows()
                .order()
                .get(a.top_item_index)
                .map(|id| format!("{id:?}"));
            (a, top, r.rows().order().len())
        })
        .unwrap();
    let c2_resident_lo = weak
        .update_in(&mut wcx, |view, _, cx| view.replica.read(cx).resident_lo())
        .unwrap();

    const C2_INSERTED: usize = 40;
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.replica.update(cx, |r, cx| {
            let rows: Vec<_> = ((c2_resident_lo - C2_INSERTED as i64)..c2_resident_lo)
                .map(|i| {
                    row_with_len_bytes(
                        i,
                        110 * 1024,
                        message_item(&format!("big{i}"), &format!("big row {i}")),
                    )
                })
                .collect();
            r.apply_read(
                1,
                ReadRange::Backward {
                    before: c2_resident_lo,
                    byte_budget: 4 * 1024 * 1024,
                },
                RangeRead {
                    rows,
                    skipped: vec![],
                    watermark: Some(1199),
                },
                cx,
            );
        });
        cx.notify();
    });
    wait_frames(&mut wcx, 4).await;

    let (c2_top_after, c2_count_after) = weak
        .update_in(&mut wcx, |view, _, cx| {
            let r = view.replica.read(cx);
            let a = AnchorSnapshot::from(r.list_state().logical_scroll_top());
            let top = r
                .rows()
                .order()
                .get(a.top_item_index)
                .map(|id| format!("{id:?}"));
            (top, r.rows().order().len())
        })
        .unwrap();

    // Setup guards: genuinely scrolled up, and the page actually forced eviction
    // (else the eviction arm of C2 is untested — a false-green).
    let c2_scrolled_up = c2_anchor_before.top_item_index < c2_count_before;
    let c2_evicted = c2_count_after < c2_count_before + C2_INSERTED;
    let c2_identity_ok = c2_top_before.is_some() && c2_top_before == c2_top_after;

    let ok = weak
        .update_in(&mut wcx, |view, _, _| {
            let mut p = view.probe.borrow_mut();
            p.finalize_anchor_stable = Some(anchor_before_finalize == anchor_after_finalize);
            p.paused_anchor_stable =
                Some(anchor_before_paused_appends == anchor_after_paused_appends);
            p.prepend_anchor_stable = Some(prepend_identity_ok);
            if !p.saw_initial_bottom {
                p.failures
                    .push("contract 4: new session did not land at bottom".into());
            }
            if !p.saw_stick_while_following {
                p.failures
                    .push("contract 1: stick-to-bottom while following failed".into());
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
            if !scrolled_up_before {
                p.failures.push(format!(
                    "prepend-anchor setup: not scrolled up before prepend (anchor {:?}, count {})",
                    anchor_before_prepend, row_count_before
                ));
            }
            if p.prepend_anchor_stable != Some(true) {
                p.failures.push(format!(
                    "prepend-anchor: visible content yanked, top row {:?} -> {:?} (anchor {:?} -> {:?})",
                    top_row_before, top_row_after, anchor_before_prepend, anchor_after_prepend
                ));
            }
            if !c2_scrolled_up {
                p.failures.push(format!(
                    "C2 setup: not scrolled up before eviction page (anchor {:?}, count {})",
                    c2_anchor_before, c2_count_before
                ));
            }
            if !c2_evicted {
                p.failures.push(format!(
                    "C2 setup: backward page did not force hi eviction (count {} -> {}, inserted {})",
                    c2_count_before, c2_count_after, C2_INSERTED
                ));
            }
            if !c2_identity_ok {
                p.failures.push(format!(
                    "C2 prepend+eviction yanked visible content: top row {:?} -> {:?}",
                    c2_top_before, c2_top_after
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
    } else {
        eprintln!("SCROLL PROBE: all contracts passed (samples ok)");
    }
    *exit_ok.borrow_mut() = ok;
    // `cx.quit()` routes through AppKit `[NSApp terminate:]`, which calls
    // `exit(0)` itself and never returns to `main`'s `process::exit` — so the
    // exit code would always be 0. Exit directly here to make it trustworthy.
    process::exit(if ok { 0 } else { 1 });
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
        let follow = self.transcript.read(cx).follow_mode();

        {
            let mut p = self.probe.borrow_mut();
            if count >= SEED && at_bottom(list, count) {
                p.saw_initial_bottom = true;
            }
            if count > SEED && follow == FollowMode::Following && at_bottom(list, count) {
                p.saw_stick_while_following = true;
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
