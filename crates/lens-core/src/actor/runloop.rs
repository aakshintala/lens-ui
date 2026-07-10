//! The actor run-loop: `crossbeam::Select` over events + commands, greedy drain,
//! persist write-through, coalesce, emit to the foreground bridge.

use crate::actor::api::{CommandOutcome, SessionApi};
use crate::actor::outcome::ActorOutcome;
use crate::actor::summary::SummaryUpdate;
use crate::actor::transport::{ActorTransport, ParkReason};
use crate::clock::Clock;
use crate::domain::SessionState;
use crate::domain::controls::PendingUserMessage;
use crate::persist::{ControlStore, TranscriptStore};
use crate::reduce::{StreamUpdate, Updates, reduce};
use crossbeam_channel::{Receiver, Select};
use lens_client::sessions::SessionEventInput;
use lens_client::stream::DisconnectReason;
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
    /// Optimistic user message. Actor wraps plain text into `SessionEventInput::Message`.
    Send {
        text: String,
        model_override: Option<String>,
    },
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
    pub outcomes: async_channel::Receiver<ActorOutcome>,
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

    #[cfg(test)]
    pub fn join_without_stop(self) {
        self.join
            .join()
            .expect("actor thread panicked or was poisoned");
    }
}

/// Bridge senders + output granularity for the actor run-loop.
struct ActorOutput {
    updates: async_channel::Sender<StreamUpdate>,
    summaries: async_channel::Sender<SummaryUpdate>,
    outcomes: async_channel::Sender<ActorOutcome>,
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
    api: Box<dyn SessionApi + Send>,
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
        api,
    )
}

/// Spawn the actor thread with explicit output mode and both bridge senders.
#[allow(clippy::too_many_arguments)]
pub fn spawn_actor_dual(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    summaries: async_channel::Sender<SummaryUpdate>,
    mode: OutputMode,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> ActorHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<SessionCommand>(64);
    let (out_tx, out_rx) = async_channel::bounded(64);
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
                    outcomes: out_tx,
                    mode,
                },
                stores,
                clock,
                api,
            )
        })
        .expect("actor thread");
    ActorHandle {
        commands: cmd_tx,
        outcomes: out_rx,
        join,
    }
}

