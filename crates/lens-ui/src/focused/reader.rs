//! Serialized background reader worker for `FocusedTranscript` (T-2 §3.3).

use crate::fleet::store::ReaderFactory;
use crate::focused::FocusedTranscript;
use async_channel::{Receiver, Sender};
use gpui::{Context, Task, WeakEntity};
use lens_core::persist::{PersistError, RangeRead, ReadRange, TranscriptReader};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHANNEL_BOUND: usize = 16;
const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);
#[cfg(test)]
const RETRY_BACKOFF_MS: [u64; 5] = [0, 0, 0, 0, 0];
#[cfg(not(test))]
const RETRY_BACKOFF_MS: [u64; 5] = [5, 10, 25, 50, 100];

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

#[derive(Debug)]
enum ReadOutcome {
    Ok(RangeRead),
    Retryable,
    Fatal(String),
}

type ReadFn = Arc<dyn Fn(ReadRange) -> Result<RangeRead, PersistError> + Send + Sync>;

pub struct ReaderWorkerHandle {
    tx: Sender<ReadTarget>,
    _worker: Task<()>,
}

impl Clone for ReaderWorkerHandle {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            _worker: Task::ready(()),
        }
    }
}

impl ReaderWorkerHandle {
    pub fn enqueue(&self, target: ReadTarget) {
        let _ = self.tx.try_send(target);
    }

    pub fn spawn(
        factory: ReaderFactory,
        replica: WeakEntity<FocusedTranscript>,
        cx: &mut Context<FocusedTranscript>,
    ) -> Self {
        match factory.open(BUSY_TIMEOUT) {
            Ok(reader) => {
                let reader = Arc::new(Mutex::new(reader));
                let read_fn: ReadFn = Arc::new(move |range| {
                    reader
                        .lock()
                        .expect("reader mutex poisoned")
                        .read_range(range)
                });
                Self::spawn_with_reader(read_fn, replica, cx)
            }
            Err(err) => {
                let msg = err.to_string();
                let (tx, _rx) = async_channel::bounded(CHANNEL_BOUND);
                let worker = cx.spawn(async move |_this, cx| {
                    let _ = replica.update(cx, |replica, cx| {
                        replica.on_reader_fatal(msg, cx);
                    });
                });
                Self {
                    tx,
                    _worker: worker,
                }
            }
        }
    }

    fn spawn_with_reader(
        read_fn: ReadFn,
        replica: WeakEntity<FocusedTranscript>,
        cx: &mut Context<FocusedTranscript>,
    ) -> Self {
        let (tx, rx) = async_channel::bounded(CHANNEL_BOUND);
        let worker = cx.spawn(async move |_this, cx| {
            run_worker(read_fn, replica, rx, cx).await;
        });
        Self {
            tx,
            _worker: worker,
        }
    }

