//! Store-owned focused transcript replica (T-2 §5). State machine + fold rules;
//! two-level retained rows, staged finalize, and collapse timing (Task 12).

pub mod reader;
mod rowsource;

use crate::fleet::store::{ReaderFactory, ReconcileEpoch};
use gpui::{Context, ListAlignment, ListState, Pixels};
use lens_core::domain::ids::{AccId, ItemId, ResponseId, SessionId};
use lens_core::domain::item::{Item, ItemKind, StreamScratch};
use lens_core::domain::scalars::Role;
use lens_core::persist::{RangeRead, ReadRange};
use lens_core::reduce::{RetireDisposition, StreamUpdate, group_work_section, project};
use std::collections::HashMap;
use std::sync::Arc;

pub use reader::{Priority, ReadTarget, ReaderWorkerHandle};
pub use rowsource::{
    RowId, RowKind, RowPresentation, RowState, RowStore, SectionKey, UpsertEffect,
};

const LIST_OVERDRAW: Pixels = gpui::px(200.);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marker; // stub — Task 14 fleshes out

pub struct FocusedTranscript {
    items: Vec<Item>,
    scratch: Arc<StreamScratch>,
    active_response: Option<ResponseId>,
    last_rendered_ordinal: i64,
    live_section_start: usize,
    rows: RowStore,
    list_state: ListState,
    pending_finalize: HashMap<AccId, RowPresentation>,
    pending_item_ids: HashMap<ItemId, AccId>,
    #[allow(dead_code)] // populated in Task 14 (ReconnectBreak markers)
    markers: Vec<Marker>,
    focus_generation: u64,
    reader: ReaderWorkerHandle,
    reader_error: Option<String>,
    #[allow(dead_code)] // read in Task 13 (view/mount) + Task 15 (syncing indicator)
    session_id: SessionId,
    baseline_epoch: u64,
    baseline_reconcile_in_flight: bool,
    live_section_projection_count: usize,
    /// Top-level structure entries materialized from settled prefix (D-1 cache index).
    settled_structure_len: usize,
}

impl FocusedTranscript {
    pub fn new(
        factory: ReaderFactory,
        seed_epoch: ReconcileEpoch,
        focus_generation: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_id = factory.session_id().clone();
        let weak = cx.weak_entity();
        let reader = ReaderWorkerHandle::spawn(factory, weak, cx);
        Self::new_with_reader(reader, session_id, seed_epoch, focus_generation, cx)
    }

