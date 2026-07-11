//! The actor run-loop: `crossbeam::Select` over events + commands, greedy drain,
//! persist write-through, coalesce, emit to the foreground bridge.

use crate::actor::api::{CommandOutcome, SessionApi};
use crate::actor::outcome::{ActorOutcome, OutcomeRing, map_client_error};
use crate::actor::summary::SummaryUpdate;
use crate::actor::transport::{ActorTransport, ParkReason};
use crate::clock::Clock;
use crate::domain::SessionState;
use crate::domain::controls::PendingUserMessage;
use crate::domain::item::{BlockContext, Item};
use crate::domain::scalars::SessionLifecycle;
use crate::persist::{ControlStore, TranscriptStore};
use crate::reduce::map_wire_item;
use crate::reduce::{StreamUpdate, Updates, reduce};
use crossbeam_channel::{Receiver, Select};
use lens_client::sessions::ItemsPage;
use lens_client::sessions::SessionEventInput;
use lens_client::stream::DisconnectReason;
use lens_client::stream::Item as WireItem;
use lens_client::stream::ServerStreamEvent;
use std::thread::JoinHandle;

/// Forward catch-up page size (D19).
const CATCHUP_PAGE: u32 = 200;

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
    /// D21: durable sleep — in-loop quiescence recheck, flush `Slept`, stop actor.
    Sleep,
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

/// §2.3 D21: quiescent ⇔ no transient work ∧ transport live ∧ not mid-reconcile.
fn is_quiesced(
    state: &SessionState,
    transport: &ActorTransport,
    reconcile_in_flight: bool,
) -> bool {
    !state.transient_work_outstanding()
        && matches!(transport, ActorTransport::Connected)
        && !reconcile_in_flight
}

enum LoopControl {
    Continue,
    Break,
}

impl PartialEq for LoopControl {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (LoopControl::Continue, LoopControl::Continue)
                | (LoopControl::Break, LoopControl::Break)
        )
    }
}

enum CatchupResult {
    CaughtUp {
        buffered_events: Vec<ServerStreamEvent>,
        deferred_commands: Vec<SessionCommand>,
    },
    Aborted,
}

/// Map a durable `/items` wire row to a domain `Item` for catch-up persist.
fn wire_to_domain_item(wire: &WireItem, clock: &dyn Clock) -> Option<Item> {
    let (id, kind) = map_wire_item(wire)?;
    Some(Item {
        id,
        seq: None,
        ctx: BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        },
        created_at: clock.now_millis(),
        kind,
    })
}

/// Catch-up upsert: advance `next_ordinal` only on a fresh insert at the passed ordinal
/// (same discipline as `commit_terminal_prefix`).
fn upsert_catchup_item(
    stores: &ActorStores,
    next_ordinal: &mut i64,
    item: &Item,
    ring: &mut OutcomeRing,
) -> bool {
    match stores.transcript.upsert_item(*next_ordinal, item) {
        Ok(stored_ord) => {
            let requested = *next_ordinal;
            if stored_ord == requested {
                *next_ordinal += 1;
                true
            } else {
                false
            }
        }
        Err(e) => {
            ring.push(ActorOutcome::PersistError {
                where_: "transcript.upsert_item",
                message: e.to_string(),
            });
            false
        }
    }
}

/// D19: mode-switched forward catch-up on the actor thread. Drains live events into RAM
/// between pages; honors `Stop` immediately; stashes other commands for post-catch-up replay.
#[allow(clippy::too_many_arguments)]
fn run_catchup(
    api: &dyn SessionApi,
    stores: &ActorStores,
    state: &SessionState,
    next_ordinal: &mut i64,
    events: &Receiver<ServerStreamEvent>,
    commands: &Receiver<SessionCommand>,
    output: &ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
) -> CatchupResult {
    let mut after = stores
        .transcript
        .frontier()
        .ok()
        .flatten()
        .map(|(_, id)| id.to_string());
    let mut buffered_events = Vec::new();
    let mut deferred_commands = Vec::new();

    loop {
        while let Ok(ev) = events.try_recv() {
            buffered_events.push(ev);
        }
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                SessionCommand::Stop => return CatchupResult::Aborted,
                other => deferred_commands.push(other),
            }
        }

        let page = ItemsPage {
            after: after.clone(),
            order: Some("asc".into()),
            before: None,
            limit: Some(CATCHUP_PAGE),
        };
        let list = match api.fetch_items(&state.id, &page) {
            Ok(l) => l,
            Err(e) => {
                ring.push(ActorOutcome::PersistError {
                    where_: "catchup.fetch_items",
                    message: e.to_string(),
                });
                break;
            }
        };

        let mut wrote_any = false;
        for wire in list.items() {
            if let Some(domain) = wire_to_domain_item(wire, clock)
                && upsert_catchup_item(stores, next_ordinal, &domain, ring)
            {
                wrote_any = true;
            }
            after = Some(wire.id().to_string());
        }
        if wrote_any
            && output
                .updates
                .send_blocking(StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: *next_ordinal - 1,
                })
                .is_err()
        {
            return CatchupResult::Aborted;
        }
        if !list.has_more() {
            break;
        }
    }

    CatchupResult::CaughtUp {
        buffered_events,
        deferred_commands,
    }
}

fn emit_transport_changed(
    output: &ActorOutput,
    transport: ActorTransport,
    reconcile_in_flight: bool,
) -> bool {
    output
        .outcomes
        .send_blocking(ActorOutcome::TransportChanged {
            transport,
            reconcile_in_flight,
        })
        .is_ok()
}

/// Set/clear `reconcile_in_flight` around catch-up; emit only when the flag changes.
fn begin_catchup_reconcile(
    reconcile_in_flight: &mut bool,
    output: &ActorOutput,
    transport: ActorTransport,
    emit_transport: bool,
) -> bool {
    if !*reconcile_in_flight {
        *reconcile_in_flight = true;
        if emit_transport {
            return emit_transport_changed(output, transport, true);
        }
    }
    true
}

