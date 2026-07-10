//! The actor run-loop: `crossbeam::Select` over events + commands, greedy drain,
//! persist write-through, coalesce, emit to the foreground bridge.

use crate::actor::summary::SummaryUpdate;
use crate::clock::Clock;
use crate::domain::SessionState;
use crate::persist::{ControlStore, TranscriptStore};
use crate::reduce::{StreamUpdate, Updates, reduce};
use crossbeam_channel::{Receiver, Select};
use lens_client::stream::ServerStreamEvent;
use std::collections::HashSet;
use std::mem::Discriminant;
use std::thread::JoinHandle;

/// Commands to the actor thread.
#[derive(Debug)]
pub enum SessionCommand {
    Stop,
    Promote,
    Demote,
}

/// Persist role stores moved into the actor thread.
pub struct ActorStores {
    pub control: Box<dyn ControlStore + Send>,
    pub transcript: Box<dyn TranscriptStore + Send>,
}

/// Output granularity for the foreground bridge (wired in Task 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Detailed,
    Summary,
}

pub struct ActorHandle {
    pub commands: crossbeam_channel::Sender<SessionCommand>,
    join: JoinHandle<()>,
}

impl ActorHandle {
    /// Send `Stop` and block until the actor thread exits.
    pub fn stop_and_join(self) {
        let _ = self.commands.send(SessionCommand::Stop);
        self.join
            .join()
            .expect("actor thread panicked or was poisoned");
    }
}

/// Bridge senders + output granularity for the actor run-loop.
struct ActorOutput {
    updates: async_channel::Sender<StreamUpdate>,
    summaries: async_channel::Sender<SummaryUpdate>,
    mode: OutputMode,
}

/// Spawn the actor thread in `Detailed` mode (Task 5 API). Summary channel is
/// created internally and dropped — never emitted to in Detailed mode.
pub fn spawn_actor(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
) -> ActorHandle {
    let (sum_tx, _sum_rx) = async_channel::bounded(1);
    spawn_actor_dual(
        state,
        events,
        updates,
        sum_tx,
        OutputMode::Detailed,
        stores,
        clock,
    )
}

/// Spawn the actor thread with explicit output mode and both bridge senders.
pub fn spawn_actor_dual(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    summaries: async_channel::Sender<SummaryUpdate>,
    mode: OutputMode,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
) -> ActorHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<SessionCommand>(64);
    let join = std::thread::Builder::new()
        .name("lens-actor".into())
        .spawn(move || {
            run(
                state,
                events,
                cmd_rx,
                ActorOutput {
                    updates,
                    summaries,
                    mode,
                },
                stores,
                clock,
            )
        })
        .expect("actor thread");
    ActorHandle {
        commands: cmd_tx,
        join,
    }
}

