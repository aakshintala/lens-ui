//! Store-owned focused transcript replica (T-2 §5). State machine + fold rules;
//! rendering (Task 13) and staged finalize (Task 12) land later.

use crate::fleet::store::{ReaderFactory, ReconcileEpoch};
use async_channel::Receiver;
use gpui::Context;
use lens_core::domain::ids::{AccId, ResponseId, SessionId};
use lens_core::domain::item::{Item, StreamScratch};
use lens_core::persist::{RangeRead, ReadRange};
use lens_core::reduce::{StreamUpdate, project};
use std::collections::HashMap;
use std::sync::Arc;

// ── reader-enqueue seam (Task 10 moves worker loop to `focused/reader.rs`) ──

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Baseline,
    Delta,
    Reconcile,
    Rewrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadTarget {
    pub range: ReadRange,
    pub generation: u64,
    pub priority: Priority,
}

#[derive(Clone)]
pub struct ReaderWorkerHandle {
    tx: async_channel::Sender<ReadTarget>,
}

impl ReaderWorkerHandle {
    pub fn enqueue(&self, target: ReadTarget) {
        let _ = self.tx.try_send(target);
    }

    fn new_channel() -> (Self, Receiver<ReadTarget>) {
        let (tx, rx) = async_channel::bounded(16);
        (Self { tx }, rx)
    }

    /// Test-only: observe exactly what the replica enqueues.
    #[cfg(test)]
    pub fn new_test() -> (Self, Receiver<ReadTarget>) {
        Self::new_channel()
    }
}

// ── stubs — later tasks flesh these out ──

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowStore; // stub — Task 11 fleshes out

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowPresentation; // stub — Task 11 fleshes out

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marker; // stub — Task 14 fleshes out

// ── replica ──

pub struct FocusedTranscript {
    items: Vec<Item>,
    scratch: Arc<StreamScratch>,
    active_response: Option<ResponseId>,
    last_rendered_ordinal: i64,
    live_section_start: usize,
    #[allow(dead_code)] // Task 11
    rows: RowStore,
    #[allow(dead_code)] // Task 12
    pending_finalize: HashMap<AccId, RowPresentation>,
    #[allow(dead_code)] // Task 14
    markers: Vec<Marker>,
    focus_generation: u64,
    reader: ReaderWorkerHandle,
    #[allow(dead_code)]
    session_id: SessionId,
    baseline_epoch: u64,
    baseline_reconcile_in_flight: bool,
    /// Cheap probe: projection block count for `&items[live_section_start..]` + scratch.
    live_section_projection_count: usize,
}

impl FocusedTranscript {
    pub fn new(
        factory: ReaderFactory,
        seed_epoch: ReconcileEpoch,
        focus_generation: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let (reader, _rx) = ReaderWorkerHandle::new_channel();
        Self::new_with_reader(
            reader,
            factory.session_id().clone(),
            seed_epoch,
            focus_generation,
            cx,
        )
    }

    fn new_with_reader(
        reader: ReaderWorkerHandle,
        session_id: SessionId,
        seed_epoch: ReconcileEpoch,
        focus_generation: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let replica = Self {
            items: Vec::new(),
            scratch: Arc::new(StreamScratch::default()),
            active_response: None,
            last_rendered_ordinal: -1,
            live_section_start: 0,
            rows: RowStore,
            pending_finalize: HashMap::new(),
            markers: Vec::new(),
            focus_generation,
            reader,
            session_id,
            baseline_epoch: seed_epoch.epoch,
            baseline_reconcile_in_flight: seed_epoch.in_flight,
            live_section_projection_count: 0,
        };
        replica.enqueue_read(ReadRange::All, Priority::Baseline, focus_generation);
        cx.notify();
        replica
    }