#[allow(unused_assignments)] // actor-owned transport/reconcile_in_flight persist for P3-3 quiescence gate
fn run(
    mut state: SessionState,
    events: Receiver<ServerStreamEvent>,
    commands: Receiver<SessionCommand>,
    mut output: ActorOutput,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) {
    // Seed past any lens-local ids already resident (reconnect carries pending_user
    // over — D16 KEEP path), so respawn cannot re-mint a colliding lens_pend_N.
    let mut send_seq: u64 = state
        .pending_user
        .iter()
        .filter_map(|p| {
            p.pending_id
                .strip_prefix("lens_pend_")
                .and_then(|s| s.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    let mut transport = ActorTransport::Connected;
    let mut reconcile_in_flight = false;
    let mut parked = false;
    loop {
        let mut sel = Select::new();
        let ev_idx = if parked {
            None
        } else {
            Some(sel.recv(&events))
        };
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
                Ok(SessionCommand::Send {
                    text,
                    model_override,
                }) => {
                    send_seq += 1;
                    let lens_pending_id = format!("lens_pend_{send_seq}");
                    state.pending_user.push(PendingUserMessage {
                        pending_id: lens_pending_id.clone(),
                        server_pending_id: None,
                        store_item_id: None,
                        content: text.clone(),
                        created_at: clock.now_millis(),
                    });
                    if !emit_pending_user(&output, &state) {
                        return;
                    }

                    let evt = SessionEventInput::Message {
                        content: vec![serde_json::json!({"type":"input_text","text": text})],
                        model_override,
                        tools: None,
                    };
                    match api.send_event(&state.id, &evt) {
                        Ok(ack) if ack.denied => {
                            rollback_pending(&mut state, &lens_pending_id);
                            if !emit_pending_user(&output, &state) {
                                return;
                            }
                            let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                                CommandOutcome::SendDenied {
                                    lens_pending_id,
                                    reason: ack.reason,
                                },
                            ));
                        }
                        Ok(ack) => {
                            let p = state
                                .pending_user
                                .iter_mut()
                                .find(|p| p.pending_id == lens_pending_id)
                                .expect("optimistic bubble present for stamp");
                            // Stamp whichever id is present — NEVER branch on harness/native.
                            p.server_pending_id = ack.pending_id.clone();
                            p.store_item_id = ack.item_id.clone();
                            if !emit_pending_user(&output, &state) {
                                return;
                            }
                            let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                                CommandOutcome::SendAccepted {
                                    lens_pending_id,
                                    ack,
                                },
                            ));
                        }
                        Err(e @ lens_client::error::ClientError::Network(_)) => {
                            rollback_pending(&mut state, &lens_pending_id);
                            if !emit_pending_user(&output, &state) {
                                return;
                            }
                            let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                                CommandOutcome::SendFailed {
                                    lens_pending_id,
                                    error: e.to_string(),
                                },
                            ));
                        }
                        Err(e) => {
                            // TODO(P3-2 Task 9): refine per D18 Table B (Auth hold, NotFound tombstone, Server 5xx transient).
                            let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                                CommandOutcome::SendFailed {
                                    lens_pending_id,
                                    error: e.to_string(),
                                },
                            ));
                        }
                    }
                }
            },
            i if ev_idx == Some(i) => match oper.recv(&events) {
                Ok(event) => {
                    let mut batch = reduce(&mut state, &event, clock.as_ref());
                    while let Ok(next) = events.try_recv() {
                        batch.extend(reduce(&mut state, &next, clock.as_ref()));
                    }
                    persist_write_through(&stores, &state, &batch, clock.now_millis());
                    let disconnect_reason = batch.iter().find_map(|u| match u {
                        StreamUpdate::Disconnected(reason) => Some(*reason),
                        _ => None,
                    });
                    let saw_reconnecting = batch
                        .iter()
                        .any(|u| matches!(u, StreamUpdate::Reconnecting { .. }));
                    let saw_reconnected =
                        batch.iter().any(|u| matches!(u, StreamUpdate::Reconnected));
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
                            // Missing summary consumer (Detailed-only spawn_actor, summary rx dropped)
                            // is non-fatal: Demote becomes a mode flip with no listener, and the actor
                            // stays alive to accept Stop. TODO(P3-2 Task 9): push
                            // ActorOutcome::SummaryConsumerGone to the introspection ring instead of dropping.
                            let _ = output
                                .summaries
                                .send_blocking(SummaryUpdate::from_state(&state));
                        }
                    }
                    if let Some(reason) = disconnect_reason {
                        match reason {
                            DisconnectReason::Unauthorized => {
                                transport = ActorTransport::Parked {
                                    reason: ParkReason::Unauthorized,
                                };
                                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                                    reason: ParkReason::Unauthorized,
                                });
                                parked = true;
                            }
                            DisconnectReason::SessionFailed => {
                                transport = ActorTransport::Parked {
                                    reason: ParkReason::SessionFailed,
                                };
                                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                                    reason: ParkReason::SessionFailed,
                                });
                                parked = true;
                            }
                            DisconnectReason::RetriesExhausted => {
                                transport = ActorTransport::Parked {
                                    reason: ParkReason::RetriesExhausted,
                                };
                                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                                    reason: ParkReason::RetriesExhausted,
                                });
                                parked = true;
                            }
                            DisconnectReason::Forbidden => {
                                let _ = output.outcomes.send_blocking(ActorOutcome::StoppedRemoved);
                                break;
                            }
                            DisconnectReason::NotFound => {
                                // TODO(P3-3): SessionLifecycle::Deleted disk write + wake/tombstone schema
                                let _ = output
                                    .outcomes
                                    .send_blocking(ActorOutcome::StoppedTombstone);
                                break;
                            }
                        }
                    }
                    if saw_reconnecting {
                        transport = ActorTransport::Reconnecting;
                        reconcile_in_flight = true;
                        let _ = output
                            .outcomes
                            .send_blocking(ActorOutcome::TransportChanged {
                                transport,
                                reconcile_in_flight,
                            });
                    }
                    if saw_reconnected {
                        transport = ActorTransport::Connected;
                        reconcile_in_flight = false;
                        let _ = output
                            .outcomes
                            .send_blocking(ActorOutcome::TransportChanged {
                                transport,
                                reconcile_in_flight,
                            });
                    }
                }
                Err(_) => break,
            },
            _ => unreachable!(),
        }
    }
}