    pub(crate) fn new_with_reader(
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
            rows: RowStore::new(),
            list_state: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW),
            pending_finalize: HashMap::new(),
            pending_item_ids: HashMap::new(),
            markers: Vec::new(),
            focus_generation,
            reader,
            reader_error: None,
            session_id,
            baseline_epoch: seed_epoch.epoch,
            baseline_reconcile_in_flight: seed_epoch.in_flight,
            live_section_projection_count: 0,
            settled_structure_len: 0,
        };
        replica.enqueue_read(ReadRange::All, Priority::Baseline, focus_generation);
        cx.notify();
        replica
    }

    #[doc(hidden)]
    pub fn new_test_no_baseline(
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
            rows: RowStore::new(),
            list_state: ListState::new(0, ListAlignment::Bottom, LIST_OVERDRAW),
            pending_finalize: HashMap::new(),
            pending_item_ids: HashMap::new(),
            markers: Vec::new(),
            focus_generation,
            reader,
            reader_error: None,
            session_id,
            baseline_epoch: seed_epoch.epoch,
            baseline_reconcile_in_flight: seed_epoch.in_flight,
            live_section_projection_count: 0,
            settled_structure_len: 0,
        };
        cx.notify();
        replica
    }

    pub fn fold_detailed(&mut self, u: StreamUpdate, cx: &mut Context<Self>) {
        let mut dirty = false;
        match u {
            StreamUpdate::Rebased(state) => {
                self.active_response = state.active_response.clone();
                self.recompute_live_section_start();
                self.reproject(true, cx);
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
                self.recompute_settled_prefix();
                self.apply_expansion_flags(cx);
                self.reproject(false, cx);
                dirty = true;
            }
            StreamUpdate::ScratchChanged(scratch) => {
                self.scratch = scratch;
                self.reproject(false, cx);
                dirty = true;
            }
            StreamUpdate::Retired {
                acc_id,
                disposition,
            } => {
                self.handle_retired(acc_id, disposition, cx);
                dirty = true;
            }
            StreamUpdate::StatusChanged(_)
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
        self.reader_error = None;
        let full_replace = matches!(range, ReadRange::All);
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
        self.recompute_settled_prefix();
        self.commit_pending_disk_rows(&read.rows, cx);
        if full_replace {
            self.settled_structure_len = 0;
            self.reproject(true, cx);
        } else {
            self.apply_expansion_flags(cx);
            self.reproject(false, cx);
        }
        cx.notify();
    }

    pub(crate) fn on_read_error(&mut self, generation: u64, err: String, cx: &mut Context<Self>) {
        if generation != self.focus_generation {
            return;
        }
        self.reader_error = Some(err);
        cx.notify();
    }

    pub(crate) fn on_reader_fatal(&mut self, err: String, cx: &mut Context<Self>) {
        self.reader_error = Some(err);
        // Staged rows in pending_finalize stay visible — recovery path, not orphan.
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn reader_handle(&self) -> ReaderWorkerHandle {
        self.reader.clone()
    }

    #[cfg(test)]
    pub(crate) fn reader_error(&self) -> Option<&str> {
        self.reader_error.as_deref()
    }

    #[doc(hidden)]
    pub fn rows(&self) -> &RowStore {
        &self.rows
    }

    #[cfg(test)]
    #[allow(dead_code)] // used by Task 13 view tests + the real-window probe
    pub(crate) fn list_state(&self) -> &ListState {
        &self.list_state
    }

    #[cfg(test)]
    pub(crate) fn pending_finalize_len(&self) -> usize {
        self.pending_finalize.len()
    }

    #[cfg(test)]
    pub(crate) fn section_expanded(&self, response_id: &ResponseId) -> bool {
        self.rows.section_expanded(response_id)
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

    fn handle_retired(
        &mut self,
        acc_id: AccId,
        disposition: RetireDisposition,
        cx: &mut Context<Self>,
    ) {
        match disposition {
            RetireDisposition::Finalizing { item_id } => {
                self.pending_item_ids.insert(item_id, acc_id.clone());
                let pres =
                    self.stream_presentation(&acc_id, cx)
                        .unwrap_or_else(|| RowPresentation {
                            kind: RowKind::StreamingMessage,
                            text: String::new(),
                            collapsed: false,
                            height_hint: None,
                        });
                self.pending_finalize.insert(acc_id.clone(), pres.clone());
                self.rows.stage_stream_finalize(&acc_id, pres, cx);
            }
            RetireDisposition::Discarded => {
                self.pending_finalize.remove(&acc_id);
                self.pending_item_ids.retain(|_, a| a != &acc_id);
                self.rows
                    .discard_stream_tail(&acc_id, Some(&self.list_state), cx);
            }
        }
        let prev_len = self.rows.len();
        self.rows.sync_list_count(&self.list_state, prev_len);
    }

    fn stream_presentation(&self, acc_id: &AccId, cx: &Context<Self>) -> Option<RowPresentation> {
        let id = RowId::StreamTail(acc_id.clone());
        self.rows
            .entity(&id)
            .map(|e| e.read(cx).presentation.clone())
    }

    fn commit_pending_disk_rows(&mut self, rows: &[(i64, Item)], cx: &mut Context<Self>) {
        for (_, item) in rows {
            let Some(acc_id) = self.pending_item_ids.remove(&item.id) else {
                continue;
            };
            let as_sibling = matches!(
                item.kind,
                ItemKind::Message {
                    role: Role::User,
                    ..
                } | ItemKind::Message {
                    role: Role::Assistant,
                    ..
                } | ItemKind::ResourceEvent { .. }
            );
            let pres = if as_sibling {
                rowsource::presentation_for_item(item)
            } else {
                rowsource::presentation_for_work_item(item)
            };
            self.rows.commit_stream_finalize(
                &acc_id,
                &item.id,
                pres,
                as_sibling,
                Some(&self.list_state),
                cx,
            );
            self.pending_finalize.remove(&acc_id);
        }
    }

    fn apply_expansion_flags(&mut self, cx: &mut Context<Self>) {
        let flags = self.compute_expansion_flags();
        self.rows
            .set_all_response_expansion(&flags, Some(&self.list_state));
        let _ = cx;
    }

    fn compute_expansion_flags(&self) -> HashMap<ResponseId, bool> {
        let latest_settled = self.latest_settled_before_next_user();
        let mut flags = HashMap::new();
        for item in &self.items {
            if let Some(r) = &item.ctx.response_id {
                flags.entry(r.clone()).or_insert_with(|| {
                    self.active_response.as_ref() == Some(r) || latest_settled.as_ref() == Some(r)
                });
            }
        }
        if let Some(active) = &self.active_response {
            flags.insert(active.clone(), true);
        }
        flags
    }

    fn latest_settled_before_next_user(&self) -> Option<ResponseId> {
        let active = self.active_response.as_ref();
        let last_user_idx = self
            .items
            .iter()
            .enumerate()
            .rposition(|(_, item)| is_user_message(item));

        let mut last_idx_per_resp: HashMap<&ResponseId, usize> = HashMap::new();
        for (i, item) in self.items.iter().enumerate() {
            if let Some(r) = &item.ctx.response_id {
                last_idx_per_resp.insert(r, i);
            }
        }

        let mut best: Option<(ResponseId, usize)> = None;
        for (resp, &idx) in &last_idx_per_resp {
            if active == Some(*resp) {
                continue;
            }
            if let Some(u_idx) = last_user_idx
                && u_idx > idx
            {
                continue;
            }
            if best.as_ref().is_none_or(|(_, b)| idx > *b) {
                best = Some(((*resp).clone(), idx));
            }
        }
        best.map(|(r, _)| r)
    }

    fn recompute_settled_prefix(&mut self) {
        let slice = &self.items[..self.live_section_start.min(self.items.len())];
        let refs: Vec<&Item> = slice.iter().collect();
        let empty_scratch = StreamScratch::default();
        let blocks = group_work_section(
            project(&refs, &empty_scratch, self.active_response.as_ref()),
            self.active_response.as_ref(),
        );
        self.settled_structure_len = blocks.len();
    }

    fn reproject(&mut self, full: bool, cx: &mut Context<Self>) {
        let expansion = self.compute_expansion_flags();
        let prev_len = self.rows.len();
        self.rows.set_all_response_expansion(&expansion, None);

        if full {
            let refs: Vec<&Item> = self.items.iter().collect();
            let blocks = group_work_section(
                project(&refs, self.scratch.as_ref(), self.active_response.as_ref()),
                self.active_response.as_ref(),
            );
            RowStore::materialize_full(&blocks, &mut self.rows, cx);
            self.settled_structure_len = self.rows.structure_len();
        } else {
            let prefix = self.settled_structure_len;
            let slice = &self.items[self.live_section_start..];
            let refs: Vec<&Item> = slice.iter().collect();
            let blocks = group_work_section(
                project(&refs, self.scratch.as_ref(), self.active_response.as_ref()),
                self.active_response.as_ref(),
            );
            RowStore::materialize_live_tail(prefix, &blocks, &mut self.rows, cx);
        }

        self.overlay_pending_finalize(cx);
        self.rows.sync_list_count(&self.list_state, prev_len);
        let slice = &self.items[self.live_section_start..];
        let refs: Vec<&Item> = slice.iter().collect();
        self.live_section_projection_count =
            project(&refs, self.scratch.as_ref(), self.active_response.as_ref()).len();
    }

    fn overlay_pending_finalize(&mut self, cx: &mut Context<Self>) {
        let pending: Vec<_> = self
            .pending_finalize
            .iter()
            .map(|(a, p)| (a.clone(), p.clone()))
            .collect();
        for (acc_id, pres) in pending {
            self.rows.ensure_stream_tail_visible(&acc_id, pres, cx);
        }
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

fn is_user_message(item: &Item) -> bool {
    matches!(
        item.kind,
        ItemKind::Message {
            role: Role::User,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_channel::Receiver;
    use gpui::AppContext;
    use lens_core::domain::ids::{AgentId, ConnectionId, SessionId as Sid};
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

    fn user_item(id: &str, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: None,
            },
            created_at: 1,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }

    fn reasoning_item(id: &str, resp: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: Some(ResponseId::new(resp)),
            },
            created_at: 1,
            kind: ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: String::new(),
                encrypted: false,
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

    fn range_read(rows: Vec<(i64, Item)>, watermark: i64) -> RangeRead {
        RangeRead {
            rows,
            skipped: vec![],
            watermark: Some(watermark),
        }
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
            });
            replica.update(cx, |r, cx| {
                r.reproject(false, cx);
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
            assert_eq!(r.items.len(), 2);
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

    #[gpui::test]
    async fn discarded_drops_stream_tail_no_ghost(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");
        let acc = AccId::new("acc_drop");
        let resp = ResponseId::new("resp_a");

        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.active_response = Some(resp.clone());
                r.live_section_start = 0;
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ScratchChanged(Arc::new(StreamScratch {
                        open_message: Some(lens_core::domain::item::MessageAcc {
                            acc_id: acc.clone(),
                            message_id: None,
                            text: "partial".into(),
                            block_index: 0,
                        }),
                        ..Default::default()
                    })),
                    cx,
                );
            });
        });
        let rows_before = cx.read(|cx| replica.read(cx).rows().len());

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::Retired {
                        acc_id: acc.clone(),
                        disposition: RetireDisposition::Discarded,
                    },
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.pending_finalize_len(), 0);
            assert!(
                r.rows().len() <= rows_before,
                "discarded stream tail must not leave a ghost row"
            );
        });
        assert!(rx.try_recv().is_err());
    }

    #[gpui::test]
    async fn collapse_timing_latest_settled_until_next_user(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let resp_a = ResponseId::new("resp_a");
        let resp_b = ResponseId::new("resp_b");
        let resp_c = ResponseId::new("resp_c");

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::All,
                    range_read(
                        vec![
                            (0, reasoning_item("r_a", "resp_a")),
                            (1, user_item("u1", "hi")),
                            (2, reasoning_item("r_b0", "resp_b")),
                            (3, reasoning_item("r_b1", "resp_b")),
                        ],
                        3,
                    ),
                    cx,
                );
                r.active_response = Some(resp_c.clone());
                r.recompute_live_section_start();
                r.apply_expansion_flags(cx);
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert!(
                r.section_expanded(&resp_b),
                "latest settled before next user must stay expanded"
            );
            assert!(
                !r.section_expanded(&resp_a),
                "older settled turn must be collapsed"
            );
        });

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::Delta {
                        after: 3,
                        through: 4,
                    },
                    range_read(vec![(4, user_item("u2", "next"))], 4),
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert!(
                !r.section_expanded(&resp_b),
                "resp_b must collapse after the next user message"
            );
        });
    }

    #[gpui::test]
    async fn collapse_timing_two_runs_fold_together(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let resp = ResponseId::new("resp_a");
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::All,
                    range_read(
                        vec![
                            (0, reasoning_item("r0", "resp_a")),
                            (1, user_item("u1", "break")),
                            (2, reasoning_item("r1", "resp_a")),
                        ],
                        2,
                    ),
                    cx,
                );
                r.active_response = Some(ResponseId::new("resp_live"));
                r.recompute_live_section_start();
                r.apply_expansion_flags(cx);
            });
        });

        let before_user = cx.read(|cx| {
            let r = replica.read(cx);
            (r.section_expanded(&resp), r.rows().len())
        });

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::Delta {
                        after: 2,
                        through: 3,
                    },
                    range_read(vec![(3, user_item("u2", "next"))], 3),
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert!(
                !r.section_expanded(&resp),
                "both runs of resp_a must collapse together on next user message"
            );
            assert!(
                r.rows().len() < before_user.1,
                "collapsed runs shrink row count"
            );
        });
    }

    #[gpui::test]
    async fn staged_finalize_swaps_on_disk_row(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let acc = AccId::new("acc_fin");
        let item_id = ItemId::new("msg_local_0");
        let resp = ResponseId::new("resp_a");

        let entity_before = cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.active_response = Some(resp.clone());
                r.live_section_start = 0;
            });
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ScratchChanged(Arc::new(StreamScratch {
                        open_message: Some(lens_core::domain::item::MessageAcc {
                            acc_id: acc.clone(),
                            message_id: None,
                            text: "streaming text".into(),
                            block_index: 0,
                        }),
                        ..Default::default()
                    })),
                    cx,
                );
            });
            let tail = RowId::StreamTail(acc.clone());
            replica.read(cx).rows().entity_id(&tail, cx)
        });

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::Retired {
                        acc_id: acc.clone(),
                        disposition: RetireDisposition::Finalizing {
                            item_id: item_id.clone(),
                        },
                    },
                    cx,
                );
                // Reducer clears scratch in the same batch as Retired{Finalizing}.
                r.fold_detailed(
                    StreamUpdate::ScratchChanged(Arc::new(StreamScratch::default())),
                    cx,
                );
            });
        });

        let count_after_retire = cx.read(|cx| replica.read(cx).rows().len());
        assert!(count_after_retire > 0);

        let finalized = message_item("msg_local_0", Some("resp_a"));
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.apply_read(
                    1,
                    ReadRange::Delta {
                        after: -1,
                        through: 0,
                    },
                    range_read(vec![(0, finalized)], 0),
                    cx,
                );
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.pending_finalize_len(), 0);
            assert_eq!(r.rows().len(), count_after_retire, "row count must not dip");
            let sibling = RowId::Sibling(item_id.clone());
            let entity_after = r.rows().entity_id(&sibling, cx);
            assert_eq!(
                entity_before, entity_after,
                "finalize must preserve EntityId (acc_id != item_id)"
            );
            assert_eq!(
                r.rows()
                    .entity(&sibling)
                    .unwrap()
                    .read(cx)
                    .presentation
                    .text,
                "hi"
            );
        });
    }

    #[gpui::test]
    async fn fatal_after_finalizing_keeps_staged_row(cx: &mut gpui::TestAppContext) {
        let (replica, rx) = new_replica(cx, ReconcileEpoch::default(), 1);
        let _ = rx.try_recv().expect("baseline");

        let acc = AccId::new("acc_fatal");
        let item_id = ItemId::new("msg_local_1");

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.fold_detailed(
                    StreamUpdate::ScratchChanged(Arc::new(StreamScratch {
                        open_message: Some(lens_core::domain::item::MessageAcc {
                            acc_id: acc.clone(),
                            message_id: None,
                            text: "keep me".into(),
                            block_index: 0,
                        }),
                        ..Default::default()
                    })),
                    cx,
                );
                r.fold_detailed(
                    StreamUpdate::Retired {
                        acc_id: acc.clone(),
                        disposition: RetireDisposition::Finalizing { item_id },
                    },
                    cx,
                );
            });
        });

        let staged_count = cx.read(|cx| replica.read(cx).rows().len());

        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                r.on_reader_fatal("disk write failed".into(), cx);
            });
        });

        cx.read(|cx| {
            let r = replica.read(cx);
            assert_eq!(r.pending_finalize_len(), 1, "staged row must survive Fatal");
            assert_eq!(r.rows().len(), staged_count, "no ghost drop on Fatal");
            assert_eq!(r.reader_error(), Some("disk write failed"));
        });
        assert!(rx.try_recv().is_err());
    }
}