    pub fn fold_detailed(&mut self, u: StreamUpdate, cx: &mut Context<Self>) {
        let mut dirty = false;
        match u {
            StreamUpdate::Rebased(state) => {
                self.active_response = state.active_response.clone();
                self.recompute_live_section_start();
                dirty = true;
            }
            StreamUpdate::TranscriptAdvanced {
                committed_ordinal: ord,
            } => {
                if ord > self.last_rendered_ordinal {
                    self.enqueue_read(
                        ReadRange::Delta {
                            after: self.last_rendered_ordinal,
                            through: ord,
                        },
                        Priority::Delta,
                        self.focus_generation,
                    );
                }
            }
            StreamUpdate::TranscriptRewritten { ordinal: ord } => {
                self.enqueue_read(
                    ReadRange::One { ordinal: ord },
                    Priority::Rewrite,
                    self.focus_generation,
                );
            }
            StreamUpdate::ActiveResponseChanged(r) => {
                self.active_response = r;
                self.recompute_live_section_start();
                self.reproject_live_section();
                dirty = true;
            }
            StreamUpdate::ScratchChanged(scratch) => {
                self.scratch = scratch;
                self.reproject_live_section();
                dirty = true;
            }
            StreamUpdate::Retired { .. }
            | StreamUpdate::StatusChanged(_)
            | StreamUpdate::LastTaskErrorChanged(_)
            | StreamUpdate::UsageChanged(_)
            | StreamUpdate::ModelChanged { .. }
            | StreamUpdate::ReasoningEffortChanged(_)
            | StreamUpdate::CollaborationModeChanged(_)
            | StreamUpdate::ModelOptionsChanged(_)
            | StreamUpdate::TodosChanged(_)
            | StreamUpdate::SkillsChanged(_)
            | StreamUpdate::SandboxChanged(_)
            | StreamUpdate::TerminalPendingChanged(_)
            | StreamUpdate::ElicitationsChanged(_)
            | StreamUpdate::PendingUserChanged(_)
            | StreamUpdate::ChildSessionChanged
            | StreamUpdate::PresenceChanged(_)
            | StreamUpdate::ResourcesChanged
            | StreamUpdate::AgentChanged { .. }
            | StreamUpdate::TitleChanged(_)
            | StreamUpdate::LastTokensChanged(_)
            | StreamUpdate::ContextWindowChanged(_)
            | StreamUpdate::Reconnecting { .. }
            | StreamUpdate::Reconnected { .. }
            | StreamUpdate::Disconnected(_)
            | StreamUpdate::SnapshotRestored(_) => {}
        }
        if dirty {
            cx.notify();
        }
    }

    pub fn on_reconcile_epoch_settled(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.epoch_overlapped_baseline(epoch) {
            self.enqueue_read(ReadRange::All, Priority::Reconcile, self.focus_generation);
            cx.notify();
        }
    }

    pub fn apply_read(
        &mut self,
        generation: u64,
        range: ReadRange,
        read: RangeRead,
        cx: &mut Context<Self>,
    ) {
        if generation != self.focus_generation {
            return;
        }
        match range {
            ReadRange::All => self.replace_read_rows(&read.rows),
            ReadRange::Delta { .. } | ReadRange::One { .. } => {
                self.upsert_read_rows(&read.rows);
            }
        }
        if let Some(watermark) = read.watermark {
            self.last_rendered_ordinal = watermark;
        }
        self.recompute_live_section_start();
        self.reproject_live_section();
        cx.notify();
    }

    fn enqueue_read(&self, range: ReadRange, priority: Priority, generation: u64) {
        self.reader.enqueue(ReadTarget {
            range,
            generation,
            priority,
        });
    }

    fn epoch_overlapped_baseline(&self, epoch: u64) -> bool {
        if self.baseline_reconcile_in_flight && epoch == self.baseline_epoch {
            return true;
        }
        epoch > self.baseline_epoch
    }

    fn recompute_live_section_start(&mut self) {
        let Some(active) = &self.active_response else {
            self.live_section_start = self.items.len();
            return;
        };
        self.live_section_start = self
            .items
            .iter()
            .position(|item| item.ctx.response_id.as_ref() == Some(active))
            .unwrap_or(self.items.len());
    }

    fn reproject_live_section(&mut self) {
        let slice = &self.items[self.live_section_start..];
        let refs: Vec<&Item> = slice.iter().collect();
        self.live_section_projection_count =
            project(&refs, self.scratch.as_ref(), self.active_response.as_ref()).len();
    }

    fn replace_read_rows(&mut self, rows: &[(i64, Item)]) {
        let mut sorted: Vec<_> = rows.iter().collect();
        sorted.sort_by_key(|(ordinal, _)| *ordinal);
        self.items = sorted.into_iter().map(|(_, item)| item.clone()).collect();
    }

    fn upsert_read_rows(&mut self, rows: &[(i64, Item)]) {
        for (ordinal, item) in rows {
            self.items.retain(|existing| existing.id != item.id);
            let ord = *ordinal as usize;
            if ord >= self.items.len() {
                self.items.push(item.clone());
            } else {
                self.items.insert(ord, item.clone());
            }
        }
    }