fn end_catchup_reconcile(
    reconcile_in_flight: &mut bool,
    output: &ActorOutput,
    transport: ActorTransport,
    emit_transport: bool,
) -> bool {
    if *reconcile_in_flight {
        *reconcile_in_flight = false;
        if emit_transport {
            return emit_transport_changed(output, transport, false);
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn handle_command(
    cmd: SessionCommand,
    state: &mut SessionState,
    stores: &ActorStores,
    output: &mut ActorOutput,
    _ring: &mut OutcomeRing,
    clock: &dyn Clock,
    api: &dyn SessionApi,
    transport: &ActorTransport,
    reconcile_in_flight: bool,
    send_seq: &mut u64,
) -> LoopControl {
    match cmd {
        SessionCommand::Stop => LoopControl::Break,
        SessionCommand::Sleep => {
            if !is_quiesced(state, transport, reconcile_in_flight) {
                let _ = output.outcomes.send_blocking(ActorOutcome::SleepDeclined);
                return LoopControl::Continue;
            }
            let prev_lifecycle = state.lifecycle;
            state.lifecycle = SessionLifecycle::Slept;
            let now = clock.now_millis();
            match stores.control.upsert_session(state, now) {
                Ok(()) => {
                    let _ = api.send_event(&state.id, &SessionEventInput::StopSession);
                    let _ = output.outcomes.send_blocking(ActorOutcome::Slept);
                    LoopControl::Break
                }
                Err(e) => {
                    state.lifecycle = prev_lifecycle;
                    let _ = output.outcomes.send_blocking(ActorOutcome::PersistError {
                        where_: "sleep.upsert_session",
                        message: e.to_string(),
                    });
                    let _ = output.outcomes.send_blocking(ActorOutcome::SleepDeclined);
                    LoopControl::Continue
                }
            }
        }
        SessionCommand::Promote => {
            if output
                .updates
                .send_blocking(StreamUpdate::Rebased(scalars_baseline(state)))
                .is_err()
            {
                return LoopControl::Break;
            }
            output.mode = OutputMode::Detailed;
            LoopControl::Continue
        }
        SessionCommand::Demote => {
            output.mode = OutputMode::Summary;
            LoopControl::Continue
        }
        SessionCommand::Send {
            text,
            model_override,
        } => {
            if matches!(transport, ActorTransport::Parked { .. }) {
                let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                    CommandOutcome::SendRejected {
                        reason: "session parked — awaiting re-auth/retry".into(),
                    },
                ));
            } else {
                *send_seq += 1;
                let lens_pending_id = format!("lens_pend_{send_seq}");
                state.pending_user.push(PendingUserMessage {
                    pending_id: lens_pending_id.clone(),
                    server_pending_id: None,
                    store_item_id: None,
                    content: text.clone(),
                    created_at: clock.now_millis(),
                });
                if !emit_pending_user(output, state) {
                    return LoopControl::Break;
                }

                let evt = SessionEventInput::Message {
                    content: vec![serde_json::json!({"type":"input_text","text": text})],
                    model_override,
                    tools: None,
                };
                match api.send_event(&state.id, &evt) {
                    Ok(ack) if ack.denied => {
                        rollback_pending(state, &lens_pending_id);
                        if !emit_pending_user(output, state) {
                            return LoopControl::Break;
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
                        if !emit_pending_user(output, state) {
                            return LoopControl::Break;
                        }
                        let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                            CommandOutcome::SendAccepted {
                                lens_pending_id,
                                ack,
                            },
                        ));
                    }
                    Err(e) => {
                        let m = map_client_error(&e);
                        // Table B (§13.1) rollback: NetworkTransient, LostAccess,
                        // Tombstone, Denied — send definitively won't land.
                        // Hold bubble: ReAuth (401), ServerTransient (5xx),
                        // WrongVersion, DecodeDrift — reached server or reconcile pending.
                        // TODO(§9/P3-3): command-path LostAccess (Auth403) / Tombstone
                        // (NotFound) should escalate to registry removal / tombstone per
                        // design §13.1 Table B. Deferred — the stream's own terminal
                        // Disconnected(Forbidden/NotFound) drives Table A stop today;
                        // the command path only rolls back + SendFailed for now.
                        if m.rolls_back_send() {
                            rollback_pending(state, &lens_pending_id);
                            if !emit_pending_user(output, state) {
                                return LoopControl::Break;
                            }
                        }
                        let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                            CommandOutcome::SendFailed {
                                lens_pending_id,
                                error: e.to_string(),
                            },
                        ));
                    }
                }
            }
            LoopControl::Continue
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_reduced_batch(
    batch: Updates,
    state: &mut SessionState,
    stores: &ActorStores,
    output: &mut ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
    next_ordinal: &mut i64,
    transport: &mut ActorTransport,
    reconcile_in_flight: &mut bool,
    parked: &mut bool,
    defer_transcript_commit: bool,
) -> (LoopControl, bool) {
    persist_scalars(stores, state, &batch, clock.now_millis(), ring);
    let mut batch = batch;
    if !defer_transcript_commit
        && let Some(ord) = commit_terminal_prefix(stores, state, next_ordinal, ring)
    {
        batch.push(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: ord,
        });
    }
    let disconnect_reason = batch.iter().find_map(|u| match u {
        StreamUpdate::Disconnected(reason) => Some(*reason),
        _ => None,
    });
    let saw_reconnecting = batch
        .iter()
        .any(|u| matches!(u, StreamUpdate::Reconnecting { .. }));
    let saw_reconnected = batch.iter().any(|u| matches!(u, StreamUpdate::Reconnected));
    match output.mode {
        OutputMode::Detailed => {
            let had_snapshot = batch
                .iter()
                .any(|u| matches!(u, StreamUpdate::SnapshotRestored));
            for u in coalesce(batch) {
                if output.updates.send_blocking(u).is_err() {
                    return (LoopControl::Break, false);
                }
            }
            if had_snapshot
                && output
                    .updates
                    .send_blocking(StreamUpdate::Rebased(scalars_baseline(state)))
                    .is_err()
            {
                return (LoopControl::Break, false);
            }
        }
        OutputMode::Summary => {
            if output
                .summaries
                .send_blocking(SummaryUpdate::from_state(state))
                .is_err()
            {
                ring.push(ActorOutcome::SummaryConsumerGone);
            }
        }
    }
    drain_outcome_ring(ring, &output.outcomes);
    if let Some(reason) = disconnect_reason {
        match reason {
            DisconnectReason::Unauthorized => {
                *transport = ActorTransport::Parked {
                    reason: ParkReason::Unauthorized,
                };
                *reconcile_in_flight = false;
                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                    reason: ParkReason::Unauthorized,
                });
                *parked = true;
            }
            DisconnectReason::SessionFailed => {
                *transport = ActorTransport::Parked {
                    reason: ParkReason::SessionFailed,
                };
                *reconcile_in_flight = false;
                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                    reason: ParkReason::SessionFailed,
                });
                *parked = true;
            }
            DisconnectReason::RetriesExhausted => {
                *transport = ActorTransport::Parked {
                    reason: ParkReason::RetriesExhausted,
                };
                *reconcile_in_flight = false;
                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                    reason: ParkReason::RetriesExhausted,
                });
                *parked = true;
            }
            DisconnectReason::Forbidden => {
                let _ = output.outcomes.send_blocking(ActorOutcome::StoppedRemoved);
                return (LoopControl::Break, false);
            }
            DisconnectReason::NotFound => {
                let _ = output
                    .outcomes
                    .send_blocking(ActorOutcome::StoppedTombstone);
                return (LoopControl::Break, false);
            }
        }
    }
    if disconnect_reason.is_none() && saw_reconnecting {
        *transport = ActorTransport::Reconnecting;
        *reconcile_in_flight = true;
        if !emit_transport_changed(output, *transport, *reconcile_in_flight) {
            return (LoopControl::Break, false);
        }
    }
    if disconnect_reason.is_none() && saw_reconnected {
        *transport = ActorTransport::Connected;
    }
    (
        LoopControl::Continue,
        disconnect_reason.is_none() && saw_reconnected,
    )
}

fn reconnected_defer_commit(batch: &Updates) -> bool {
    let has_disconnect = batch
        .iter()
        .any(|u| matches!(u, StreamUpdate::Disconnected(_)));
    let saw_reconnected = batch.iter().any(|u| matches!(u, StreamUpdate::Reconnected));
    saw_reconnected && !has_disconnect
}

/// Shared Reconnected path: catch-up (if needed), then commit deferred live tail.
#[allow(clippy::too_many_arguments)]
fn finish_reconnected_catchup(
    needs_catchup: bool,
    defer_transcript_commit: bool,
    api: &dyn SessionApi,
    stores: &ActorStores,
    state: &mut SessionState,
    events: &Receiver<ServerStreamEvent>,
    commands: &Receiver<SessionCommand>,
    output: &mut ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
    next_ordinal: &mut i64,
    transport: &mut ActorTransport,
    reconcile_in_flight: &mut bool,
    parked: &mut bool,
    send_seq: &mut u64,
    emit_transport: bool,
) -> LoopControl {
    if !needs_catchup {
        return LoopControl::Continue;
    }
    if invoke_catchup_and_replay(
        api,
        stores,
        state,
        events,
        commands,
        output,
        ring,
        clock,
        next_ordinal,
        transport,
        reconcile_in_flight,
        parked,
        send_seq,
        emit_transport,
    ) == LoopControl::Break
    {
        return LoopControl::Break;
    }
    if defer_transcript_commit {
        return finish_deferred_transcript_commit(stores, state, next_ordinal, ring, output);
    }
    LoopControl::Continue
}