/// Emit `pending_user` to the foreground bridge (mode-aware, mirrors the event arm).
fn emit_pending_user(output: &ActorOutput, state: &SessionState) -> bool {
    match output.mode {
        OutputMode::Detailed => output
            .updates
            .send_blocking(StreamUpdate::PendingUserChanged(state.pending_user.clone()))
            .is_ok(),
        OutputMode::Summary => {
            // Summary is not a pending_user replica; the optimistic bubble surfaces on Promote/Rebased.
            let _ = output
                .summaries
                .send_blocking(SummaryUpdate::from_state(state));
            true
        }
    }
}

fn rollback_pending(state: &mut SessionState, lens_pending_id: &str) {
    state
        .pending_user
        .retain(|p| p.pending_id != lens_pending_id);
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
            | StreamUpdate::Reconnected => {}
            StreamUpdate::Disconnected(_) => {}
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
    use crate::actor::api::SessionApi;
    use crate::actor::outcome::ActorOutcome;
    use crate::actor::transport::{ActorTransport, ParkReason};
    use crate::clock::ManualClock;
    use crate::domain::controls::{Elicitation, ElicitationParams, PendingUserMessage};
    use crate::domain::ids::ElicitationId;
    use crate::domain::ids::ItemId;
    use crate::domain::ids::{ConnectionId, SessionId};
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::Role;
    use crate::persist::{
        ConnectionRecord, SqliteControlStore, SqliteTranscriptStore, TranscriptStore,
    };
    use crate::reduce::testutil::{fresh_state, parse_response, snapshot_fixture};
    use lens_client::error::ClientError;
    use lens_client::sessions::{SendEventAck, SessionEventInput};
    use lens_client::stream::{
        DisconnectReason, ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus,
    };
    use smallvec::smallvec;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn test_clock() -> Box<dyn Clock + Send> {
        Box::new(ManualClock::new(1_700_000_000_000))
    }

    /// Scriptable mock — one scripted result per `send_event` call (FIFO).
    struct MockApi {
        script: Mutex<VecDeque<Result<SendEventAck, ClientError>>>,
        last_evt: Mutex<Option<SessionEventInput>>,
    }

    impl MockApi {
        fn succeed_with_ack(ack: SendEventAck) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                script: Mutex::new(VecDeque::from([Ok(ack)])),
                last_evt: Mutex::new(None),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn fail(err: ClientError) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                script: Mutex::new(VecDeque::from([Err(err)])),
                last_evt: Mutex::new(None),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn last_evt(&self) -> Option<SessionEventInput> {
            self.last_evt.lock().unwrap().clone()
        }
    }

    impl SessionApi for Arc<MockApi> {
        fn send_event(
            &self,
            _id: &SessionId,
            evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
            *self.last_evt.lock().unwrap() = Some(evt.clone());
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock send_event called more times than scripted")
        }
    }

    struct PanicApi;

    impl SessionApi for PanicApi {
        fn send_event(
            &self,
            _id: &SessionId,
            _evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
            panic!("send_event not expected in this test");
        }
    }

    fn noop_api() -> Box<dyn SessionApi + Send> {
        Box::new(PanicApi)
    }

    fn expect_pending_user_changed(
        up_rx: &async_channel::Receiver<StreamUpdate>,
    ) -> Vec<PendingUserMessage> {
        match up_rx.recv_blocking().unwrap() {
            StreamUpdate::PendingUserChanged(v) => v,
            other => panic!("expected PendingUserChanged, got {other:?}"),
        }
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
            noop_api(),
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
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

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
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

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
    fn demote_on_detailed_only_handle_does_not_kill_actor() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        handle.commands.send(SessionCommand::Demote).unwrap();
        ev_tx.send(status_running_event()).unwrap();

        // Actor must survive Summary emit with no consumer and still accept Promote.
        handle.commands.send(SessionCommand::Promote).unwrap();
        assert!(matches!(
            up_rx.recv_blocking().unwrap(),
            StreamUpdate::Rebased(_)
        ));
        handle.stop_and_join();
    }

    #[test]
    fn actor_stops_on_command_even_while_idle() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );
        handle.stop_and_join();
    }

    #[test]
    fn send_inserts_optimistic_bubble_then_stamps_item_id_from_ack() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_42".into()),
            pending_id: None,
            ..Default::default()
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: Some("gpt-x".into()),
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&up_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");
        assert_eq!(optimistic[0].content, "hello");
        assert_eq!(optimistic[0].server_pending_id, None);
        assert_eq!(optimistic[0].store_item_id, None);

        let stamped = expect_pending_user_changed(&up_rx);
        assert_eq!(stamped.len(), 1);
        assert_eq!(stamped[0].pending_id, "lens_pend_1");
        assert_eq!(stamped[0].server_pending_id, None);
        assert_eq!(stamped[0].store_item_id.as_deref(), Some("msg_42"));

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendAccepted {
                lens_pending_id,
                ack,
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
                assert_eq!(ack.item_id.as_deref(), Some("msg_42"));
            }
            other => panic!("expected SendAccepted, got {other:?}"),
        }

        match mock.last_evt().expect("mock recorded POST") {
            SessionEventInput::Message {
                content,
                model_override,
                ..
            } => {
                assert_eq!(
                    content,
                    vec![serde_json::json!({"type":"input_text","text":"hello"})]
                );
                assert_eq!(model_override.as_deref(), Some("gpt-x"));
            }
            other => panic!("expected Message POST, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_respawn_seeds_send_seq_past_carried_pending_user() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_99".into()),
            pending_id: None,
            ..Default::default()
        });
        let mut state = fresh_state();
        state.pending_user.push(PendingUserMessage {
            pending_id: "lens_pend_1".into(),
            server_pending_id: None,
            store_item_id: None,
            content: "carried".into(),
            created_at: 1_700_000_000_000,
        });

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(state, ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "new".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&up_rx);
        assert_eq!(optimistic.len(), 2);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");
        assert_eq!(optimistic[0].content, "carried");
        assert_eq!(optimistic[1].pending_id, "lens_pend_2");
        assert_eq!(optimistic[1].content, "new");

        let stamped = expect_pending_user_changed(&up_rx);
        assert_eq!(stamped.len(), 2);
        assert_eq!(stamped[0].pending_id, "lens_pend_1");
        assert_eq!(stamped[0].content, "carried");
        assert_eq!(stamped[1].pending_id, "lens_pend_2");
        assert_eq!(stamped[1].store_item_id.as_deref(), Some("msg_99"));

        handle.stop_and_join();
    }

    #[test]
    fn send_network_error_rolls_back_optimistic_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::network_for_test());
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&up_rx);
        assert_eq!(optimistic.len(), 1);

        let rolled_back = expect_pending_user_changed(&up_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed {
                lens_pending_id, ..
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
            }
            other => panic!("expected SendFailed, got {other:?}"),
        }

        // Actor survives — still processes events after a network rollback.
        ev_tx.send(status_running_event()).unwrap();
        assert!(up_rx.recv_blocking().is_ok());
        handle.stop_and_join();
    }

    #[test]
    fn send_non_network_error_leaves_optimistic_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Auth { status: 401 });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&up_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");

        // No rollback emit — bubble stays resident.
        assert!(up_rx.try_recv().is_err());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed {
                lens_pending_id, ..
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
            }
            other => panic!("expected SendFailed, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_denied_ack_rolls_back_and_reports_reason() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: false,
            denied: true,
            reason: Some("policy".into()),
            ..Default::default()
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "blocked".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&up_rx);
        let rolled_back = expect_pending_user_changed(&up_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendDenied {
                lens_pending_id,
                reason,
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
                assert_eq!(reason.as_deref(), Some("policy"));
            }
            other => panic!("expected SendDenied, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_stamps_whichever_id_is_present_never_assumes_native() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        // Case A: pending_id only → server_pending_id set, store_item_id None.
        {
            let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
                queued: true,
                pending_id: Some("pending_a1b2".into()),
                item_id: None,
                ..Default::default()
            });
            let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
            let (up_tx, up_rx) = async_channel::bounded(64);
            let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);
            handle
                .commands
                .send(SessionCommand::Send {
                    text: "native path".into(),
                    model_override: None,
                })
                .unwrap();
            let _ = expect_pending_user_changed(&up_rx);
            let stamped = expect_pending_user_changed(&up_rx);
            assert_eq!(stamped.len(), 1);
            assert_eq!(
                stamped[0].server_pending_id.as_deref(),
                Some("pending_a1b2")
            );
            assert_eq!(stamped[0].store_item_id, None);
            match handle.outcomes.recv_blocking().unwrap() {
                ActorOutcome::Command(CommandOutcome::SendAccepted {
                    lens_pending_id, ..
                }) => {
                    assert_eq!(lens_pending_id, "lens_pend_1");
                }
                other => panic!("expected SendAccepted, got {other:?}"),
            }
            handle.stop_and_join();
        }

        // Case B: item_id only → store_item_id set, server_pending_id None.
        {
            let stores_b = test_stores(_dir.path());
            seed_connection(&stores_b);
            let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
                queued: true,
                item_id: Some("msg_non_native".into()),
                pending_id: None,
                ..Default::default()
            });
            let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
            let (up_tx, up_rx) = async_channel::bounded(64);
            let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores_b, test_clock(), api);
            handle
                .commands
                .send(SessionCommand::Send {
                    text: "non-native path".into(),
                    model_override: None,
                })
                .unwrap();
            let _ = expect_pending_user_changed(&up_rx);
            let stamped = expect_pending_user_changed(&up_rx);
            assert_eq!(stamped.len(), 1);
            assert_eq!(stamped[0].server_pending_id, None);
            assert_eq!(stamped[0].store_item_id.as_deref(), Some("msg_non_native"));
            match handle.outcomes.recv_blocking().unwrap() {
                ActorOutcome::Command(CommandOutcome::SendAccepted {
                    lens_pending_id, ..
                }) => {
                    assert_eq!(lens_pending_id, "lens_pend_1");
                }
                other => panic!("expected SendAccepted, got {other:?}"),
            }
            handle.stop_and_join();
        }
    }

    #[test]
    fn send_then_input_consumed_clears_optimistic_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_1".into()),
            pending_id: None,
            ..Default::default()
        });
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&up_rx);
        let _ = expect_pending_user_changed(&up_rx);
        let _ = handle.outcomes.recv_blocking().unwrap();

        ev_tx
            .send(ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
                cleared_pending_id: None,
            }))
            .unwrap();

        let cleared = expect_pending_user_changed(&up_rx);
        assert!(cleared.is_empty(), "bubble removed after consumed");

        handle.stop_and_join();
    }

    #[test]
    fn disconnect_unauthorized_parks_actor_still_accepts_stop() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Parked {
                reason: ParkReason::Unauthorized,
            } => {}
            other => panic!("expected Parked Unauthorized, got {other:?}"),
        }

        assert!(matches!(
            up_rx.recv_blocking().unwrap(),
            StreamUpdate::Disconnected(DisconnectReason::Unauthorized)
        ));

        ev_tx.send(status_running_event()).unwrap();
        assert!(
            up_rx.try_recv().is_err(),
            "parked actor must not process further events"
        );

        handle.stop_and_join();
    }

    #[test]
    fn disconnect_forbidden_stops_actor_thread() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Forbidden,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::StoppedRemoved => {}
            other => panic!("expected StoppedRemoved, got {other:?}"),
        }

        handle.join_without_stop();
    }

    #[test]
    fn disconnect_not_found_stops_with_tombstone_outcome() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::NotFound,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::StoppedTombstone => {}
            other => panic!("expected StoppedTombstone, got {other:?}"),
        }

        handle.join_without_stop();
    }

    #[test]
    fn reconnecting_sets_reconcile_in_flight() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            up_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx
            .send(ServerStreamEvent::Reconnecting { attempt: 2 })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::TransportChanged {
                transport: ActorTransport::Reconnecting,
                reconcile_in_flight: true,
            } => {}
            other => panic!("expected TransportChanged Reconnecting, got {other:?}"),
        }

        handle.stop_and_join();
    }
}