    #[cfg(test)]
    fn seed_item(&mut self, ordinal: usize, item: Item) {
        if ordinal >= self.items.len() {
            self.items.push(item);
        } else {
            self.items.insert(ordinal, item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    use lens_core::domain::ids::{AgentId, ConnectionId, ItemId, SessionId as Sid};
    use lens_core::domain::item::{BlockContext, ContentBlock, ItemKind};
    use lens_core::domain::scalars::{Role, SessionStatusValue};
    use lens_core::domain::session::SessionState;

    fn new_replica(
        cx: &mut gpui::TestAppContext,
        seed_epoch: ReconcileEpoch,
        focus_generation: u64,
    ) -> (gpui::Entity<FocusedTranscript>, Receiver<ReadTarget>) {
        let (reader, rx) = ReaderWorkerHandle::new_test();
        let session_id = Sid::new("sess_test");
        let replica = cx.update(|cx| {
            cx.new(|cx| {
                FocusedTranscript::new_with_reader(
                    reader,
                    session_id,
                    seed_epoch,
                    focus_generation,
                    cx,
                )
            })
        });
        (replica, rx)
    }

    fn message_item(id: &str, response_id: Option<&str>) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: response_id.map(ResponseId::new),
            },
            created_at: 1,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hi".into()),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }

    fn rebased_state(active: Option<ResponseId>, title: Option<&str>) -> SessionState {
        let mut state =
            SessionState::new(ConnectionId::new("c"), Sid::new("s"), AgentId::new("ag"));
        state.active_response = active;
        state.title = title.map(str::to_owned);
        state.status = SessionStatusValue::Running;
        state
    }

    #[gpui::test]
    async fn rebased_updates_scalars_never_clears_items(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let baseline = rx.try_recv().expect("baseline read at new");
        assert_eq!(baseline.range, ReadRange::All);
        assert_eq!(baseline.priority, Priority::Baseline);

        let resp = ResponseId::new("resp_a");
        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.seed_item(0, message_item("item_a", Some("resp_a")));
                r.active_response = Some(resp.clone());
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::Rebased(Box::new(rebased_state(
                        Some(ResponseId::new("resp_b")),
                        Some("new title"),
                    ))),
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.items.len(), 1, "Rebased must never clear items");
            assert_eq!(r.active_response.as_ref(), Some(&ResponseId::new("resp_b")));
        });
        assert!(rx.try_recv().is_err(), "Rebased does not enqueue reads");
    }

    #[gpui::test]
    async fn transcript_advanced_enqueues_delta_and_skips_stale(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.last_rendered_ordinal = 2;
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::TranscriptAdvanced {
                        committed_ordinal: 5,
                    },
                    cx,
                );
            });
            replica.update(cx, |r, _| {
                r.last_rendered_ordinal = 5;
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::TranscriptAdvanced {
                        committed_ordinal: 3,
                    },
                    cx,
                );
            });
        });
        let delta = rx.try_recv().expect("delta read");
        assert_eq!(
            delta.range,
            ReadRange::Delta {
                after: 2,
                through: 5
            }
        );
        assert_eq!(delta.priority, Priority::Delta);
        assert!(rx.try_recv().is_err(), "stale ord is a forward no-op");
    }

    #[gpui::test]
    async fn transcript_rewritten_enqueues_one(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(StreamUpdate::TranscriptRewritten { ordinal: 7 }, cx);
            });
        });

        let one = rx.try_recv().expect("one read");
        assert_eq!(one.range, ReadRange::One { ordinal: 7 });
        assert_eq!(one.priority, Priority::Rewrite);
    }

    #[gpui::test]
    async fn active_response_changed_recomputes_live_section_start(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let resp_b = ResponseId::new("resp_b");
        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.seed_item(0, message_item("m0", Some("resp_a")));
                r.seed_item(1, message_item("m1", Some("resp_b")));
                r.seed_item(2, message_item("m2", Some("resp_b")));
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ActiveResponseChanged(Some(resp_b.clone())),
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.live_section_start, 1);
            assert_eq!(r.active_response.as_ref(), Some(&resp_b));
        });

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(StreamUpdate::ActiveResponseChanged(None), cx);
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.live_section_start, 3);
        });

        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.items.clear();
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ActiveResponseChanged(Some(ResponseId::new("resp_x"))),
                    cx,
                );
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.live_section_start, 0);
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn scratch_changed_reprojects_live_section(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.seed_item(0, message_item("m0", Some("resp_a")));
                r.live_section_start = 0;
                r.reproject_live_section();
            });
        });
        let before = cx.read(|cx| replica.read(cx).live_section_projection_count);

        let scratch = StreamScratch {
            open_message: Some(lens_core::domain::item::MessageAcc {
                acc_id: AccId::new("acc_1"),
                message_id: None,
                text: "streaming".into(),
                block_index: 0,
            }),
            ..StreamScratch::default()
        };
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(StreamUpdate::ScratchChanged(Arc::new(scratch)), cx);
            });
        });

        let after = cx.read(|cx| replica.read(cx).live_section_projection_count);
        assert!(
            after > before,
            "scratch change must re-project live section"
        );
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn reconcile_epoch_settled_enqueues_all_when_overlapped(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(
            cx,
            ReconcileEpoch {
                epoch: 3,
                in_flight: true,
            },
            1,
        );
        let _ = rx.try_recv().expect("baseline");

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.on_reconcile_epoch_settled(3, cx);
            });
        });

        let reconcile = rx.try_recv().expect("reconcile re-read");
        assert_eq!(reconcile.range, ReadRange::All);
        assert_eq!(reconcile.priority, Priority::Reconcile);
    }

    fn range_read(rows: Vec<(i64, Item)>, watermark: i64) -> RangeRead {
        RangeRead {
            rows,
            skipped: vec![],
            watermark: Some(watermark),
        }
    }

    #[gpui::test]
    async fn reconcile_drops_deleted_row(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let a = message_item("a", None);
        let b = message_item("b", None);
        let c = message_item("c", None);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::All,
                    range_read(vec![(0, a.clone()), (1, b.clone()), (2, c.clone())], 2),
                    cx,
                );
            });
        });
        cx.read(|cx| {
            assert_eq!(replica.read(cx).items.len(), 3);
        });

        let c_reord = message_item("c", None);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::All,
                    range_read(vec![(0, a.clone()), (1, c_reord.clone())], 1),
                    cx,
                );
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.items.len(), 2, "deleted row b must not ghost");
            assert_eq!(r.items[0].id, a.id);
            assert_eq!(r.items[1].id, c_reord.id);
            assert!(
                r.items.iter().all(|item| item.id != b.id),
                "b must be gone after reconcile All read"
            );
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn reconcile_rekey_no_stale_twin(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let x = message_item("x", None);
        let y = message_item("y", None);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(1, ReadRange::All, range_read(vec![(0, x.clone())], 0), cx);
            });
        });
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(1, ReadRange::All, range_read(vec![(0, y.clone())], 0), cx);
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.items.len(), 1, "rekey must not leave stale twin");
            assert_eq!(r.items[0].id, y.id);
            assert!(
                r.items.iter().all(|item| item.id != x.id),
                "stale x must be gone after reconcile All read"
            );
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn delta_still_upserts(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let a = message_item("a", None);
        let b = message_item("b", None);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(1, ReadRange::All, range_read(vec![(0, a.clone())], 0), cx);
            });
        });
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::Delta {
                        after: 0,
                        through: 1,
                    },
                    range_read(vec![(1, b.clone())], 1),
                    cx,
                );
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(
                r.items.len(),
                2,
                "delta must append without dropping baseline rows"
            );
            assert_eq!(r.items[0].id, a.id);
            assert_eq!(r.items[1].id, b.id);
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn apply_read_drops_stale_generation(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let item = message_item("item_stale", None);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    99,
                    ReadRange::All,
                    RangeRead {
                        rows: vec![(0, item.clone())],
                        skipped: vec![],
                        watermark: Some(0),
                    },
                    cx,
                );
            });
        });
        cx.read(|cx| {
            assert!(replica.read(cx).items.is_empty());
        });

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::All,
                    RangeRead {
                        rows: vec![(0, item)],
                        skipped: vec![],
                        watermark: Some(0),
                    },
                    cx,
                );
            });
        });
        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.items.len(), 1);
            assert_eq!(r.last_rendered_ordinal, 0);
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn focus_mid_reconcile_rereads_on_epoch_settle(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(
            cx,
            ReconcileEpoch {
                epoch: 2,
                in_flight: true,
            },
            7,
        );
        let baseline = rx.try_recv().expect("baseline at focus");
        assert_eq!(baseline.generation, 7);

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.on_reconcile_epoch_settled(2, cx);
            });
        });

        let reconcile = rx.try_recv().expect("Imp-4 reconcile re-read");
        assert_eq!(reconcile.range, ReadRange::All);
        assert_eq!(reconcile.generation, 7);
        assert_eq!(reconcile.priority, Priority::Reconcile);
    }
}