/// Reduce a buffered event slice and run the Reconnected defer/catch-up/finish path.
#[allow(clippy::too_many_arguments)]
fn replay_buffered_batch(
    buffered_events: &[ServerStreamEvent],
    api: &dyn SessionApi,
    stores: &ActorStores,
    state: &mut SessionState,
    events: &Receiver<ServerStreamEvent>,
    commands: &Receiver<SessionCommand>,
    output: &mut ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
    next_ordinal: &mut i64,
    transport: &mut ActorTransport,
    reconcile_in_flight: &mut bool,
    parked: &mut bool,
    send_seq: &mut u64,
) -> LoopControl {
    let mut batch = reduce(state, &buffered_events[0], clock);
    for ev in &buffered_events[1..] {
        batch.extend(reduce(state, ev, clock));
    }
    let defer_transcript_commit = reconnected_defer_commit(&batch);
    let (ctrl, needs_catchup) = apply_reduced_batch(
        batch,
        state,
        stores,
        output,
        ring,
        clock,
        next_ordinal,
        transport,
        reconcile_in_flight,
        parked,
        defer_transcript_commit,
    );
    if ctrl == LoopControl::Break {
        return LoopControl::Break;
    }
    finish_reconnected_catchup(
        needs_catchup,
        defer_transcript_commit,
        api,
        stores,
        state,
        events,
        commands,
        output,
        ring,
        clock,
        next_ordinal,
        transport,
        reconcile_in_flight,
        parked,
        send_seq,
        true,
    )
}

/// Commit transcript items deferred across a Reconnected batch (live tail lands after catch-up).
fn finish_deferred_transcript_commit(
    stores: &ActorStores,
    state: &mut SessionState,
    next_ordinal: &mut i64,
    ring: &mut OutcomeRing,
    output: &mut ActorOutput,
) -> LoopControl {
    if let Some(ord) = commit_terminal_prefix(stores, state, next_ordinal, ring)
        && output
            .updates
            .send_blocking(StreamUpdate::TranscriptAdvanced {
                committed_ordinal: ord,
            })
            .is_err()
    {
        return LoopControl::Break;
    }
    drain_outcome_ring(ring, &output.outcomes);
    LoopControl::Continue
}