fn run(
    mut state: SessionState,
    events: Receiver<ServerStreamEvent>,
    commands: Receiver<SessionCommand>,
    mut output: ActorOutput,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
) {
    loop {
        let mut sel = Select::new();
        let ev_idx = sel.recv(&events);
        let cmd_idx = sel.recv(&commands);
        let oper = sel.select();
        match oper.index() {
            i if i == cmd_idx => match oper.recv(&commands) {
                Ok(SessionCommand::Stop) | Err(_) => break,
                Ok(SessionCommand::Promote) => {
                    if output
                        .updates
                        .send_blocking(StreamUpdate::Rebased(Box::new(state.clone())))
                        .is_err()
                    {
                        return;
                    }
                    output.mode = OutputMode::Detailed;
                }
                Ok(SessionCommand::Demote) => {
                    output.mode = OutputMode::Summary;
                }
            },
            i if i == ev_idx => match oper.recv(&events) {
                Ok(event) => {
                    let mut batch = reduce(&mut state, &event, clock.as_ref());
                    while let Ok(next) = events.try_recv() {
                        batch.extend(reduce(&mut state, &next, clock.as_ref()));
                    }
                    persist_write_through(&stores, &state, &batch, clock.now_millis());
                    match output.mode {
                        OutputMode::Detailed => {
                            let had_snapshot = batch
                                .iter()
                                .any(|u| matches!(u, StreamUpdate::SnapshotRestored));
                            for u in coalesce(batch) {
                                if output.updates.send_blocking(u).is_err() {
                                    return;
                                }
                            }
                            if had_snapshot {
                                // SnapshotRestored bulk-folds ~20 chrome scalars actor-side with no
                                // per-field delta; a Detailed replica learns them only via a full
                                // baseline. (review I1)
                                if output
                                    .updates
                                    .send_blocking(StreamUpdate::Rebased(Box::new(state.clone())))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        OutputMode::Summary => {
                            if output
                                .summaries
                                .send_blocking(SummaryUpdate::from_state(&state))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
                Err(_) => break,
            },
            _ => unreachable!(),
        }
    }
}

/// Write the deltas of this batch to disk. Items → `TranscriptStore` by ordinal;
/// a scalar/collection change → one control upsert of the whole session row.
fn persist_write_through(stores: &ActorStores, state: &SessionState, batch: &Updates, now_ms: i64) {
    // Appended items occupy the last `appends` slots of state.items, in batch order.
    let appends = batch
        .iter()
        .filter(|u| matches!(u, StreamUpdate::ItemAppended(_)))
        .count();
    let base = state.items.len().saturating_sub(appends);
    let mut append_i = 0usize;
    let mut touched_scalar = false;
    for u in batch {
        match u {
            StreamUpdate::ItemAppended(item) => {
                // TODO(P3-3): replace this positional ordinal with the owned ordinal cursor once byte-window eviction lands (D11).
                let ordinal = (base + append_i) as i64;
                let _ = stores.transcript.upsert_item(ordinal, item.as_ref());
                append_i += 1;
            }
            StreamUpdate::ItemUpdated { index, item } => {
                let _ = stores.transcript.upsert_item(*index as i64, item.as_ref());
            }
            StreamUpdate::ScratchChanged(_)
            | StreamUpdate::ChildSessionChanged
            | StreamUpdate::ResourcesChanged
            | StreamUpdate::Reconnecting { .. }
            | StreamUpdate::Reconnected
            | StreamUpdate::Disconnected => {}
            _ => touched_scalar = true,
        }
    }
    if touched_scalar {
        let _ = stores.control.upsert_session(state, now_ms);
    }
}

/// Drop superseded scratch/scalar deltas within one batch (keep the last of each
/// kind); item deltas always survive (order-significant transcript growth).
fn coalesce(batch: Updates) -> Updates {
    let mut last_non_item: Vec<(Discriminant<StreamUpdate>, usize)> = Vec::new();
    for (i, u) in batch.iter().enumerate() {
        match u {
            StreamUpdate::ItemAppended(_) | StreamUpdate::ItemUpdated { .. } => {}
            _ => {
                let d = std::mem::discriminant(u);
                if let Some(entry) = last_non_item.iter_mut().find(|(k, _)| *k == d) {
                    entry.1 = i;
                } else {
                    last_non_item.push((d, i));
                }
            }
        }
    }
    let keep: HashSet<usize> = last_non_item.into_iter().map(|(_, i)| i).collect();
    batch
        .into_iter()
        .enumerate()
        .filter_map(|(i, u)| match &u {
            StreamUpdate::ItemAppended(_) | StreamUpdate::ItemUpdated { .. } => Some(u),
            _ if keep.contains(&i) => Some(u),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::domain::controls::{Elicitation, ElicitationParams};
    use crate::domain::ids::ElicitationId;
    use crate::domain::ids::ItemId;
    use crate::domain::ids::{ConnectionId, SessionId};
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::Role;
    use crate::persist::{
        ConnectionRecord, SqliteControlStore, SqliteTranscriptStore, TranscriptStore,
    };
    use crate::reduce::testutil::{fresh_state, parse_response, snapshot_fixture};
    use lens_client::stream::{ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus};
    use smallvec::smallvec;
    use std::path::Path;
    use std::sync::Arc;

    fn test_clock() -> Box<dyn Clock + Send> {
        Box::new(ManualClock::new(1_700_000_000_000))
    }

    fn test_stores(dir: &Path) -> ActorStores {
        let control = SqliteControlStore::open(&dir.join("lens.db")).unwrap();
        let transcript = SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        ActorStores {
            control: Box::new(control),
            transcript: Box::new(transcript),
        }
    }

    fn seed_connection(stores: &ActorStores) {
        let _ = stores.control.upsert_connection(&ConnectionRecord {
            id: ConnectionId::new("conn_1"),
            base_url: "http://localhost:8080".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1_700_000_000_000,
        });
    }

    fn one_output_item_done_event() -> ServerStreamEvent {
        parse_response(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"read","arguments":"{}","call_id":"toolu_1","agent":"coder"}}"#,
        )
    }

    fn sample_elicitation() -> Elicitation {
        Elicitation {
            id: ElicitationId::new("e1"),
            target_session_id: SessionId::new("conv_1"),
            params: ElicitationParams {
                mode: "confirm".into(),
                message: "approve?".into(),
                url: None,
                phase: None,
                policy_name: None,
                content_preview: None,
            },
        }
    }

    fn status_running_event() -> ServerStreamEvent {
        ServerStreamEvent::Session(SessionEvent::Status {
            status: WireStatus::Running,
            response_id: None,
            background_task_count: None,
        })
    }

    #[test]
    fn summary_flags_needs_attention_on_pending_elicitation() {
        let mut s = fresh_state();
        s.pending_elicitations.push(sample_elicitation());
        let sum = SummaryUpdate::from_state(&s);
        assert!(sum.needs_attention);
    }

    #[test]
    fn summary_mode_emits_summary_not_detailed_then_promote_rebases() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let (sum_tx, sum_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            up_tx,
            sum_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
        );

        ev_tx.send(status_running_event()).unwrap();
        assert!(matches!(
            sum_rx.recv_blocking().unwrap(),
            SummaryUpdate { .. }
        ));
        assert!(
            up_rx.try_recv().is_err(),
            "no Detailed deltas in Summary mode"
        );

        handle.commands.send(SessionCommand::Promote).unwrap();
        assert!(matches!(
            up_rx.recv_blocking().unwrap(),
            StreamUpdate::Rebased(_)
        ));
        handle.stop_and_join();
    }

    #[test]
    fn actor_reduces_persists_and_emits_on_event() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock());

        ev_tx.send(one_output_item_done_event()).unwrap();
        let update = up_rx.recv_blocking().expect("actor emitted an update");
        assert!(matches!(update, StreamUpdate::ItemAppended(_)));

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &_dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        assert_eq!(reopened.load_items().unwrap().rows.len(), 1);
    }

    fn test_item(id: &str, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 0,
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

    #[test]
    fn batched_appends_persist_at_distinct_ordinals() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);
        let mut state = fresh_state();
        let a = Arc::new(test_item("fc_1", "a"));
        let b = Arc::new(test_item("fc_2", "b"));
        state.items.push(Arc::clone(&a));
        state.items.push(Arc::clone(&b));
        let batch: Updates = smallvec![
            StreamUpdate::ItemAppended(Arc::clone(&a)),
            StreamUpdate::ItemAppended(Arc::clone(&b)),
        ];
        persist_write_through(&stores, &state, &batch, 1_700_000_000_000);
        let rows = stores.transcript.load_items().unwrap().rows;
        assert_eq!(rows.len(), 2, "both batched appends must persist");
        assert_eq!(rows[0].id.as_str(), "fc_1");
        assert_eq!(rows[1].id.as_str(), "fc_2");
    }

    #[test]
    fn detailed_mode_emits_rebased_after_snapshot_restored() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock());

        let snap = snapshot_fixture(serde_json::json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "llm_model": "opus",
            "title": "snapshot title",
            "items": []
        }));
        ev_tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .unwrap();

        let mut saw_snapshot = false;
        let mut saw_rebased = false;
        while let Ok(u) = up_rx.recv_blocking() {
            match &u {
                StreamUpdate::SnapshotRestored => saw_snapshot = true,
                StreamUpdate::Rebased(baseline) => {
                    saw_rebased = true;
                    assert_eq!(baseline.title.as_deref(), Some("snapshot title"));
                    assert_eq!(baseline.llm_model.as_deref(), Some("opus"));
                }
                _ => {}
            }
            if saw_snapshot && saw_rebased {
                break;
            }
        }
        assert!(saw_snapshot, "expected SnapshotRestored delta");
        assert!(
            saw_rebased,
            "Detailed replica must receive Rebased after snapshot fold"
        );
        handle.stop_and_join();
    }

    #[test]
    fn actor_stops_on_command_even_while_idle() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock());
        handle.stop_and_join();
    }
}