    /// Test-only: observe exactly what the replica enqueues (no worker).
    #[cfg(test)]
    pub fn new_test() -> (Self, Receiver<ReadTarget>) {
        let (tx, rx) = async_channel::bounded(CHANNEL_BOUND);
        (
            Self {
                tx,
                _worker: Task::ready(()),
            },
            rx,
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_test(
        read_fn: ReadFn,
        replica: WeakEntity<FocusedTranscript>,
        cx: &mut Context<FocusedTranscript>,
    ) -> Self {
        Self::spawn_with_reader(read_fn, replica, cx)
    }
}

struct TargetCoalescer {
    rewrite: Option<ReadTarget>,
    reconcile: Option<ReadTarget>,
    baseline: Option<ReadTarget>,
    delta: Option<ReadTarget>,
}

impl TargetCoalescer {
    fn new() -> Self {
        Self {
            rewrite: None,
            reconcile: None,
            baseline: None,
            delta: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.rewrite.is_none()
            && self.reconcile.is_none()
            && self.baseline.is_none()
            && self.delta.is_none()
    }

    fn insert(&mut self, target: ReadTarget) {
        match target.priority {
            Priority::Rewrite => self.rewrite = Some(target),
            Priority::Reconcile => self.reconcile = Some(target),
            Priority::Baseline => self.baseline = Some(target),
            Priority::Delta => {
                if let Some(existing) = &mut self.delta {
                    existing.range = merge_delta_ranges(&existing.range, &target.range);
                    existing.generation = target.generation;
                } else {
                    self.delta = Some(target);
                }
            }
        }
    }

    fn pop_highest(&mut self) -> Option<ReadTarget> {
        if let Some(target) = self.rewrite.take() {
            return Some(target);
        }
        if let Some(target) = self.reconcile.take() {
            return Some(target);
        }
        if let Some(target) = self.baseline.take() {
            return Some(target);
        }
        self.delta.take()
    }
}

fn merge_delta_ranges(existing: &ReadRange, incoming: &ReadRange) -> ReadRange {
    match (existing, incoming) {
        (
            ReadRange::Delta {
                after: a1,
                through: t1,
            },
            ReadRange::Delta {
                after: a2,
                through: t2,
            },
        ) => ReadRange::Delta {
            after: (*a1).min(*a2),
            through: (*t1).max(*t2),
        },
        _ => *incoming,
    }
}

fn classify_persist_error(err: PersistError) -> ReadOutcome {
    if is_sqlite_busy(&err) {
        ReadOutcome::Retryable
    } else {
        ReadOutcome::Fatal(err.to_string())
    }
}

fn is_sqlite_busy(err: &PersistError) -> bool {
    match err {
        PersistError::Sqlite(e) => matches!(
            e.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        ),
        _ => false,
    }
}

async fn run_worker(
    read_fn: ReadFn,
    replica: WeakEntity<FocusedTranscript>,
    rx: Receiver<ReadTarget>,
    cx: &mut gpui::AsyncApp,
) {
    let mut coalescer = TargetCoalescer::new();
    loop {
        if coalescer.is_empty() {
            match rx.recv().await {
                Ok(target) => coalescer.insert(target),
                Err(_) => break,
            }
        }
        while let Ok(target) = rx.try_recv() {
            coalescer.insert(target);
        }

        let Some(target) = coalescer.pop_highest() else {
            continue;
        };

        let mut attempt = 0usize;
        loop {
            let range = target.range;
            let read_fn = Arc::clone(&read_fn);
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    match read_fn(range) {
                        Ok(read) => ReadOutcome::Ok(read),
                        Err(err) => classify_persist_error(err),
                    }
                })
                .await;

            match outcome {
                ReadOutcome::Ok(read) => {
                    let generation = target.generation;
                    let range = target.range;
                    let _ = replica.update(cx, |replica, cx| {
                        replica.apply_read(generation, range, read, cx);
                    });
                    break;
                }
                ReadOutcome::Retryable => {
                    if attempt >= RETRY_BACKOFF_MS.len() {
                        let generation = target.generation;
                        let _ = replica.update(cx, |replica, cx| {
                            replica.on_read_error(
                                generation,
                                "transcript read busy: retries exhausted".into(),
                                cx,
                            );
                        });
                        break;
                    }
                    let delay = RETRY_BACKOFF_MS[attempt];
                    cx.background_executor()
                        .timer(Duration::from_millis(delay))
                        .await;
                    attempt += 1;
                }
                ReadOutcome::Fatal(message) => {
                    let generation = target.generation;
                    let _ = replica.update(cx, |replica, cx| {
                        replica.on_read_error(generation, message, cx);
                    });
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::store::ReconcileEpoch;
    use crate::focused::FocusedTranscript;
    use gpui::AppContext;
    use lens_core::domain::ids::{ItemId, SessionId};
    use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use lens_core::domain::scalars::Role;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    enum ScriptedOutcome {
        Ok(RangeRead),
        Busy,
        Fatal(String),
    }

    struct FakeReader {
        outcomes: Mutex<VecDeque<ScriptedOutcome>>,
        read_log: Arc<Mutex<Vec<ReadRange>>>,
        busy_attempts: Arc<AtomicUsize>,
    }

    impl FakeReader {
        fn new(outcomes: Vec<ScriptedOutcome>) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into()),
                read_log: Arc::new(Mutex::new(Vec::new())),
                busy_attempts: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn read_log(&self) -> Arc<Mutex<Vec<ReadRange>>> {
            Arc::clone(&self.read_log)
        }

        fn busy_attempts(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.busy_attempts)
        }

        fn read_range(&self, range: ReadRange) -> Result<RangeRead, PersistError> {
            self.read_log.lock().expect("read log lock").push(range);
            let next = self
                .outcomes
                .lock()
                .expect("outcomes lock")
                .pop_front()
                .unwrap_or(ScriptedOutcome::Ok(empty_read()));
            match next {
                ScriptedOutcome::Ok(read) => Ok(read),
                ScriptedOutcome::Busy => {
                    self.busy_attempts.fetch_add(1, Ordering::SeqCst);
                    Err(PersistError::Sqlite(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error {
                            code: rusqlite::ErrorCode::DatabaseBusy,
                            extended_code: rusqlite::ffi::SQLITE_BUSY,
                        },
                        Some("database is locked".into()),
                    )))
                }
                ScriptedOutcome::Fatal(message) => {
                    Err(PersistError::Io(std::io::Error::other(message)))
                }
            }
        }
    }

    fn empty_read() -> RangeRead {
        RangeRead {
            rows: vec![],
            skipped: vec![],
            watermark: None,
        }
    }

    fn message_item(id: &str) -> Item {
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
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hi".into()),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }

    fn spawn_replica_with_fake(
        cx: &mut gpui::TestAppContext,
        fake: Arc<FakeReader>,
        focus_generation: u64,
        enqueue_baseline: bool,
    ) -> (gpui::Entity<FocusedTranscript>, ReaderWorkerHandle) {
        let read_fn: ReadFn = {
            let fake = Arc::clone(&fake);
            Arc::new(move |range| fake.read_range(range))
        };
        let session_id = SessionId::new("sess_reader_test");
        cx.update(|cx| {
            let replica = cx.new(|cx| {
                let weak = cx.weak_entity();
                let reader = ReaderWorkerHandle::spawn_test(read_fn, weak, cx);
                if enqueue_baseline {
                    FocusedTranscript::new_with_reader(
                        reader,
                        session_id,
                        ReconcileEpoch::default(),
                        focus_generation,
                        cx,
                    )
                } else {
                    FocusedTranscript::new_with_reader_no_baseline(
                        reader,
                        session_id,
                        ReconcileEpoch::default(),
                        focus_generation,
                        cx,
                    )
                }
            });
            let reader = replica.read_with(cx, |replica, _| replica.reader_handle());
            (replica, reader)
        })
    }

    fn spawn_and_drain_baseline(
        cx: &mut gpui::TestAppContext,
        fake: Arc<FakeReader>,
        focus_generation: u64,
    ) -> (gpui::Entity<FocusedTranscript>, ReaderWorkerHandle) {
        let (replica, reader) = spawn_replica_with_fake(cx, fake, focus_generation, true);
        cx.run_until_parked();
        (replica, reader)
    }

    #[gpui::test]
    async fn two_deltas_coalesce_to_higher_through(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(empty_read()),
        ]));
        let read_log = fake.read_log();
        let (_replica, reader) = spawn_and_drain_baseline(cx, fake, 1);

        reader.enqueue(ReadTarget {
            range: ReadRange::Delta {
                after: 0,
                through: 3,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        reader.enqueue(ReadTarget {
            range: ReadRange::Delta {
                after: 0,
                through: 7,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        cx.run_until_parked();

        let ranges = read_log.lock().expect("read log lock");
        let deltas: Vec<_> = ranges
            .iter()
            .filter(|range| matches!(range, ReadRange::Delta { .. }))
            .collect();
        assert_eq!(deltas.len(), 1, "coalesced to one delta read");
        assert_eq!(
            deltas[0],
            &ReadRange::Delta {
                after: 0,
                through: 7
            }
        );
    }

    #[gpui::test]
    async fn reconcile_jumps_ahead_of_pending_delta(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(empty_read()),
        ]));
        let read_log = fake.read_log();
        let (_replica, reader) = spawn_and_drain_baseline(cx, fake, 1);

        reader.enqueue(ReadTarget {
            range: ReadRange::Delta {
                after: 0,
                through: 3,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        reader.enqueue(ReadTarget {
            range: ReadRange::All,
            generation: 1,
            priority: Priority::Reconcile,
        });
        cx.run_until_parked();

        let ranges = read_log.lock().expect("read log lock");
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[1], ReadRange::All);
        assert_eq!(
            ranges[2],
            ReadRange::Delta {
                after: 0,
                through: 3
            }
        );
    }

    #[gpui::test]
    async fn baseline_jumps_ahead_of_pending_delta(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(empty_read()),
        ]));
        let read_log = fake.read_log();
        let (_replica, reader) = spawn_and_drain_baseline(cx, fake, 1);

        reader.enqueue(ReadTarget {
            range: ReadRange::Delta {
                after: 1,
                through: 4,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        reader.enqueue(ReadTarget {
            range: ReadRange::All,
            generation: 1,
            priority: Priority::Baseline,
        });
        cx.run_until_parked();

        let ranges = read_log.lock().expect("read log lock");
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[1], ReadRange::All);
        assert_eq!(
            ranges[2],
            ReadRange::Delta {
                after: 1,
                through: 4
            }
        );
    }

    #[gpui::test]
    async fn retryable_reenqueues_same_target_with_bounded_backoff(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Busy,
            ScriptedOutcome::Busy,
            ScriptedOutcome::Ok(RangeRead {
                rows: vec![(0, message_item("ok"))],
                skipped: vec![],
                watermark: Some(0),
            }),
        ]));
        let busy_attempts = fake.busy_attempts();
        let read_log = fake.read_log();
        let (replica, reader) = spawn_replica_with_fake(cx, fake, 1, false);

        reader.enqueue(ReadTarget {
            range: ReadRange::Delta {
                after: 0,
                through: 2,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        cx.run_until_parked();

        assert_eq!(busy_attempts.load(Ordering::SeqCst), 2);
        let ranges = read_log.lock().expect("read log lock");
        assert_eq!(ranges.len(), 3, "three delta attempts");
        assert!(ranges.iter().all(|range| {
            *range
                == ReadRange::Delta {
                    after: 0,
                    through: 2,
                }
        }));
        cx.read(|cx| {
            assert_eq!(replica.read(cx).items.len(), 1);
            assert!(replica.read(cx).reader_error().is_none());
        });
    }

    #[gpui::test]
    async fn fatal_calls_on_read_error_not_apply_read(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Fatal("disk exploded".into()),
        ]));
        let (replica, reader) = spawn_and_drain_baseline(cx, fake, 1);

        reader.enqueue(ReadTarget {
            range: ReadRange::All,
            generation: 1,
            priority: Priority::Baseline,
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let replica = replica.read(cx);
            assert!(replica.items.is_empty());
            assert_eq!(replica.reader_error(), Some("io error: disk exploded"));
        });
    }

    #[gpui::test]
    async fn generation_mismatch_drops_result(cx: &mut gpui::TestAppContext) {
        let fake = Arc::new(FakeReader::new(vec![
            ScriptedOutcome::Ok(empty_read()),
            ScriptedOutcome::Ok(RangeRead {
                rows: vec![(0, message_item("stale"))],
                skipped: vec![],
                watermark: Some(0),
            }),
        ]));
        let (replica, reader) = spawn_and_drain_baseline(cx, fake, 1);

        reader.enqueue(ReadTarget {
            range: ReadRange::All,
            generation: 99,
            priority: Priority::Baseline,
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let replica = replica.read(cx);
            assert!(replica.items.is_empty());
            assert!(replica.reader_error().is_none());
        });
    }

    #[test]
    fn fake_reader_scripts_busy_then_ok() {
        let fake = FakeReader::new(vec![
            ScriptedOutcome::Busy,
            ScriptedOutcome::Busy,
            ScriptedOutcome::Ok(empty_read()),
        ]);
        assert!(is_sqlite_busy(
            &fake
                .read_range(ReadRange::All)
                .expect_err("first read busy")
        ));
        assert!(is_sqlite_busy(
            &fake
                .read_range(ReadRange::All)
                .expect_err("second read busy")
        ));
        assert!(fake.read_range(ReadRange::All).is_ok());
        assert_eq!(fake.busy_attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn coalescer_merges_delta_through() {
        let mut coalescer = TargetCoalescer::new();
        coalescer.insert(ReadTarget {
            range: ReadRange::Delta {
                after: 2,
                through: 5,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        coalescer.insert(ReadTarget {
            range: ReadRange::Delta {
                after: 2,
                through: 9,
            },
            generation: 1,
            priority: Priority::Delta,
        });
        let target = coalescer.pop_highest().expect("delta target");
        assert_eq!(
            target.range,
            ReadRange::Delta {
                after: 2,
                through: 9
            }
        );
    }
}