#[allow(clippy::too_many_arguments)]
fn process_main_loop_event(
    event: ServerStreamEvent,
    events: &Receiver<ServerStreamEvent>,
    commands: &Receiver<SessionCommand>,
    api: &dyn SessionApi,
    state: &mut SessionState,
    stores: &ActorStores,
    output: &mut ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
    next_ordinal: &mut i64,
    transport: &mut ActorTransport,
    reconcile_in_flight: &mut bool,
    parked: &mut bool,
    send_seq: &mut u64,
) -> LoopControl {
    let mut batch = reduce(state, &event, clock);
    while let Ok(next) = events.try_recv() {
        batch.extend(reduce(state, &next, clock));
    }
    // D19: live follow-ons in the same greedy-drained batch must not commit before catch-up.
    let defer_transcript_commit = reconnected_defer_commit(&batch);

    let (ctrl, needs_catchup) = apply_reduced_batch(
        batch,
        state,
        stores,
        output,
        ring,
        clock,
        next_ordinal,
        transport,
        reconcile_in_flight,
        parked,
        defer_transcript_commit,
    );
    if ctrl == LoopControl::Break {
        return LoopControl::Break;
    }
    finish_reconnected_catchup(
        needs_catchup,
        defer_transcript_commit,
        api,
        stores,
        state,
        events,
        commands,
        output,
        ring,
        clock,
        next_ordinal,
        transport,
        reconcile_in_flight,
        parked,
        send_seq,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn invoke_catchup_and_replay(
    api: &dyn SessionApi,
    stores: &ActorStores,
    state: &mut SessionState,
    events: &Receiver<ServerStreamEvent>,
    commands: &Receiver<SessionCommand>,
    output: &mut ActorOutput,
    ring: &mut OutcomeRing,
    clock: &dyn Clock,
    next_ordinal: &mut i64,
    transport: &mut ActorTransport,
    reconcile_in_flight: &mut bool,
    parked: &mut bool,
    send_seq: &mut u64,
    emit_transport: bool,
) -> LoopControl {
    if !begin_catchup_reconcile(reconcile_in_flight, output, *transport, emit_transport) {
        return LoopControl::Break;
    }
    let catchup = run_catchup(
        api,
        stores,
        state,
        next_ordinal,
        events,
        commands,
        output,
        ring,
        clock,
    );
    if !end_catchup_reconcile(reconcile_in_flight, output, *transport, emit_transport) {
        return LoopControl::Break;
    }
    drain_outcome_ring(ring, &output.outcomes);
    match catchup {
        CatchupResult::Aborted => LoopControl::Break,
        CatchupResult::CaughtUp {
            buffered_events,
            deferred_commands,
        } => {
            if !buffered_events.is_empty()
                && replay_buffered_batch(
                    &buffered_events,
                    api,
                    stores,
                    state,
                    events,
                    commands,
                    output,
                    ring,
                    clock,
                    next_ordinal,
                    transport,
                    reconcile_in_flight,
                    parked,
                    send_seq,
                ) == LoopControl::Break
            {
                return LoopControl::Break;
            }
            for cmd in deferred_commands {
                if handle_command(
                    cmd,
                    state,
                    stores,
                    output,
                    ring,
                    clock,
                    api,
                    transport,
                    *reconcile_in_flight,
                    send_seq,
                ) == LoopControl::Break
                {
                    return LoopControl::Break;
                }
            }
            LoopControl::Continue
        }
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
    let mut ring = OutcomeRing::with_cap(64);
    // TODO(P3-3b, scaffold-id): frontier seeds next_ordinal by item_id; scaffold fc_* vs store-native id reconciliation is deferred.
    let mut next_ordinal: i64 = match stores.transcript.frontier() {
        Ok(Some((o, _))) => o + 1,
        Ok(None) => 0,
        Err(e) => {
            ring.push(ActorOutcome::PersistError {
                where_: "transcript.frontier",
                message: e.to_string(),
            });
            0
        }
    };
    let mut transport = ActorTransport::Connected;
    let mut reconcile_in_flight = false;
    let mut parked = false;

    if invoke_catchup_and_replay(
        api.as_ref(),
        &stores,
        &mut state,
        &events,
        &commands,
        &mut output,
        &mut ring,
        clock.as_ref(),
        &mut next_ordinal,
        &mut transport,
        &mut reconcile_in_flight,
        &mut parked,
        &mut send_seq,
        false, // pre-loop spawn catch-up: no TransportChanged — Sleep cannot race yet
    ) == LoopControl::Break
    {
        return;
    }

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
                Ok(cmd) => {
                    if handle_command(
                        cmd,
                        &mut state,
                        &stores,
                        &mut output,
                        &mut ring,
                        clock.as_ref(),
                        api.as_ref(),
                        &transport,
                        reconcile_in_flight,
                        &mut send_seq,
                    ) == LoopControl::Break
                    {
                        break;
                    }
                }
                Err(_) => break,
            },
            i if ev_idx == Some(i) => match oper.recv(&events) {
                Ok(event) => {
                    if process_main_loop_event(
                        event,
                        &events,
                        &commands,
                        api.as_ref(),
                        &mut state,
                        &stores,
                        &mut output,
                        &mut ring,
                        clock.as_ref(),
                        &mut next_ordinal,
                        &mut transport,
                        &mut reconcile_in_flight,
                        &mut parked,
                        &mut send_seq,
                    ) == LoopControl::Break
                    {
                        break;
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

/// Non-blocking drain: stop on first full channel, leave remainder for next batch.
fn drain_outcome_ring(ring: &mut OutcomeRing, outcomes: &async_channel::Sender<ActorOutcome>) {
    ring.try_drain(|o| outcomes.try_send(o).is_ok());
}

/// D23: Rebased is scalars-only; baseline items come from disk on promote (deferred UI).
fn scalars_baseline(state: &SessionState) -> Box<SessionState> {
    let mut b = state.clone();
    b.items.clear();
    Box::new(b)
}

/// Scalar/collection persistence only — items are committed via `commit_terminal_prefix`.
fn persist_scalars(
    stores: &ActorStores,
    state: &SessionState,
    batch: &Updates,
    now_ms: i64,
    ring: &mut OutcomeRing,
) {
    let mut touched_scalar = false;
    for u in batch {
        match u {
            StreamUpdate::ScratchChanged(_)
            | StreamUpdate::ChildSessionChanged
            | StreamUpdate::ResourcesChanged
            | StreamUpdate::PendingUserChanged(_)
            | StreamUpdate::Reconnecting { .. }
            | StreamUpdate::Reconnected
            | StreamUpdate::TranscriptAdvanced { .. } => {}
            StreamUpdate::Disconnected(_) => {}
            _ => touched_scalar = true,
        }
    }
    if touched_scalar && let Err(e) = stores.control.upsert_session(state, now_ms) {
        ring.push(ActorOutcome::PersistError {
            where_: "control.upsert_session",
            message: e.to_string(),
        });
    }
}

/// D20/D23: commit the terminal PREFIX of the working set to disk in order,
/// prune it from RAM, advance `next_ordinal`. Returns the new watermark iff ≥1
/// item committed. A non-terminal front item stops the prefix (it and everything
/// after stay above the watermark). A persist error stops the prefix and leaves
/// the item resident for the next batch (no ordinal gap, no RAM loss).
fn commit_terminal_prefix(
    stores: &ActorStores,
    state: &mut SessionState,
    next_ordinal: &mut i64,
    ring: &mut OutcomeRing,
) -> Option<i64> {
    let mut committed = false;
    while let Some(front) = state.items.first() {
        if !front.kind.is_terminal() {
            break;
        }
        // TODO(P3-3b, scaffold-id): scaffold fc_* ids may double-commit vs store-native ids.
        match stores.transcript.upsert_item(*next_ordinal, front) {
            Ok(stored_ord) => {
                let requested = *next_ordinal;
                state.items.remove(0);
                if stored_ord == requested {
                    *next_ordinal += 1;
                    committed = true;
                }
            }
            Err(e) => {
                ring.push(ActorOutcome::PersistError {
                    where_: "transcript.upsert_item",
                    message: e.to_string(),
                });
                break;
            }
        }
    }
    committed.then(|| *next_ordinal - 1)
}

/// Drop superseded deltas within one batch (keep the last of each discriminant).
fn coalesce(batch: Updates) -> Updates {
    let mut last: Vec<(std::mem::Discriminant<StreamUpdate>, usize)> = Vec::new();
    for (i, u) in batch.iter().enumerate() {
        let d = std::mem::discriminant(u);
        if let Some(entry) = last.iter_mut().find(|(k, _)| *k == d) {
            entry.1 = i;
        } else {
            last.push((d, i));
        }
    }
    let keep: std::collections::HashSet<usize> = last.into_iter().map(|(_, i)| i).collect();
    batch
        .into_iter()
        .enumerate()
        .filter_map(|(i, u)| keep.contains(&i).then_some(u))
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
        ConnectionRecord, Loaded, PersistError, SqliteControlStore, SqliteTranscriptStore,
        StoreMode, TranscriptStore,
    };
    use crate::reduce::testutil::{fresh_state, parse_response, snapshot_fixture};
    use lens_client::error::ClientError;
    use lens_client::sessions::{ItemList, SendEventAck, SessionEventInput};
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

    /// Scriptable mock — one scripted result per `send_event` / `fetch_items` call (FIFO).
    struct MockApi {
        send_script: Mutex<VecDeque<Result<SendEventAck, ClientError>>>,
        fetch_script: Mutex<VecDeque<Result<ItemList, ClientError>>>,
        last_evt: Mutex<Option<SessionEventInput>>,
    }

    fn empty_item_list() -> ItemList {
        serde_json::from_str(r#"{"data":[],"has_more":false}"#).expect("empty item list")
    }

    impl MockApi {
        fn succeed_with_ack(ack: SendEventAck) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(VecDeque::from([Ok(ack)])),
                fetch_script: Mutex::new(VecDeque::new()),
                last_evt: Mutex::new(None),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn with_fetch_script(
            fetch_script: VecDeque<Result<ItemList, ClientError>>,
        ) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(VecDeque::new()),
                fetch_script: Mutex::new(fetch_script),
                last_evt: Mutex::new(None),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn fail(err: ClientError) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(VecDeque::from([Err(err)])),
                fetch_script: Mutex::new(VecDeque::new()),
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
            self.send_script
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock send_event called more times than scripted")
        }

        fn fetch_items(
            &self,
            _id: &SessionId,
            _page: &lens_client::sessions::ItemsPage,
        ) -> Result<ItemList, ClientError> {
            self.fetch_script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(empty_item_list()))
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

        fn fetch_items(
            &self,
            _id: &SessionId,
            _page: &lens_client::sessions::ItemsPage,
        ) -> Result<ItemList, ClientError> {
            Ok(empty_item_list())
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

    /// Transcript role that always fails `upsert_item` — persist introspection test stub.
    struct FailingTranscriptStore;

    impl TranscriptStore for FailingTranscriptStore {
        fn mode(&self) -> StoreMode {
            StoreMode::ReadWrite
        }

        fn identity(&self) -> crate::persist::Result<(ConnectionId, SessionId)> {
            Ok((ConnectionId::new("conn_1"), SessionId::new("conv_1")))
        }

        fn upsert_item(&self, _ordinal: i64, _item: &Item) -> crate::persist::Result<i64> {
            Err(PersistError::ReadOnly)
        }

        fn load_items(&self) -> crate::persist::Result<Loaded<Item>> {
            Ok(Loaded {
                rows: vec![],
                skipped: vec![],
            })
        }

        fn reconcile(&self, _items: &[Item]) -> crate::persist::Result<()> {
            Ok(())
        }

        fn frontier(&self) -> crate::persist::Result<Option<(i64, ItemId)>> {
            Ok(None)
        }
    }

    fn failing_transcript_stores(dir: &Path) -> ActorStores {
        let control = SqliteControlStore::open(&dir.join("lens.db")).unwrap();
        ActorStores {
            control: Box::new(control),
            transcript: Box::new(FailingTranscriptStore),
        }
    }

    /// Control role that always fails `upsert_session` — persist introspection test stub.
    struct FailingControlStore {
        inner: SqliteControlStore,
    }

    impl ControlStore for FailingControlStore {
        fn mode(&self) -> StoreMode {
            self.inner.mode()
        }

        fn upsert_connection(&self, c: &ConnectionRecord) -> crate::persist::Result<()> {
            self.inner.upsert_connection(c)
        }

        fn load_connections(&self) -> crate::persist::Result<Vec<ConnectionRecord>> {
            self.inner.load_connections()
        }

        fn upsert_session(&self, _s: &SessionState, _now_ms: i64) -> crate::persist::Result<()> {
            Err(PersistError::ReadOnly)
        }

        fn load_session(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
        ) -> crate::persist::Result<Option<SessionState>> {
            self.inner.load_session(conn, id)
        }

        fn list_sessions(
            &self,
            conn: &ConnectionId,
        ) -> crate::persist::Result<Loaded<SessionState>> {
            self.inner.list_sessions(conn)
        }

        fn insert_cost_sample(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
            sampled_at: i64,
            total_cost_usd: f64,
        ) -> crate::persist::Result<()> {
            self.inner
                .insert_cost_sample(conn, id, sampled_at, total_cost_usd)
        }

        fn cost_samples_in(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
            since: i64,
            until: i64,
        ) -> crate::persist::Result<Vec<(i64, f64)>> {
            self.inner.cost_samples_in(conn, id, since, until)
        }
    }

    fn failing_control_stores(dir: &Path) -> (ActorStores, std::path::PathBuf) {
        let db_path = dir.join("lens.db");
        let inner = SqliteControlStore::open(&db_path).unwrap();
        let transcript = SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        (
            ActorStores {
                control: Box::new(FailingControlStore { inner }),
                transcript: Box::new(transcript),
            },
            db_path,
        )
    }

    /// Counts `upsert_session` calls — persist introspection test stub.
    struct CountingControlStore {
        inner: SqliteControlStore,
        upsert_session_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl CountingControlStore {
        fn open(path: &Path) -> Self {
            Self {
                inner: SqliteControlStore::open(path).unwrap(),
                upsert_session_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }
    }

    impl ControlStore for CountingControlStore {
        fn mode(&self) -> StoreMode {
            self.inner.mode()
        }

        fn upsert_connection(&self, c: &ConnectionRecord) -> crate::persist::Result<()> {
            self.inner.upsert_connection(c)
        }

        fn load_connections(&self) -> crate::persist::Result<Vec<ConnectionRecord>> {
            self.inner.load_connections()
        }

        fn upsert_session(&self, s: &SessionState, now_ms: i64) -> crate::persist::Result<()> {
            self.upsert_session_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.upsert_session(s, now_ms)
        }

        fn load_session(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
        ) -> crate::persist::Result<Option<SessionState>> {
            self.inner.load_session(conn, id)
        }

        fn list_sessions(
            &self,
            conn: &ConnectionId,
        ) -> crate::persist::Result<Loaded<SessionState>> {
            self.inner.list_sessions(conn)
        }

        fn insert_cost_sample(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
            sampled_at: i64,
            total_cost_usd: f64,
        ) -> crate::persist::Result<()> {
            self.inner
                .insert_cost_sample(conn, id, sampled_at, total_cost_usd)
        }

        fn cost_samples_in(
            &self,
            conn: &ConnectionId,
            id: &SessionId,
            since: i64,
            until: i64,
        ) -> crate::persist::Result<Vec<(i64, f64)>> {
            self.inner.cost_samples_in(conn, id, since, until)
        }
    }

    fn counting_stores(dir: &Path) -> (ActorStores, Arc<std::sync::atomic::AtomicUsize>) {
        let control = CountingControlStore::open(&dir.join("lens.db"));
        let calls = Arc::clone(&control.upsert_session_calls);
        let transcript = SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        (
            ActorStores {
                control: Box::new(control),
                transcript: Box::new(transcript),
            },
            calls,
        )
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
    fn persist_error_lands_on_ring_without_blocking_emit() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = failing_transcript_stores(_dir.path());
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
        let _update = up_rx
            .recv_blocking()
            .expect("actor still emitted stream update");

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::PersistError { where_, message } => {
                assert_eq!(where_, "transcript.upsert_item");
                assert!(!message.is_empty());
            }
            other => panic!("expected PersistError, got {other:?}"),
        }

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
        let mut saw_watermark = false;
        while let Ok(update) = up_rx.recv_blocking() {
            if matches!(
                update,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 0
                }
            ) {
                saw_watermark = true;
                break;
            }
        }
        assert!(
            saw_watermark,
            "expected TranscriptAdvanced{{committed_ordinal:0}}"
        );

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &_dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        assert_eq!(reopened.load_items().unwrap().rows.len(), 1);
    }

    #[test]
    fn in_progress_call_blocks_commit_completed_message_advances_watermark() {
        use lens_client::stream::ResponseEvent;

        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
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

        // Terminal assistant message commits at ordinal 0 (must precede a non-terminal
        // front item — prefix commit stops at the first in-progress function call).
        ev_tx
            .send(ServerStreamEvent::Response(
                ResponseEvent::OutputTextDelta {
                    delta: "hello".into(),
                    message_id: None,
                    index: None,
                    last: None,
                },
            ))
            .unwrap();
        ev_tx
            .send(ServerStreamEvent::Response(ResponseEvent::Completed))
            .unwrap();

        let mut saw_watermark = false;
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 0
                }
            ) {
                saw_watermark = true;
                break;
            }
        }
        assert!(
            saw_watermark,
            "expected TranscriptAdvanced{{committed_ordinal:0}}"
        );

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1, "only the terminal message is on disk");
        assert!(matches!(rows[0].kind, ItemKind::Message { .. }));

        // In-progress function call — non-terminal, must NOT commit.
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"toolu_1"}}"#,
            ))
            .unwrap();

        while up_rx.try_recv().is_ok() {}

        let rows = reopened.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1, "fc_1 must stay above the watermark");
        assert!(
            !rows.iter().any(|r| r.id.as_str() == "fc_1"),
            "in-progress fc_1 must not be on disk"
        );

        handle.stop_and_join();
    }

    #[test]
    fn golden_order_dual_id_function_call_commits_in_wire_order() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
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

        // Golden happy_path.stream.sse L38–L50 order (dual-id function_call seam).
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"fc_52f83171c226","type":"function_call","status":"in_progress","name":"sys_os_shell","arguments":"{}","call_id":"toolu_01HijYUkd7fDUELjLrF5bsKy"}}"#,
            ))
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"msg_957e7144","type":"message","role":"assistant","content":[{"type":"output_text","text":"confirmed"}]}}"#,
            ))
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"fc_5a32092a5f02","type":"function_call","status":"completed","name":"sys_os_shell","arguments":"{}","call_id":"toolu_01HijYUkd7fDUELjLrF5bsKy"}}"#,
            ))
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"fco_d56eb811c793","type":"function_call_output","call_id":"toolu_01HijYUkd7fDUELjLrF5bsKy","output":"ok"}}"#,
            ))
            .unwrap();

        let mut saw_final_watermark = false;
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 2
                }
            ) {
                saw_final_watermark = true;
                break;
            }
        }
        assert!(
            saw_final_watermark,
            "FCO commit must advance watermark to ordinal 2"
        );

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["msg_957e7144", "fc_5a32092a5f02", "fco_d56eb811c793"],
            "wire order: message, completed FC, FCO"
        );
        assert!(
            !ids.contains(&"fc_52f83171c226"),
            "in_progress twin must be superseded, not on disk"
        );
    }

    #[test]
    fn refire_of_pruned_item_does_not_gap_ordinals() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
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
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_a","type":"message","role":"assistant","content":[{"type":"output_text","text":"a"}]}}"#,
            ))
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_b","type":"message","role":"assistant","content":[{"type":"output_text","text":"b"}]}}"#,
            ))
            .unwrap();
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 1
                }
            ) {
                break;
            }
        }

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        assert_eq!(reopened.load_items().unwrap().rows.len(), 2);

        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_a","type":"message","role":"assistant","content":[{"type":"output_text","text":"a-refire"}]}}"#,
            ))
            .unwrap();
        let mut saw_watermark_on_refire = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            match up_rx.try_recv() {
                Ok(StreamUpdate::TranscriptAdvanced { .. }) => {
                    saw_watermark_on_refire = true;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        assert!(
            !saw_watermark_on_refire,
            "re-fire of an already-persisted id must not emit TranscriptAdvanced"
        );

        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_c","type":"message","role":"assistant","content":[{"type":"output_text","text":"c"}]}}"#,
            ))
            .unwrap();
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 2
                }
            ) {
                break;
            }
        }

        let rows = reopened.load_items().unwrap().rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id.as_str(), "item_a");
        assert_eq!(rows[1].id.as_str(), "item_b");
        assert_eq!(rows[2].id.as_str(), "item_c");
        match &rows[0].kind {
            ItemKind::Message { content, .. } => {
                assert_eq!(content[0].text.as_deref(), Some("a-refire"));
            }
            other => panic!("{other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn persist_pending_user_changed_only_skips_control_upsert() {
        let _dir = tempfile::tempdir().unwrap();
        let (stores, upsert_calls) = counting_stores(_dir.path());
        seed_connection(&stores);
        let mut state = fresh_state();
        state.pending_user.push(PendingUserMessage {
            pending_id: "lens_pend_1".into(),
            server_pending_id: None,
            store_item_id: None,
            content: "held".into(),
            created_at: 0,
        });
        let batch: Updates =
            smallvec![StreamUpdate::PendingUserChanged(state.pending_user.clone())];
        persist_scalars(
            &stores,
            &state,
            &batch,
            1_700_000_000_000,
            &mut OutcomeRing::with_cap(64),
        );
        assert_eq!(
            upsert_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "PendingUserChanged is RAM-only — must not touch control store"
        );

        let status_batch: Updates = smallvec![StreamUpdate::StatusChanged(
            crate::domain::scalars::SessionStatusValue::Running
        )];
        persist_scalars(
            &stores,
            &state,
            &status_batch,
            1_700_000_000_001,
            &mut OutcomeRing::with_cap(64),
        );
        assert_eq!(
            upsert_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "scalar deltas other than PendingUserChanged still upsert control"
        );
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
                    assert!(baseline.items.is_empty(), "D23: Rebased is scalars-only");
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
        while !matches!(up_rx.recv_blocking().unwrap(), StreamUpdate::Rebased(_)) {}
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
    fn send_auth401_holds_bubble() {
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
    fn send_server5xx_holds_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Server {
            status: 503,
            body: serde_json::json!({}),
        });
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
        assert!(
            up_rx.try_recv().is_err(),
            "5xx keeps bubble — no rollback emit"
        );

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { .. }) => {}
            other => panic!("expected SendFailed, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_server4xx_rolls_back() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Server {
            status: 400,
            body: serde_json::json!({}),
        });
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

        let _ = expect_pending_user_changed(&up_rx);
        let rolled_back = expect_pending_user_changed(&up_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { .. }) => {}
            other => panic!("expected SendFailed, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_not_found_rolls_back() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::NotFound {
            what: "session".into(),
        });
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

        let _ = expect_pending_user_changed(&up_rx);
        let rolled_back = expect_pending_user_changed(&up_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { .. }) => {}
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

        // Drain park disconnect delta — baseline before post-park event.
        let mut updates_at_park = Vec::new();
        while let Ok(u) = up_rx.try_recv() {
            updates_at_park.push(u);
        }

        ev_tx.send(status_running_event()).unwrap();
        handle.stop_and_join();

        // Post-park event must be dropped, not merely delayed.
        let mut updates_after_park = Vec::new();
        while let Ok(u) = up_rx.try_recv() {
            updates_after_park.push(u);
        }
        assert!(
            updates_after_park.is_empty(),
            "parked actor must drop further events (got {updates_after_park:?})"
        );
    }

    #[test]
    fn disconnect_session_failed_parks_actor() {
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
                reason: DisconnectReason::SessionFailed,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Parked {
                reason: ParkReason::SessionFailed,
            } => {}
            other => panic!("expected Parked SessionFailed, got {other:?}"),
        }

        assert!(matches!(
            up_rx.recv_blocking().unwrap(),
            StreamUpdate::Disconnected(DisconnectReason::SessionFailed)
        ));

        handle.stop_and_join();
    }

    #[test]
    fn disconnect_retries_exhausted_parks_actor() {
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
                reason: DisconnectReason::RetriesExhausted,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Parked {
                reason: ParkReason::RetriesExhausted,
            } => {}
            other => panic!("expected Parked RetriesExhausted, got {other:?}"),
        }

        assert!(matches!(
            up_rx.recv_blocking().unwrap(),
            StreamUpdate::Disconnected(DisconnectReason::RetriesExhausted)
        ));

        handle.stop_and_join();
    }

    #[test]
    fn send_while_parked_is_rejected_no_bubble() {
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

        let _ = up_rx.recv_blocking().unwrap();
        while up_rx.try_recv().is_ok() {}

        handle
            .commands
            .send(SessionCommand::Send {
                text: "must not land".into(),
                model_override: None,
            })
            .unwrap();

        assert!(
            up_rx.try_recv().is_err(),
            "parked Send must not emit PendingUserChanged"
        );

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendRejected { reason }) => {
                assert!(reason.contains("parked"));
            }
            other => panic!("expected SendRejected, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn persist_error_drains_before_terminal_stop() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = failing_transcript_stores(_dir.path());
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

        // Determinism: actor idle in select; queue item + Forbidden disconnect before greedy drain.
        ev_tx.send(one_output_item_done_event()).unwrap();
        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Forbidden,
            })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::PersistError { where_, message } => {
                assert_eq!(where_, "transcript.upsert_item");
                assert!(!message.is_empty());
            }
            other => panic!("expected PersistError first, got {other:?}"),
        }
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::StoppedRemoved => {}
            other => panic!("expected StoppedRemoved after PersistError, got {other:?}"),
        }
        assert!(
            handle.outcomes.try_recv().is_err(),
            "no further outcomes after terminal stop"
        );

        handle.join_without_stop();
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

    #[test]
    fn reconnected_clears_reconcile_in_flight_and_connects() {
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

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::TransportChanged {
                transport: ActorTransport::Connected,
                reconcile_in_flight: false,
            } => {}
            other => panic!("expected TransportChanged Connected, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn park_suppresses_same_batch_reconnect() {
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

        // Determinism: actor is idle in select; queue both events before it drains.
        // Greedy try_recv folds Reconnecting + Disconnected into one batch.
        ev_tx
            .send(ServerStreamEvent::Reconnecting { attempt: 1 })
            .unwrap();
        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized,
            })
            .unwrap();

        let mut transport_outcomes = vec![handle.outcomes.recv_blocking().unwrap()];
        while let Ok(more) = handle.outcomes.try_recv() {
            transport_outcomes.push(more);
        }

        let last = transport_outcomes.last().expect("park outcome emitted");
        match last {
            ActorOutcome::Parked {
                reason: ParkReason::Unauthorized,
            } => {}
            other => panic!(
                "terminal park must win over same-batch reconnect (got {other:?}, all: {transport_outcomes:?})"
            ),
        }
        assert!(
            !transport_outcomes.iter().any(|o| matches!(
                o,
                ActorOutcome::TransportChanged {
                    transport: ActorTransport::Reconnecting,
                    ..
                }
            )),
            "same-batch reconnect must not emit TransportChanged Reconnecting after park"
        );

        let mut stream_updates = Vec::new();
        while let Ok(u) = up_rx.try_recv() {
            stream_updates.push(u);
        }
        assert!(
            stream_updates.iter().any(|u| matches!(
                u,
                StreamUpdate::Disconnected(DisconnectReason::Unauthorized)
            )),
            "batch must still emit Disconnected to foreground (got {stream_updates:?})"
        );

        ev_tx.send(status_running_event()).unwrap();
        handle.stop_and_join();

        let mut post_park = Vec::new();
        while let Ok(u) = up_rx.try_recv() {
            post_park.push(u);
        }
        assert!(
            post_park.is_empty(),
            "parked actor must drop post-park events (got {post_park:?})"
        );
    }

    /// `fetch_items` blocks on the Nth call until released; signals each completed fetch.
    struct GateFetchMock {
        script: Mutex<VecDeque<Result<ItemList, ClientError>>>,
        fetch_count: Mutex<u32>,
        block_on_fetch: u32,
        entered_tx: std::sync::mpsc::Sender<u32>,
        release_rx: std::sync::mpsc::Receiver<()>,
        sent_stop: Arc<Mutex<bool>>,
    }

    impl GateFetchMock {
        #[allow(clippy::type_complexity)]
        fn with_script(
            script: VecDeque<Result<ItemList, ClientError>>,
            block_on_fetch: u32,
        ) -> (
            Box<dyn SessionApi + Send>,
            std::sync::mpsc::Receiver<u32>,
            std::sync::mpsc::Sender<()>,
            Arc<Mutex<bool>>,
        ) {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let sent_stop = Arc::new(Mutex::new(false));
            (
                Box::new(Self {
                    script: Mutex::new(script),
                    fetch_count: Mutex::new(0),
                    block_on_fetch,
                    entered_tx,
                    release_rx,
                    sent_stop: Arc::clone(&sent_stop),
                }),
                entered_rx,
                release_tx,
                sent_stop,
            )
        }
    }

    impl SessionApi for GateFetchMock {
        fn send_event(
            &self,
            _id: &SessionId,
            evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
            if matches!(evt, SessionEventInput::StopSession) {
                *self.sent_stop.lock().unwrap() = true;
            }
            Ok(SendEventAck {
                queued: true,
                ..Default::default()
            })
        }

        fn fetch_items(
            &self,
            _id: &SessionId,
            _page: &lens_client::sessions::ItemsPage,
        ) -> Result<ItemList, ClientError> {
            let n = {
                let mut c = self.fetch_count.lock().unwrap();
                *c += 1;
                *c
            };
            let _ = self.entered_tx.send(n);
            if n == self.block_on_fetch {
                self.release_rx
                    .recv()
                    .expect("test must release blocked fetch");
            }
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(empty_item_list()))
        }
    }

    /// Mock whose `send_event` blocks until the test sends on `release_tx`.
    struct BlockingMockApi {
        entered_tx: std::sync::mpsc::Sender<()>,
        release_rx: std::sync::mpsc::Receiver<()>,
        ack: SendEventAck,
    }

    impl BlockingMockApi {
        fn with_ack(
            ack: SendEventAck,
        ) -> (
            Box<dyn SessionApi + Send>,
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::Sender<()>,
        ) {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let mock = BlockingMockApi {
                entered_tx,
                release_rx,
                ack,
            };
            (Box::new(mock), entered_rx, release_tx)
        }
    }

    impl SessionApi for BlockingMockApi {
        fn send_event(
            &self,
            _id: &SessionId,
            _evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
            let _ = self.entered_tx.send(());
            self.release_rx
                .recv()
                .expect("test must release blocked send");
            Ok(self.ack.clone())
        }

        fn fetch_items(
            &self,
            _id: &SessionId,
            _page: &lens_client::sessions::ItemsPage,
        ) -> Result<ItemList, ClientError> {
            Ok(empty_item_list())
        }
    }

    #[test]
    fn send_while_running_reconciles_without_teardown() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_running".into()),
            pending_id: None,
            ..Default::default()
        });
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        ev_tx.send(status_running_event()).unwrap();
        let _ = up_rx.recv_blocking().expect("running status delta");

        handle
            .commands
            .send(SessionCommand::Send {
                text: "while running".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&up_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].content, "while running");

        let stamped = expect_pending_user_changed(&up_rx);
        assert_eq!(stamped[0].store_item_id.as_deref(), Some("msg_running"));

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendAccepted { .. }) => {}
            other => panic!("expected SendAccepted, got {other:?}"),
        }

        ev_tx
            .send(ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_running".into(),
                item_type: "message".into(),
                cleared_pending_id: None,
            }))
            .unwrap();
        let cleared = expect_pending_user_changed(&up_rx);
        assert!(
            cleared.is_empty(),
            "InputConsumed clears bubble while running"
        );

        // Stream stays alive — further events and Stop still work.
        ev_tx.send(status_running_event()).unwrap();
        assert!(up_rx.recv_blocking().is_ok());
        handle.stop_and_join();

        // Network fail while running: rollback but no stream teardown.
        let _dir2 = tempfile::tempdir().unwrap();
        let stores2 = test_stores(_dir2.path());
        seed_connection(&stores2);
        let (api2, _mock2) = MockApi::fail(ClientError::network_for_test());
        let (ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let (up_tx2, up_rx2) = async_channel::bounded(64);
        let handle2 = spawn_actor(fresh_state(), ev_rx2, up_tx2, stores2, test_clock(), api2);
        ev_tx2.send(status_running_event()).unwrap();
        let _ = up_rx2.recv_blocking();
        handle2
            .commands
            .send(SessionCommand::Send {
                text: "will fail".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&up_rx2);
        assert!(expect_pending_user_changed(&up_rx2).is_empty());
        match handle2.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { .. }) => {}
            other => panic!("expected SendFailed, got {other:?}"),
        }
        ev_tx2.send(status_running_event()).unwrap();
        assert!(up_rx2.recv_blocking().is_ok());
        handle2.stop_and_join();
    }

    #[test]
    fn send_while_reconnecting_retains_then_reconcile_clears() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            pending_id: Some("pend_recon".into()),
            item_id: None,
            ..Default::default()
        });
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "mid-reconnect".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&up_rx);
        let stamped = expect_pending_user_changed(&up_rx);
        assert_eq!(stamped.len(), 1);
        assert_eq!(stamped[0].server_pending_id.as_deref(), Some("pend_recon"));
        let _ = handle.outcomes.recv_blocking().unwrap();

        ev_tx
            .send(ServerStreamEvent::Reconnecting { attempt: 1 })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::TransportChanged {
                transport: ActorTransport::Reconnecting,
                reconcile_in_flight: true,
            } => {}
            other => panic!("expected TransportChanged Reconnecting, got {other:?}"),
        }

        // §4 P3(b) rule 1: gap/reconnect alone must not clear pending_user.
        let reconnect_delta = up_rx.recv_blocking().expect("Reconnecting delta");
        assert!(matches!(
            reconnect_delta,
            StreamUpdate::Reconnecting { attempt: 1 }
        ));
        while let Ok(u) = up_rx.try_recv() {
            assert!(
                !matches!(u, StreamUpdate::PendingUserChanged(ref v) if v.is_empty()),
                "bubble must not be cleared before snapshot reconcile"
            );
        }

        let snap = snapshot_fixture(serde_json::json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "llm_model": "opus",
            "pending_inputs": [],
            "items": []
        }));
        ev_tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .unwrap();

        let mut saw_reconcile_clear = false;
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(&u, StreamUpdate::PendingUserChanged(v) if v.is_empty()) {
                saw_reconcile_clear = true;
                break;
            }
        }
        assert!(
            saw_reconcile_clear,
            "SnapshotRestored reconcile must drop bubble missing from pending_inputs"
        );

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::TransportChanged {
                transport: ActorTransport::Connected,
                reconcile_in_flight: false,
            } => {}
            other => panic!("expected TransportChanged Connected, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn demote_then_send_works_in_summary_mode() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_summary".into()),
            pending_id: None,
            ..Default::default()
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let (sum_tx, sum_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            up_tx,
            sum_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            api,
        );

        handle.commands.send(SessionCommand::Demote).unwrap();

        handle
            .commands
            .send(SessionCommand::Send {
                text: "summary send".into(),
                model_override: None,
            })
            .unwrap();

        // Summary mode does not mirror pending_user on the Detailed channel (M2).
        assert!(sum_rx.recv_blocking().is_ok(), "summary emitted on send");

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendAccepted {
                lens_pending_id,
                ack,
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
                assert_eq!(ack.item_id.as_deref(), Some("msg_summary"));
            }
            other => panic!("expected SendAccepted, got {other:?}"),
        }

        handle.stop_and_join();

        // Summary consumer dropped (spawn_actor) — Send still completes, actor survives.
        let _dir2 = tempfile::tempdir().unwrap();
        let stores2 = test_stores(_dir2.path());
        seed_connection(&stores2);
        let (api2, _mock2) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_no_sum".into()),
            pending_id: None,
            ..Default::default()
        });
        let (ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let (up_tx2, _up_rx2) = async_channel::bounded(64);
        let handle2 = spawn_actor(fresh_state(), ev_rx2, up_tx2, stores2, test_clock(), api2);
        handle2.commands.send(SessionCommand::Demote).unwrap();
        handle2
            .commands
            .send(SessionCommand::Send {
                text: "no consumer".into(),
                model_override: None,
            })
            .unwrap();
        match handle2.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendAccepted { .. }) => {}
            other => panic!("expected SendAccepted, got {other:?}"),
        }
        ev_tx2.send(status_running_event()).unwrap();
        handle2.stop_and_join();
    }

    #[test]
    fn stop_during_in_flight_send_joins_after_send_returns() {
        // Risk 5a: while blocked in `api.send_event`, Select services nothing — Stop
        // queues until POST returns. Production bounds this via REST_TIMEOUT (Task 6 F1).
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, entered_rx, release_tx) = BlockingMockApi::with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_blocked".into()),
            pending_id: None,
            ..Default::default()
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "in flight".into(),
                model_override: None,
            })
            .unwrap();

        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("actor must enter blocking send_event");

        handle.commands.send(SessionCommand::Stop).unwrap();

        release_tx.send(()).expect("release blocked send");

        let join = std::thread::spawn(move || handle.join_without_stop());
        join.join()
            .expect("join thread panicked — actor hung with Stop queued during in-flight send");

        // Optimistic + stamp emits complete before join returns.
        let _ = expect_pending_user_changed(&up_rx);
        let _ = expect_pending_user_changed(&up_rx);
    }

    #[test]
    fn sleep_when_quiescent_flushes_slept_and_stops() {
        use crate::domain::scalars::SessionLifecycle;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lens.db");
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, _up_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle.commands.send(SessionCommand::Sleep).unwrap();
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Slept => {}
            other => panic!("expected Slept, got {other:?}"),
        }
        handle.join_without_stop();

        let control = SqliteControlStore::open(&db_path).unwrap();
        let loaded = control.list_sessions(&ConnectionId::new("conn_1")).unwrap();
        assert_eq!(loaded.rows[0].lifecycle, SessionLifecycle::Slept);
        assert!(matches!(
            mock.last_evt(),
            Some(SessionEventInput::StopSession)
        ));
    }

    #[test]
    fn sleep_when_quiescent_but_flush_fails_is_declined_and_actor_survives() {
        use crate::domain::scalars::SessionLifecycle;

        let dir = tempfile::tempdir().unwrap();
        let (stores, db_path) = failing_control_stores(dir.path());
        seed_connection(&stores);
        SqliteControlStore::open(&db_path)
            .unwrap()
            .upsert_session(&fresh_state(), 1)
            .unwrap();
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        handle.commands.send(SessionCommand::Sleep).unwrap();
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::PersistError { where_, .. } => {
                assert_eq!(where_, "sleep.upsert_session");
            }
            other => panic!("expected PersistError first, got {other:?}"),
        }
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::SleepDeclined => {}
            other => panic!("expected SleepDeclined after flush failure, got {other:?}"),
        }
        assert!(
            mock.last_evt().is_none(),
            "StopSession must not be sent when sleep flush fails"
        );

        ev_tx.send(status_running_event()).unwrap();
        assert!(
            up_rx.recv_blocking().is_ok(),
            "actor must survive and process events"
        );

        let control = SqliteControlStore::open(&db_path).unwrap();
        let loaded = control.list_sessions(&ConnectionId::new("conn_1")).unwrap();
        assert_eq!(loaded.rows[0].lifecycle, SessionLifecycle::Active);

        handle.stop_and_join();
    }

    #[test]
    fn sleep_while_running_is_declined_and_actor_survives() {
        use crate::domain::scalars::SessionStatusValue;

        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);
        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let mut state = fresh_state();
        state.status = SessionStatusValue::Running;
        let handle = spawn_actor(state, ev_rx, up_tx, stores, test_clock(), api);

        handle.commands.send(SessionCommand::Sleep).unwrap();
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::SleepDeclined => {}
            other => panic!("expected SleepDeclined, got {other:?}"),
        }
        assert!(
            mock.last_evt().is_none(),
            "StopSession must not be sent when sleep is declined"
        );

        ev_tx.send(status_running_event()).unwrap();
        assert!(up_rx.recv_blocking().is_ok());
        handle.stop_and_join();
    }

    #[test]
    fn is_quiesced_requires_quiet_connected_and_settled() {
        use crate::domain::scalars::SessionStatusValue;

        let s = fresh_state(); // idle
        assert!(is_quiesced(&s, &ActorTransport::Connected, false));
        assert!(
            !is_quiesced(&s, &ActorTransport::Connected, true),
            "mid-reconcile"
        );
        assert!(
            !is_quiesced(
                &s,
                &ActorTransport::Parked {
                    reason: ParkReason::Unauthorized
                },
                false
            ),
            "parked is not quiesced"
        );
        let mut busy = fresh_state();
        busy.status = SessionStatusValue::Running;
        assert!(!is_quiesced(&busy, &ActorTransport::Connected, false));
    }

    fn seed_message_item(transcript: &dyn TranscriptStore, ordinal: i64, id: &str, text: &str) {
        let item = Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "output_text".into(),
                    text: Some(text.into()),
                    data: serde_json::Value::Null,
                }],
            },
        };
        transcript.upsert_item(ordinal, &item).unwrap();
    }

    fn item_list_from_messages(ids: &[&str], has_more: bool) -> ItemList {
        let data: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": id}]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "data": data,
            "has_more": has_more,
        }))
        .expect("item list fixture")
    }

    #[test]
    fn catchup_pages_forward_from_frontier_then_applies_buffered_live_tail() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        for (id, ord) in [("item_0", 0), ("item_1", 1), ("item_2", 2)] {
            seed_message_item(&*stores.transcript, ord, id, id);
        }
        assert_eq!(
            stores.transcript.frontier().unwrap(),
            Some((2, ItemId::new("item_2")))
        );

        let page1 = item_list_from_messages(&["item_3", "item_4"], true);
        let page2 = item_list_from_messages(&["item_5"], false);
        let (api, _mock) = MockApi::with_fetch_script(VecDeque::from([Ok(page1), Ok(page2)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_6","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();

        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        let mut saw_catchup_watermark = false;
        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 4
                } | StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 5
                }
            ) {
                saw_catchup_watermark = true;
            }
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 6
                }
            ) {
                break;
            }
        }
        assert!(
            saw_catchup_watermark,
            "catch-up must emit TranscriptAdvanced with committed_ordinal >= 5"
        );

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        assert_eq!(rows.len(), 7, "item_0..item_6 on disk");
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "item_0", "item_1", "item_2", "item_3", "item_4", "item_5", "item_6"
            ]
        );
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.id.as_str(), ids[i]);
        }
    }

    #[test]
    fn reconnected_greedy_drain_defers_live_commit_until_after_catchup() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_message_item(&*stores.transcript, 0, "item_0", "item_0");
        assert_eq!(
            stores.transcript.frontier().unwrap(),
            Some((0, ItemId::new("item_0")))
        );

        // Spawn catch-up consumes page 1 (empty); Reconnected catch-up consumes page 2.
        let history_page = item_list_from_messages(&["item_1", "item_2"], false);
        let (api, _mock) =
            MockApi::with_fetch_script(VecDeque::from([Ok(empty_item_list()), Ok(history_page)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        // spawn_actor returns only after spawn catch-up completes (empty page).
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_3","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();

        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 3
                }
            ) {
                break;
            }
        }

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["item_0", "item_1", "item_2", "item_3"],
            "durable history must land before the greedy-drained live tail"
        );
    }

    #[test]
    fn nested_buffered_reconnected_defers_live_until_nested_catchup() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_message_item(&*stores.transcript, 0, "item_0", "item_0");

        // Spawn: empty. Outer catch-up: empty (no history). Nested catch-up: [item_1, item_2].
        // Fetch #2 uses has_more=true so the loop drains channel events queued during the gate
        // before the outer catch-up exits; fetch #3 is the outer's terminal empty page.
        let history_page = item_list_from_messages(&["item_1", "item_2"], false);
        let (api, fetch_done_rx, release_fetch2, _sent_stop) = GateFetchMock::with_script(
            VecDeque::from([
                Ok(empty_item_list()),
                Ok(item_list_from_messages(&[], true)),
                Ok(empty_item_list()),
                Ok(history_page),
            ]),
            2, // block outer catch-up on fetch #2 until buffered slice is queued
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("spawn catch-up fetch"),
            1
        );

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("outer catch-up must enter blocked fetch"),
            2,
            "outer catch-up blocks on fetch #2"
        );

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_3","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();
        release_fetch2.send(()).expect("release blocked fetch");

        while let Ok(u) = up_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 3
                }
            ) {
                break;
            }
        }

        handle.stop_and_join();

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["item_0", "item_1", "item_2", "item_3"],
            "nested catch-up must write history before the deferred live tail commits"
        );
    }

    #[test]
    fn catchup_replays_buffered_live_before_deferred_sleep_recheck() {
        use crate::domain::scalars::SessionStatusValue;

        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        // Spawn catch-up blocks on fetch #1; has_more=true forces a drain of channel
        // events/commands before the catch-up exits on fetch #2.
        let (api, fetch_done_rx, release_fetch1, sent_stop) = GateFetchMock::with_script(
            VecDeque::from([
                Ok(item_list_from_messages(&[], true)),
                Ok(empty_item_list()),
            ]),
            1,
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), api);

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("spawn catch-up must block on fetch #1"),
            1
        );

        ev_tx.send(status_running_event()).unwrap();
        handle.commands.send(SessionCommand::Sleep).unwrap();
        release_fetch1
            .send(())
            .expect("release blocked spawn catch-up fetch");

        loop {
            match up_rx.recv_blocking() {
                Ok(StreamUpdate::StatusChanged(SessionStatusValue::Running)) => break,
                Ok(_) => {}
                Err(_) => panic!("updates channel closed before Running status"),
            }
        }

        match handle.outcomes.recv_blocking() {
            Ok(ActorOutcome::SleepDeclined) => {}
            Ok(other) => panic!("expected SleepDeclined, got {other:?}"),
            Err(_) => panic!("outcomes channel closed before SleepDeclined"),
        }
        assert!(
            !*sent_stop.lock().unwrap(),
            "StopSession must not be sent when transient work is outstanding"
        );

        ev_tx.send(status_running_event()).unwrap();
        assert!(
            up_rx.recv_blocking().is_ok(),
            "actor must survive and keep processing events"
        );

        handle.stop_and_join();
    }
}
