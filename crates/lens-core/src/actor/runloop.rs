//! The actor run-loop: `crossbeam::Select` over events + commands, greedy drain,
//! persist write-through, coalesce, emit to the foreground bridge.

use crate::actor::api::{CommandOutcome, SessionApi};
use crate::actor::feed::ActorFeed;
use crate::actor::outcome::{ActorOutcome, Mapped, OutcomeRing, map_client_error};
use crate::actor::summary::SummaryUpdate;
use crate::actor::transport::{ActorTransport, ParkReason};
use crate::clock::Clock;
use crate::domain::SessionState;
use crate::domain::controls::PendingUserMessage;
use crate::domain::item::{BlockContext, Item, ItemKind};
use crate::domain::scalars::SessionLifecycle;
use crate::persist::map::item_kind_token;
use crate::persist::{ControlStore, LiveKey, ReconcileOutcome, TranscriptStore};
use crate::reduce::map_wire_item;
use crate::reduce::user_text;
use crate::reduce::{StreamUpdate, Updates, reduce};
use crossbeam_channel::{Receiver, Select};
use lens_client::sessions::SessionEventInput;
use lens_client::sessions::{ItemsPage, PendingInput};
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

    /// True when the actor thread has finished (park terminal or stop).
    pub(crate) fn is_exited(&self) -> bool {
        self.join.is_finished()
    }

    /// Block until an actor that exited on its own (park terminal) is joined.
    pub fn join_exited(self) {
        self.join
            .join()
            .expect("actor thread panicked or was poisoned");
    }

    #[cfg(test)]
    pub fn join_without_stop(self) {
        self.join_exited();
    }
}

/// Bridge senders + output granularity for the actor run-loop.
struct ActorOutput {
    feed: async_channel::Sender<ActorFeed>,
    outcomes: async_channel::Sender<ActorOutcome>,
    mode: OutputMode,
}

/// Thread-local actor run state shared across catch-up and main-loop paths (C4).
struct RunCtx<'a> {
    api: &'a dyn SessionApi,
    stores: &'a ActorStores,
    state: &'a mut SessionState,
    events: &'a Receiver<ServerStreamEvent>,
    commands: &'a Receiver<SessionCommand>,
    output: &'a mut ActorOutput,
    ring: &'a mut OutcomeRing,
    clock: &'a dyn Clock,
    next_ordinal: &'a mut i64,
    transport: &'a mut ActorTransport,
    reconcile_in_flight: &'a mut bool,
    send_seq: &'a mut u64,
    catchup_accum: &'a mut Vec<(String, i64)>,
    snapshot_pending_inputs: &'a mut Vec<PendingInput>,
}

/// Spawn the actor thread in `Detailed` mode.
pub fn spawn_actor(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> ActorHandle {
    spawn_actor_dual(
        state,
        events,
        feed,
        OutputMode::Detailed,
        stores,
        clock,
        api,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_actor_dual(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    feed: async_channel::Sender<ActorFeed>,
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
                    feed,
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
        catchup_new_user_contents: Vec<(String, i64)>,
    },
    Aborted,
}

/// Deferred work from an outer catch-up round while a nested `Reconnected` replays.
struct CatchupFrame {
    deferred_commands: Vec<SessionCommand>,
    defer_transcript_commit: bool,
}

struct ReplayBatchResult {
    control: LoopControl,
    defer_transcript_commit: bool,
    needs_catchup: bool,
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

/// D30 reconcile key for a store `/items` row: `call_id` only on tool splits.
fn live_key_for_store_item(item: &Item) -> LiveKey {
    let call_id = match &item.kind {
        ItemKind::FunctionCall { call_id, .. } | ItemKind::FunctionCallOutput { call_id, .. } => {
            Some(call_id.clone())
        }
        _ => None,
    };
    let scaffold_kind = if call_id.is_some() {
        Some(item_kind_token(&item.kind))
    } else {
        None
    };
    LiveKey {
        id: item.id.clone(),
        call_id,
        scaffold_kind,
    }
}

fn user_message_text(item: &Item) -> Option<String> {
    user_text(item)
}

fn apply_held_reconcile(ctx: &mut RunCtx<'_>) -> LoopControl {
    let before = ctx.state.pending_user.clone();
    let lost = crate::reduce::reconcile_held_landed(
        &mut ctx.state.pending_user,
        ctx.snapshot_pending_inputs,
        ctx.catchup_accum,
    );
    for l in &lost {
        if ctx
            .output
            .outcomes
            .send_blocking(ActorOutcome::SendLost {
                lens_pending_id: l.lens_pending_id.clone(),
                content: l.content.clone(),
            })
            .is_err()
        {
            return LoopControl::Break;
        }
    }
    if ctx.state.pending_user != before && !emit_pending_user(ctx.output, ctx.state) {
        return LoopControl::Break;
    }
    drain_outcome_ring(ctx.ring, &ctx.output.outcomes);
    LoopControl::Continue
}

/// Catch-up upsert: advance `next_ordinal` only on a fresh insert at the passed ordinal.
fn upsert_catchup_item(
    stores: &ActorStores,
    next_ordinal: &mut i64,
    item: &Item,
    ring: &mut OutcomeRing,
) -> bool {
    match stores.transcript.upsert_item(*next_ordinal, item, false) {
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
    let mut after = match stores.transcript.store_frontier() {
        Ok(v) => v.map(|(_, id)| id.to_string()),
        Err(e) => {
            ring.push(ActorOutcome::PersistError {
                where_: "transcript.store_frontier",
                message: e.to_string(),
            });
            drain_outcome_ring(ring, &output.outcomes);
            let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                reason: ParkReason::SessionFailed,
            });
            return CatchupResult::Aborted;
        }
    };
    let mut buffered_events = Vec::new();
    let mut deferred_commands = Vec::new();
    let mut catchup_new_user_contents = Vec::new();

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
                drain_outcome_ring(ring, &output.outcomes);
                let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                    reason: ParkReason::SessionFailed,
                });
                return CatchupResult::Aborted;
            }
        };

        let mut wrote_any = false;
        for wire in list.items() {
            if let Some(domain) = wire_to_domain_item(wire, clock) {
                let live_key = live_key_for_store_item(&domain);
                match stores.transcript.reconcile_store_item(&domain, &live_key) {
                    Ok(ReconcileOutcome::Folded { ordinal }) => {
                        *next_ordinal = (*next_ordinal).max(ordinal + 1);
                        wrote_any = true;
                    }
                    Ok(ReconcileOutcome::NoMatch) => {
                        if upsert_catchup_item(stores, next_ordinal, &domain, ring) {
                            wrote_any = true;
                            if let Some(text) = user_message_text(&domain) {
                                catchup_new_user_contents.push((text, domain.created_at));
                            }
                        }
                    }
                    Err(e) => {
                        ring.push(ActorOutcome::PersistError {
                            where_: "transcript.reconcile_store_item",
                            message: e.to_string(),
                        });
                        drain_outcome_ring(ring, &output.outcomes);
                        let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                            reason: ParkReason::SessionFailed,
                        });
                        return CatchupResult::Aborted;
                    }
                }
            }
            after = Some(wire.id().to_string());
        }
        if wrote_any
            && output
                .feed
                .send_blocking(ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: *next_ordinal - 1,
                }))
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
        catchup_new_user_contents,
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
                .feed
                .send_blocking(ActorFeed::Detailed(StreamUpdate::Rebased(
                    scalars_baseline(state),
                )))
                .is_err()
            {
                return LoopControl::Break;
            }
            output.mode = OutputMode::Detailed;
            LoopControl::Continue
        }
        SessionCommand::Demote => {
            output.mode = OutputMode::Summary;
            // §3.3 emit-on-Demote: blur returns the card to the summary projection
            // instead of freezing on the last Detailed frame (symmetric with Promote).
            if output
                .feed
                .send_blocking(ActorFeed::Summary(Box::new(SummaryUpdate::from_state(
                    state,
                ))))
                .is_err()
            {
                return LoopControl::Break;
            }
            LoopControl::Continue
        }
        SessionCommand::Send {
            text,
            model_override,
        } => {
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
                    let content = state
                        .pending_user
                        .iter()
                        .find(|p| p.pending_id == lens_pending_id)
                        .map(|p| p.content.clone())
                        .unwrap_or_default();
                    rollback_pending(state, &lens_pending_id);
                    if !emit_pending_user(output, state) {
                        return LoopControl::Break;
                    }
                    let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                        CommandOutcome::SendDenied {
                            lens_pending_id,
                            content,
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
                    let content = state
                        .pending_user
                        .iter()
                        .find(|p| p.pending_id == lens_pending_id)
                        .map(|p| p.content.clone())
                        .unwrap_or_default();
                    if m.rolls_back_send() {
                        rollback_pending(state, &lens_pending_id);
                        if !emit_pending_user(output, state) {
                            return LoopControl::Break;
                        }
                        // Denied (Auth403) vs Failed (Network/404): both remove the bubble and restore
                        // to composer; Denied carries the server reason (other 4xx → Denied).
                        let outcome = match m {
                            Mapped::LostAccess | Mapped::Denied => CommandOutcome::SendDenied {
                                lens_pending_id,
                                content,
                                reason: Some(e.to_string()),
                            },
                            _ => CommandOutcome::SendFailed {
                                lens_pending_id,
                                content,
                                error: e.to_string(),
                            },
                        };
                        let _ = output
                            .outcomes
                            .send_blocking(ActorOutcome::Command(outcome));
                    } else {
                        // Held (5xx/401/ContractMismatch/Parse): bubble stays, soft pending, no content.
                        let _ = output.outcomes.send_blocking(ActorOutcome::Command(
                            CommandOutcome::SendPending { lens_pending_id },
                        ));
                    }
                }
            }
            LoopControl::Continue
        }
    }
}

fn apply_reduced_batch(
    ctx: &mut RunCtx<'_>,
    batch: Updates,
    defer_transcript_commit: bool,
) -> (LoopControl, bool) {
    persist_scalars(
        ctx.stores,
        ctx.state,
        &batch,
        ctx.clock.now_millis(),
        ctx.ring,
    );
    let mut batch = batch;
    if !defer_transcript_commit
        && let Some(ord) = commit_terminal_prefix(ctx.stores, ctx.state, ctx.next_ordinal, ctx.ring)
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
    if saw_reconnecting {
        ctx.snapshot_pending_inputs.clear();
        ctx.catchup_accum.clear();
    }
    if let Some(inputs) = batch.iter().rev().find_map(|u| match u {
        StreamUpdate::SnapshotRestored(inputs) => Some(inputs.clone()),
        _ => None,
    }) {
        ctx.snapshot_pending_inputs.clear();
        ctx.catchup_accum.clear();
        ctx.snapshot_pending_inputs.extend(inputs);
    }
    match ctx.output.mode {
        OutputMode::Detailed => {
            let had_snapshot = batch
                .iter()
                .any(|u| matches!(u, StreamUpdate::SnapshotRestored(_)));
            for u in coalesce(batch) {
                if ctx
                    .output
                    .feed
                    .send_blocking(ActorFeed::Detailed(u))
                    .is_err()
                {
                    return (LoopControl::Break, false);
                }
            }
            if had_snapshot
                && ctx
                    .output
                    .feed
                    .send_blocking(ActorFeed::Detailed(StreamUpdate::Rebased(
                        scalars_baseline(ctx.state),
                    )))
                    .is_err()
            {
                return (LoopControl::Break, false);
            }
        }
        OutputMode::Summary => {
            if ctx
                .output
                .feed
                .send_blocking(ActorFeed::Summary(Box::new(SummaryUpdate::from_state(
                    ctx.state,
                ))))
                .is_err()
            {
                ctx.ring.push(ActorOutcome::SummaryConsumerGone);
            }
        }
    }
    drain_outcome_ring(ctx.ring, &ctx.output.outcomes);
    if let Some(reason) = disconnect_reason {
        let park = match reason {
            DisconnectReason::Unauthorized => ParkReason::Unauthorized,
            DisconnectReason::SessionFailed => ParkReason::SessionFailed,
            DisconnectReason::RetriesExhausted => ParkReason::RetriesExhausted,
            DisconnectReason::Forbidden => ParkReason::Forbidden,
            DisconnectReason::NotFound => ParkReason::NotFound,
        };
        *ctx.reconcile_in_flight = false;
        let _ = ctx
            .output
            .outcomes
            .send_blocking(ActorOutcome::Parked { reason: park });
        return (LoopControl::Break, false);
    }
    if disconnect_reason.is_none() && saw_reconnecting {
        *ctx.transport = ActorTransport::Reconnecting;
        *ctx.reconcile_in_flight = true;
        if !emit_transport_changed(ctx.output, *ctx.transport, *ctx.reconcile_in_flight) {
            return (LoopControl::Break, false);
        }
    }
    if disconnect_reason.is_none() && saw_reconnected {
        *ctx.transport = ActorTransport::Connected;
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
fn finish_reconnected_catchup(
    ctx: &mut RunCtx<'_>,
    needs_catchup: bool,
    defer_transcript_commit: bool,
    emit_transport: bool,
) -> LoopControl {
    if !needs_catchup {
        return LoopControl::Continue;
    }
    if invoke_catchup_and_replay(ctx, emit_transport, true, &mut false) == LoopControl::Break {
        return LoopControl::Break;
    }
    if defer_transcript_commit {
        return finish_deferred_transcript_commit(
            ctx.stores,
            ctx.state,
            ctx.next_ordinal,
            ctx.ring,
            ctx.output,
        );
    }
    LoopControl::Continue
}

/// Reduce a buffered event slice; caller owns nested catch-up iteration (C3).
fn replay_buffered_batch(
    ctx: &mut RunCtx<'_>,
    buffered_events: &[ServerStreamEvent],
) -> ReplayBatchResult {
    let mut batch = reduce(ctx.state, &buffered_events[0], ctx.clock);
    for ev in &buffered_events[1..] {
        batch.extend(reduce(ctx.state, ev, ctx.clock));
    }
    let defer_transcript_commit = reconnected_defer_commit(&batch);
    let (ctrl, needs_catchup) = apply_reduced_batch(ctx, batch, defer_transcript_commit);
    ReplayBatchResult {
        control: ctrl,
        defer_transcript_commit,
        needs_catchup,
    }
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
            .feed
            .send_blocking(ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
                committed_ordinal: ord,
            }))
            .is_err()
    {
        return LoopControl::Break;
    }
    drain_outcome_ring(ring, &output.outcomes);
    LoopControl::Continue
}

fn process_main_loop_event(ctx: &mut RunCtx<'_>, event: ServerStreamEvent) -> LoopControl {
    let mut batch = reduce(ctx.state, &event, ctx.clock);
    while let Ok(next) = ctx.events.try_recv() {
        batch.extend(reduce(ctx.state, &next, ctx.clock));
    }
    // D19: live follow-ons in the same greedy-drained batch must not commit before catch-up.
    let defer_transcript_commit = reconnected_defer_commit(&batch);

    let (ctrl, needs_catchup) = apply_reduced_batch(ctx, batch, defer_transcript_commit);
    if ctrl == LoopControl::Break {
        return LoopControl::Break;
    }
    finish_reconnected_catchup(ctx, needs_catchup, defer_transcript_commit, true)
}

fn invoke_catchup_and_replay(
    ctx: &mut RunCtx<'_>,
    emit_transport: bool,
    held_reconcile: bool,
    // Set true if any buffered live events were replayed (which, in Summary mode,
    // already emit a Summary). The startup caller uses this to suppress a duplicate
    // seed; reconnect callers pass a throwaway.
    replayed_live: &mut bool,
) -> LoopControl {
    ctx.catchup_accum.clear();
    let mut frame_stack: Vec<CatchupFrame> = Vec::new();
    let mut current_emit_transport = emit_transport;
    'catchup: loop {
        if !begin_catchup_reconcile(
            ctx.reconcile_in_flight,
            ctx.output,
            *ctx.transport,
            current_emit_transport,
        ) {
            return LoopControl::Break;
        }
        let catchup = run_catchup(
            ctx.api,
            ctx.stores,
            ctx.state,
            ctx.next_ordinal,
            ctx.events,
            ctx.commands,
            ctx.output,
            ctx.ring,
            ctx.clock,
        );
        if !end_catchup_reconcile(
            ctx.reconcile_in_flight,
            ctx.output,
            *ctx.transport,
            current_emit_transport,
        ) {
            return LoopControl::Break;
        }
        drain_outcome_ring(ctx.ring, &ctx.output.outcomes);
        match catchup {
            CatchupResult::Aborted => return LoopControl::Break,
            CatchupResult::CaughtUp {
                buffered_events,
                deferred_commands,
                catchup_new_user_contents,
            } => {
                ctx.catchup_accum.extend(catchup_new_user_contents);
                if !buffered_events.is_empty() {
                    *replayed_live = true;
                    let replay = replay_buffered_batch(ctx, &buffered_events);
                    if replay.control == LoopControl::Break {
                        return LoopControl::Break;
                    }
                    if replay.needs_catchup {
                        frame_stack.push(CatchupFrame {
                            deferred_commands,
                            defer_transcript_commit: replay.defer_transcript_commit,
                        });
                        current_emit_transport = true;
                        continue 'catchup;
                    }
                    if replay.defer_transcript_commit
                        && finish_deferred_transcript_commit(
                            ctx.stores,
                            ctx.state,
                            ctx.next_ordinal,
                            ctx.ring,
                            ctx.output,
                        ) == LoopControl::Break
                    {
                        return LoopControl::Break;
                    }
                }
                for cmd in deferred_commands {
                    if handle_command(
                        cmd,
                        ctx.state,
                        ctx.stores,
                        ctx.output,
                        ctx.ring,
                        ctx.clock,
                        ctx.api,
                        ctx.transport,
                        *ctx.reconcile_in_flight,
                        ctx.send_seq,
                    ) == LoopControl::Break
                    {
                        return LoopControl::Break;
                    }
                }
                break 'catchup;
            }
        }
    }
    while let Some(frame) = frame_stack.pop() {
        if frame.defer_transcript_commit
            && finish_deferred_transcript_commit(
                ctx.stores,
                ctx.state,
                ctx.next_ordinal,
                ctx.ring,
                ctx.output,
            ) == LoopControl::Break
        {
            return LoopControl::Break;
        }
        for cmd in frame.deferred_commands {
            if handle_command(
                cmd,
                ctx.state,
                ctx.stores,
                ctx.output,
                ctx.ring,
                ctx.clock,
                ctx.api,
                ctx.transport,
                *ctx.reconcile_in_flight,
                ctx.send_seq,
            ) == LoopControl::Break
            {
                return LoopControl::Break;
            }
        }
    }
    if held_reconcile {
        if apply_held_reconcile(ctx) == LoopControl::Break {
            return LoopControl::Break;
        }
        ctx.snapshot_pending_inputs.clear();
    }
    LoopControl::Continue
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
    let mut next_ordinal: i64 = match stores.transcript.next_ordinal_seed() {
        Ok(seed) => seed,
        Err(e) => {
            ring.push(ActorOutcome::PersistError {
                where_: "transcript.next_ordinal_seed",
                message: e.to_string(),
            });
            drain_outcome_ring(&mut ring, &output.outcomes);
            let _ = output.outcomes.send_blocking(ActorOutcome::Parked {
                reason: ParkReason::SessionFailed,
            });
            return;
        }
    };
    let mut transport = ActorTransport::Connected;
    let mut reconcile_in_flight = false;
    let mut catchup_accum = Vec::new();
    let mut snapshot_pending_inputs = Vec::new();
    let mut ctx = RunCtx {
        api: api.as_ref(),
        stores: &stores,
        state: &mut state,
        events: &events,
        commands: &commands,
        output: &mut output,
        ring: &mut ring,
        clock: clock.as_ref(),
        next_ordinal: &mut next_ordinal,
        transport: &mut transport,
        reconcile_in_flight: &mut reconcile_in_flight,
        send_seq: &mut send_seq,
        catchup_accum: &mut catchup_accum,
        snapshot_pending_inputs: &mut snapshot_pending_inputs,
    };

    let mut replayed_live = false;
    if invoke_catchup_and_replay(&mut ctx, false, false, &mut replayed_live) == LoopControl::Break {
        // pre-loop spawn catch-up: no TransportChanged — Sleep cannot race yet
        return;
    }

    // §3.3 seed-on-spawn: a Summary-mode actor emits its initial projection after
    // catch-up so the card has data before the first live event. Seed-fail pushes
    // SummaryConsumerGone and continues (mirrors the Summary batch emit); it does
    // not abort the actor, so Stop still works.
    //
    // Suppressed when startup catch-up already replayed buffered live events: in
    // Summary mode that replay already emits a Summary, so an unconditional seed
    // would be a duplicate projection landing AFTER the live frame, violating the
    // catch-up → seed → live order (codex final-review C1).
    if ctx.output.mode == OutputMode::Summary
        && !replayed_live
        && ctx
            .output
            .feed
            .send_blocking(ActorFeed::Summary(Box::new(SummaryUpdate::from_state(
                ctx.state,
            ))))
            .is_err()
    {
        ctx.ring.push(ActorOutcome::SummaryConsumerGone);
        drain_outcome_ring(ctx.ring, &ctx.output.outcomes);
    }

    loop {
        let mut sel = Select::new();
        let ev_idx = sel.recv(&events);
        let cmd_idx = sel.recv(&commands);
        let oper = sel.select();
        match oper.index() {
            i if i == cmd_idx => match oper.recv(&commands) {
                Ok(cmd) => {
                    if handle_command(
                        cmd,
                        ctx.state,
                        ctx.stores,
                        ctx.output,
                        ctx.ring,
                        ctx.clock,
                        ctx.api,
                        ctx.transport,
                        *ctx.reconcile_in_flight,
                        ctx.send_seq,
                    ) == LoopControl::Break
                    {
                        break;
                    }
                }
                Err(_) => break,
            },
            i if i == ev_idx => match oper.recv(&events) {
                Ok(event) => {
                    if process_main_loop_event(&mut ctx, event) == LoopControl::Break {
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
            .feed
            .send_blocking(ActorFeed::Detailed(StreamUpdate::PendingUserChanged(
                state.pending_user.clone(),
            )))
            .is_ok(),
        OutputMode::Summary => {
            let _ =
                output
                    .feed
                    .send_blocking(ActorFeed::Summary(Box::new(SummaryUpdate::from_state(
                        state,
                    ))));
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
        // D30: live commits land provisional until catch-up folds store ids.
        match stores.transcript.upsert_item(*next_ordinal, front, true) {
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
    use crate::actor::feed::ActorFeed;
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
    use lens_client::sessions::{ItemList, SendEventAck, SessionEventInput, SessionStatus};
    use lens_client::stream::{
        DisconnectReason, ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus,
        decode_all,
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
        status_script: Mutex<VecDeque<Result<SessionStatus, ClientError>>>,
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
                status_script: Mutex::new(VecDeque::new()),
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
                status_script: Mutex::new(VecDeque::new()),
                last_evt: Mutex::new(None),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn fail(err: ClientError) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(VecDeque::from([Err(err)])),
                fetch_script: Mutex::new(VecDeque::new()),
                status_script: Mutex::new(VecDeque::new()),
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            self.status_script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(SessionStatus::Idle))
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            Ok(SessionStatus::Idle)
        }
    }

    fn noop_api() -> Box<dyn SessionApi + Send> {
        Box::new(PanicApi)
    }

    fn recv_detailed(feed_rx: &async_channel::Receiver<ActorFeed>) -> StreamUpdate {
        match feed_rx.recv_blocking().unwrap() {
            ActorFeed::Detailed(u) => u,
            other => panic!("expected Detailed, got {other:?}"),
        }
    }

    fn expect_pending_user_changed(
        feed_rx: &async_channel::Receiver<ActorFeed>,
    ) -> Vec<PendingUserMessage> {
        match recv_detailed(feed_rx) {
            StreamUpdate::PendingUserChanged(v) => v,
            other => panic!("expected PendingUserChanged, got {other:?}"),
        }
    }

    fn recv_pending_user_changed_or_none(
        feed_rx: &async_channel::Receiver<ActorFeed>,
    ) -> Option<Vec<PendingUserMessage>> {
        loop {
            match feed_rx.recv_blocking().unwrap() {
                ActorFeed::Detailed(StreamUpdate::PendingUserChanged(v)) => return Some(v),
                ActorFeed::Detailed(
                    StreamUpdate::Reconnected
                    | StreamUpdate::Reconnecting { .. }
                    | StreamUpdate::TranscriptAdvanced { .. }
                    | StreamUpdate::SnapshotRestored(_)
                    | StreamUpdate::Rebased(_),
                ) => {}
                ActorFeed::Summary(_) => {}
                ActorFeed::Detailed(other) => {
                    panic!("unexpected update while waiting for PendingUserChanged: {other:?}")
                }
            }
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

        fn upsert_item(
            &self,
            _ordinal: i64,
            _item: &Item,
            _provisional: bool,
        ) -> crate::persist::Result<i64> {
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

        fn store_frontier(&self) -> crate::persist::Result<Option<(i64, ItemId)>> {
            Ok(None)
        }

        fn next_ordinal_seed(&self) -> crate::persist::Result<i64> {
            Ok(0)
        }

        fn reconcile_store_item(
            &self,
            _store_item: &Item,
            _live_key: &crate::persist::LiveKey,
        ) -> crate::persist::Result<crate::persist::ReconcileOutcome> {
            Err(PersistError::ReadOnly)
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            noop_api(),
        );

        // §3.3 seed-on-spawn: drain the initial Summary seed before driving events.
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "spawn-in-Summary seeds an initial Summary projection"
        );

        ev_tx.send(status_running_event()).unwrap();
        assert!(matches!(
            feed_rx.recv_blocking().unwrap(),
            ActorFeed::Summary(_)
        ));
        assert!(
            feed_rx.try_recv().is_err(),
            "no Detailed deltas in Summary mode"
        );

        handle.commands.send(SessionCommand::Promote).unwrap();
        assert!(matches!(recv_detailed(&feed_rx), StreamUpdate::Rebased(_)));
        handle.stop_and_join();
    }

    #[test]
    fn summary_spawn_seeds_after_catchup() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            noop_api(),
        );

        // Empty catch-up → no TranscriptAdvanced; seed must still arrive.
        match feed_rx.recv_blocking().expect("seed") {
            ActorFeed::Summary(u) => {
                assert_eq!(u.last_completed_turn, 0);
            }
            other => panic!("expected Summary seed, got {other:?}"),
        }
        handle.stop_and_join();
    }

    #[test]
    fn summary_mode_nonempty_catchup_then_seed_preserves_fifo_order() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        // Frontier on disk so catch-up fetches a nonempty page and emits TranscriptAdvanced.
        seed_message_item(&*stores.transcript, 0, "item_0", "item_0");
        assert_eq!(
            stores.transcript.store_frontier().unwrap(),
            Some((0, ItemId::new("item_0")))
        );

        let page = item_list_from_messages(&["item_1"], false);
        let (api, _mock) = MockApi::with_fetch_script(VecDeque::from([Ok(page)]));

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            api,
        );

        let first = feed_rx.recv_blocking().expect("catch-up frame");
        let second = feed_rx.recv_blocking().expect("seed frame");
        assert!(
            matches!(
                first,
                ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })
            ),
            "first must be catch-up Detailed TranscriptAdvanced, got {first:?}"
        );
        assert!(
            matches!(second, ActorFeed::Summary(_)),
            "second must be §3.3 Summary seed, got {second:?}"
        );

        handle.stop_and_join();
    }

    /// codex final-review C1: when startup catch-up replays a buffered live event
    /// (which in Summary mode already emits a Summary), the seed must be SUPPRESSED
    /// so the FIFO is `Detailed(catch-up) → Summary(replayed live)` with no trailing
    /// duplicate seed. Order stays catch-up → live; no double projection.
    #[test]
    fn summary_seed_suppressed_when_startup_replays_buffered_live() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        // Disk frontier so catch-up fetches a nonempty page and emits TranscriptAdvanced.
        seed_message_item(&*stores.transcript, 0, "item_0", "item_0");

        let page = item_list_from_messages(&["item_1"], false);
        let (api, _mock) = MockApi::with_fetch_script(VecDeque::from([Ok(page)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        // Queue a live event BEFORE spawn so startup catch-up buffers + replays it.
        ev_tx.send(status_running_event()).unwrap();

        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            api,
        );

        // Order: catch-up Detailed(TranscriptAdvanced) THEN the replayed live Summary.
        assert!(
            matches!(
                feed_rx.recv_blocking().unwrap(),
                ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })
            ),
            "first frame must be the catch-up watermark"
        );
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "second frame must be the replayed live Summary"
        );

        // Seed suppressed: after the actor is quiesced, no duplicate seed Summary remains.
        handle.stop_and_join();
        while let Ok(frame) = feed_rx.try_recv() {
            assert!(
                !matches!(frame, ActorFeed::Summary(_)),
                "seed must be suppressed when startup already replayed a live Summary, got {frame:?}"
            );
        }
    }

    #[test]
    fn demote_emits_summary_from_state() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let mut state = fresh_state();
        state.title = Some("focused".into());
        state.stream.turn = 3;

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(state, ev_rx, feed_tx, stores, test_clock(), noop_api());

        handle.commands.send(SessionCommand::Demote).unwrap();
        match feed_rx.recv_blocking().expect("demote summary") {
            ActorFeed::Summary(u) => {
                assert_eq!(u.title.as_deref(), Some("focused"));
                assert_eq!(u.last_completed_turn, 3);
            }
            other => panic!("expected Summary on Demote, got {other:?}"),
        }
        handle.stop_and_join();
    }

    #[test]
    fn lagging_consumer_never_applies_stale_summary_after_promote() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        // Small capacity exercises backpressure on the unified FIFO without timing gates.
        let (feed_tx, feed_rx) = async_channel::bounded(2);
        let mut state = fresh_state();
        state.title = Some("bg".into());
        let handle = spawn_actor_dual(
            state,
            ev_rx,
            feed_tx,
            OutputMode::Summary,
            stores,
            test_clock(),
            noop_api(),
        );

        // Drain seed so the producer can advance on the bounded channel.
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "spawn-in-Summary seeds first"
        );

        // Enqueue live work, Promote, then a Detailed-visible event — no wall-clock waits.
        ev_tx.send(status_running_event()).unwrap();
        ev_tx.send(status_running_event()).unwrap();
        handle.commands.send(SessionCommand::Promote).unwrap();
        ev_tx.send(status_running_event()).unwrap();

        // Phase 1: recv_blocking until Rebased; Summary before Rebased is allowed.
        loop {
            match feed_rx.recv_blocking().expect("frame before Rebased") {
                ActorFeed::Summary(_) => {}
                ActorFeed::Detailed(StreamUpdate::Rebased(_)) => break,
                other @ ActorFeed::Detailed(_) => {
                    panic!("unexpected Detailed before Rebased: {other:?}");
                }
            }
        }

        // Phase 2: drain every remaining frame — no Summary may follow Rebased.
        handle.stop_and_join();
        while let Ok(frame) = feed_rx.recv_blocking() {
            assert!(
                !matches!(frame, ActorFeed::Summary(_)),
                "lagging Summary must not overtake Promote on the unified FIFO, got {frame:?}"
            );
        }
    }

    #[test]
    fn persist_error_lands_on_ring_without_blocking_emit() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = failing_transcript_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx.send(one_output_item_done_event()).unwrap();
        let _update = recv_detailed(&feed_rx);

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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
            stores,
            test_clock(),
            noop_api(),
        );

        ev_tx.send(one_output_item_done_event()).unwrap();
        let mut saw_watermark = false;
        while let Ok(ActorFeed::Detailed(update)) = feed_rx.recv_blocking() {
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.recv_blocking() {
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

        while feed_rx.try_recv().is_ok() {}

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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.recv_blocking() {
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        loop {
            let u = recv_detailed(&feed_rx);
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
            match feed_rx.try_recv() {
                Ok(ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced { .. })) => {
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
        loop {
            let u = recv_detailed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.recv_blocking() {
            match &u {
                StreamUpdate::SnapshotRestored(_) => saw_snapshot = true,
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

        let (api, _mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            item_id: Some("msg_demote".into()),
            pending_id: None,
            ..Default::default()
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        // Commands are FIFO — Demote flips mode before Send's Summary emit.
        handle.commands.send(SessionCommand::Demote).unwrap();
        handle
            .commands
            .send(SessionCommand::Send {
                text: "summary path".into(),
                model_override: None,
            })
            .unwrap();

        match feed_rx.recv_blocking().unwrap() {
            ActorFeed::Summary(_) => {}
            other => panic!("expected Summary after Demote+Send, got {other:?}"),
        }
        handle.commands.send(SessionCommand::Promote).unwrap();
        loop {
            match feed_rx.recv_blocking().unwrap() {
                ActorFeed::Detailed(StreamUpdate::Rebased(_)) => break,
                ActorFeed::Summary(_) => continue,
                other => panic!("expected Rebased after Promote, got {other:?}"),
            }
        }
        handle.stop_and_join();
    }

    #[test]
    fn actor_stops_on_command_even_while_idle() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: Some("gpt-x".into()),
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");
        assert_eq!(optimistic[0].content, "hello");
        assert_eq!(optimistic[0].server_pending_id, None);
        assert_eq!(optimistic[0].store_item_id, None);

        let stamped = expect_pending_user_changed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(state, ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "new".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 2);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");
        assert_eq!(optimistic[0].content, "carried");
        assert_eq!(optimistic[1].pending_id, "lens_pend_2");
        assert_eq!(optimistic[1].content, "new");

        let stamped = expect_pending_user_changed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);

        let rolled_back = expect_pending_user_changed(&feed_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed {
                lens_pending_id,
                content,
                ..
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
                assert_eq!(content, "hello");
            }
            other => panic!("expected SendFailed, got {other:?}"),
        }

        // Actor survives — still processes events after a network rollback.
        ev_tx.send(status_running_event()).unwrap();
        assert!(feed_rx.recv_blocking().is_ok());
        handle.stop_and_join();
    }

    #[test]
    fn send_auth401_holds_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Auth { status: 401 });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].pending_id, "lens_pend_1");

        // No rollback emit — bubble stays resident.
        assert!(feed_rx.try_recv().is_err());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendPending { lens_pending_id }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
            }
            other => panic!("expected SendPending, got {other:?}"),
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);
        assert!(
            feed_rx.try_recv().is_err(),
            "5xx keeps bubble — no rollback emit"
        );

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendPending { .. }) => {}
            other => panic!("expected SendPending, got {other:?}"),
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&feed_rx);
        let rolled_back = expect_pending_user_changed(&feed_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendDenied { .. }) => {}
            other => panic!("expected SendDenied, got {other:?}"),
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&feed_rx);
        let rolled_back = expect_pending_user_changed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "blocked".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&feed_rx);
        let rolled_back = expect_pending_user_changed(&feed_rx);
        assert!(rolled_back.is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendDenied {
                lens_pending_id,
                content,
                reason,
            }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
                assert_eq!(content, "blocked");
                assert_eq!(reason.as_deref(), Some("policy"));
            }
            other => panic!("expected SendDenied, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_fate_network_error_carries_content_and_clears_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::network_for_test());
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "typed text".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&feed_rx);
        assert!(expect_pending_user_changed(&feed_rx).is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { content, .. }) => {
                assert_eq!(content, "typed text");
            }
            other => panic!("expected SendFailed, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_fate_server503_emits_pending_and_retains_bubble() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Server {
            status: 503,
            body: serde_json::json!({}),
        });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "held".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].content, "held");
        assert!(
            feed_rx.try_recv().is_err(),
            "503 retains bubble — no rollback emit"
        );

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendPending { lens_pending_id }) => {
                assert_eq!(lens_pending_id, "lens_pend_1");
            }
            other => panic!("expected SendPending, got {other:?}"),
        }

        handle.stop_and_join();
    }

    #[test]
    fn send_fate_auth403_emits_denied_with_content() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = MockApi::fail(ClientError::Auth { status: 403 });
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "forbidden".into(),
                model_override: None,
            })
            .unwrap();

        let _ = expect_pending_user_changed(&feed_rx);
        assert!(expect_pending_user_changed(&feed_rx).is_empty());

        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendDenied { content, .. }) => {
                assert_eq!(content, "forbidden");
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
            let (feed_tx, feed_rx) = async_channel::bounded(64);
            let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);
            handle
                .commands
                .send(SessionCommand::Send {
                    text: "native path".into(),
                    model_override: None,
                })
                .unwrap();
            let _ = expect_pending_user_changed(&feed_rx);
            let stamped = expect_pending_user_changed(&feed_rx);
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
            let (feed_tx, feed_rx) = async_channel::bounded(64);
            let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores_b, test_clock(), api);
            handle
                .commands
                .send(SessionCommand::Send {
                    text: "non-native path".into(),
                    model_override: None,
                })
                .unwrap();
            let _ = expect_pending_user_changed(&feed_rx);
            let stamped = expect_pending_user_changed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hello".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&feed_rx);
        let _ = expect_pending_user_changed(&feed_rx);
        let _ = handle.outcomes.recv_blocking().unwrap();

        ev_tx
            .send(ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
                cleared_pending_id: None,
            }))
            .unwrap();

        let cleared = expect_pending_user_changed(&feed_rx);
        assert!(cleared.is_empty(), "bubble removed after consumed");

        handle.stop_and_join();
    }

    #[test]
    fn terminal_disconnect_exits_actor() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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

        handle.join_without_stop();
    }

    #[test]
    fn disconnect_unauthorized_parks_and_exits() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            recv_detailed(&feed_rx),
            StreamUpdate::Disconnected(DisconnectReason::Unauthorized)
        ));

        handle.join_without_stop();
    }

    #[test]
    fn disconnect_session_failed_parks_actor() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            recv_detailed(&feed_rx),
            StreamUpdate::Disconnected(DisconnectReason::SessionFailed)
        ));

        handle.join_without_stop();
    }

    #[test]
    fn disconnect_retries_exhausted_parks_actor() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            recv_detailed(&feed_rx),
            StreamUpdate::Disconnected(DisconnectReason::RetriesExhausted)
        ));

        handle.join_without_stop();
    }

    #[test]
    fn persist_error_drains_before_terminal_stop() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = failing_transcript_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            ActorOutcome::Parked {
                reason: ParkReason::Forbidden,
            } => {}
            other => panic!("expected Parked Forbidden after PersistError, got {other:?}"),
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
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            ActorOutcome::Parked {
                reason: ParkReason::Forbidden,
            } => {}
            other => panic!("expected Parked Forbidden, got {other:?}"),
        }

        handle.join_without_stop();
    }

    #[test]
    fn disconnect_not_found_stops_with_tombstone_outcome() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
            ActorOutcome::Parked {
                reason: ParkReason::NotFound,
            } => {}
            other => panic!("expected Parked NotFound, got {other:?}"),
        }

        handle.join_without_stop();
    }

    #[test]
    fn reconnecting_sets_reconcile_in_flight() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(
            fresh_state(),
            ev_rx,
            feed_tx,
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
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.try_recv() {
            stream_updates.push(u);
        }
        assert!(
            stream_updates.iter().any(|u| matches!(
                u,
                StreamUpdate::Disconnected(DisconnectReason::Unauthorized)
            )),
            "batch must still emit Disconnected to foreground (got {stream_updates:?})"
        );

        handle.join_without_stop();
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            Ok(SessionStatus::Idle)
        }
    }

    /// `send_event` always fails with `send_err`; `fetch_items` gates like [`GateFetchMock`].
    struct FailSendGateFetchMock {
        script: Mutex<VecDeque<Result<ItemList, ClientError>>>,
        fetch_count: Mutex<u32>,
        block_on_fetch: u32,
        entered_tx: std::sync::mpsc::Sender<u32>,
        release_rx: std::sync::mpsc::Receiver<()>,
        send_status: u16,
    }

    impl FailSendGateFetchMock {
        #[allow(clippy::type_complexity)]
        fn with_script(
            send_status: u16,
            script: VecDeque<Result<ItemList, ClientError>>,
            block_on_fetch: u32,
        ) -> (
            Box<dyn SessionApi + Send>,
            std::sync::mpsc::Receiver<u32>,
            std::sync::mpsc::Sender<()>,
        ) {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            (
                Box::new(Self {
                    script: Mutex::new(script),
                    fetch_count: Mutex::new(0),
                    block_on_fetch,
                    entered_tx,
                    release_rx,
                    send_status,
                }),
                entered_rx,
                release_tx,
            )
        }
    }

    impl SessionApi for FailSendGateFetchMock {
        fn send_event(
            &self,
            _id: &SessionId,
            _evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
            Err(ClientError::Server {
                status: self.send_status,
                body: serde_json::json!({}),
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            Ok(SessionStatus::Idle)
        }
    }

    fn expect_parked_session_failed(handle: &ActorHandle) {
        loop {
            match handle.outcomes.recv_blocking().unwrap() {
                ActorOutcome::Parked {
                    reason: ParkReason::SessionFailed,
                } => return,
                ActorOutcome::PersistError { .. } | ActorOutcome::TransportChanged { .. } => {}
                other => panic!("expected Parked(SessionFailed), got {other:?}"),
            }
        }
    }

    fn drain_outcomes_without_send_lost(handle: &ActorHandle) {
        while let Ok(outcome) = handle.outcomes.try_recv() {
            assert!(
                !matches!(outcome, ActorOutcome::SendLost { .. }),
                "must not emit SendLost on aborted catch-up: {outcome:?}"
            );
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            Ok(SessionStatus::Idle)
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        ev_tx.send(status_running_event()).unwrap();
        let _ = feed_rx.recv_blocking().expect("running status delta");

        handle
            .commands
            .send(SessionCommand::Send {
                text: "while running".into(),
                model_override: None,
            })
            .unwrap();

        let optimistic = expect_pending_user_changed(&feed_rx);
        assert_eq!(optimistic.len(), 1);
        assert_eq!(optimistic[0].content, "while running");

        let stamped = expect_pending_user_changed(&feed_rx);
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
        let cleared = expect_pending_user_changed(&feed_rx);
        assert!(
            cleared.is_empty(),
            "InputConsumed clears bubble while running"
        );

        // Stream stays alive — further events and Stop still work.
        ev_tx.send(status_running_event()).unwrap();
        assert!(feed_rx.recv_blocking().is_ok());
        handle.stop_and_join();

        // Network fail while running: rollback but no stream teardown.
        let _dir2 = tempfile::tempdir().unwrap();
        let stores2 = test_stores(_dir2.path());
        seed_connection(&stores2);
        let (api2, _mock2) = MockApi::fail(ClientError::network_for_test());
        let (ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let (feed_tx2, feed_rx2) = async_channel::bounded(64);
        let handle2 = spawn_actor(fresh_state(), ev_rx2, feed_tx2, stores2, test_clock(), api2);
        ev_tx2.send(status_running_event()).unwrap();
        let _ = feed_rx2.recv_blocking();
        handle2
            .commands
            .send(SessionCommand::Send {
                text: "will fail".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&feed_rx2);
        assert!(expect_pending_user_changed(&feed_rx2).is_empty());
        match handle2.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Command(CommandOutcome::SendFailed { .. }) => {}
            other => panic!("expected SendFailed, got {other:?}"),
        }
        ev_tx2.send(status_running_event()).unwrap();
        assert!(feed_rx2.recv_blocking().is_ok());
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "mid-reconnect".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&feed_rx);
        let stamped = expect_pending_user_changed(&feed_rx);
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
        let reconnect_delta = recv_detailed(&feed_rx);
        assert!(matches!(
            reconnect_delta,
            StreamUpdate::Reconnecting { attempt: 1 }
        ));
        while let Ok(frame) = feed_rx.try_recv() {
            match frame {
                ActorFeed::Detailed(u) => assert!(
                    !matches!(u, StreamUpdate::PendingUserChanged(ref v) if v.is_empty()),
                    "bubble must not be cleared before snapshot reconcile"
                ),
                // Detailed-mode test: a Summary here would silently end the drain and
                // let a mis-wrapped clear-frame slip past (codex final-review C2).
                ActorFeed::Summary(_) => {
                    panic!("unexpected Summary in a Detailed-mode drain")
                }
            }
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
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.recv_blocking() {
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor_dual(
            fresh_state(),
            ev_rx,
            feed_tx,
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

        // FIFO frames: spawn seed, then the Demote emit, then the Send emit
        // (commands are processed in order). Drain the first two so the final
        // assertion actually verifies the Send-path Summary, not the seed.
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "spawn-in-Summary seed"
        );
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "emit-on-Demote"
        );
        assert!(
            matches!(feed_rx.recv_blocking().unwrap(), ActorFeed::Summary(_)),
            "summary emitted on send"
        );

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

        // Dropped feed receiver — SummaryConsumerGone on the next Summary emit.
        let _dir2 = tempfile::tempdir().unwrap();
        let stores2 = test_stores(_dir2.path());
        seed_connection(&stores2);
        let (ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let (feed_tx2, feed_rx2) = async_channel::bounded(64);
        drop(feed_rx2);
        let handle2 = spawn_actor_dual(
            fresh_state(),
            ev_rx2,
            feed_tx2,
            OutputMode::Summary,
            stores2,
            test_clock(),
            noop_api(),
        );
        ev_tx2.send(status_running_event()).unwrap();
        match handle2.outcomes.recv_blocking().unwrap() {
            ActorOutcome::SummaryConsumerGone => {}
            other => panic!("expected SummaryConsumerGone, got {other:?}"),
        }
        handle2.commands.send(SessionCommand::Stop).unwrap();
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

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
        let _ = expect_pending_user_changed(&feed_rx);
        let _ = expect_pending_user_changed(&feed_rx);
    }

    #[test]
    fn sleep_when_quiescent_flushes_slept_and_stops() {
        use crate::domain::scalars::SessionLifecycle;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lens.db");
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

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
            feed_rx.recv_blocking().is_ok(),
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let (api, mock) = MockApi::succeed_with_ack(SendEventAck {
            queued: true,
            ..Default::default()
        });
        let mut state = fresh_state();
        state.status = SessionStatusValue::Running;
        let handle = spawn_actor(state, ev_rx, feed_tx, stores, test_clock(), api);

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
        assert!(feed_rx.recv_blocking().is_ok());
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
            !is_quiesced(&s, &ActorTransport::Reconnecting, false),
            "reconnecting is not quiesced"
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
        transcript.upsert_item(ordinal, &item, false).unwrap();
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

    fn item_list_from_user_messages(entries: &[(&str, &str)], has_more: bool) -> ItemList {
        let data: Vec<serde_json::Value> = entries
            .iter()
            .map(|(id, text)| {
                serde_json::json!({
                    "id": id,
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "data": data,
            "has_more": has_more,
        }))
        .expect("user item list fixture")
    }

    fn seed_user_message_item(
        transcript: &dyn TranscriptStore,
        ordinal: i64,
        id: &str,
        text: &str,
    ) {
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
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "input_text".into(),
                    text: Some(text.into()),
                    data: serde_json::Value::Null,
                }],
            },
        };
        transcript.upsert_item(ordinal, &item, false).unwrap();
    }

    fn mock_fail_with_fetch(
        err: ClientError,
        fetch_script: VecDeque<Result<ItemList, ClientError>>,
    ) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
        let mock = Arc::new(MockApi {
            send_script: Mutex::new(VecDeque::from([Err(err)])),
            fetch_script: Mutex::new(fetch_script),
            status_script: Mutex::new(VecDeque::new()),
            last_evt: Mutex::new(None),
        });
        (Box::new(Arc::clone(&mock)), mock)
    }

    fn mock_fail_n_with_fetch(
        status: u16,
        send_count: usize,
        fetch_script: VecDeque<Result<ItemList, ClientError>>,
    ) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
        let send_script: VecDeque<_> = (0..send_count)
            .map(|_| {
                Err(ClientError::Server {
                    status,
                    body: serde_json::json!({}),
                })
            })
            .collect();
        let mock = Arc::new(MockApi {
            send_script: Mutex::new(send_script),
            fetch_script: Mutex::new(fetch_script),
            status_script: Mutex::new(VecDeque::new()),
            last_evt: Mutex::new(None),
        });
        (Box::new(Arc::clone(&mock)), mock)
    }

    fn expect_send_lost(handle: &ActorHandle, content: &str) {
        loop {
            match handle.outcomes.recv_blocking().unwrap() {
                ActorOutcome::SendLost { content: c, .. } => {
                    assert_eq!(c, content);
                    return;
                }
                ActorOutcome::TransportChanged { .. } => {}
                other => panic!("expected SendLost, got {other:?}"),
            }
        }
    }

    #[test]
    fn held_reconnect_no_inputs_no_delta_emits_send_lost() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = mock_fail_with_fetch(
            ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            },
            VecDeque::from([Ok(empty_item_list()), Ok(empty_item_list())]),
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "gone".into(),
                model_override: None,
            })
            .unwrap();
        let held = expect_pending_user_changed(&feed_rx);
        assert_eq!(held.len(), 1);
        assert!(held[0].server_pending_id.is_none());
        let _ = handle.outcomes.recv_blocking();

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        expect_send_lost(&handle, "gone");
        let cleared = recv_pending_user_changed_or_none(&feed_rx).expect("cleared pending_user");
        assert!(cleared.is_empty());
        handle.stop_and_join();
    }

    #[test]
    fn held_reconnect_folded_same_content_still_send_lost() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_user_message_item(&*stores.transcript, 0, "item_0", "lost send");

        let folded_page = item_list_from_user_messages(&[("item_0", "lost send")], false);
        let (api, _mock) = mock_fail_with_fetch(
            ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            },
            VecDeque::from([Ok(empty_item_list()), Ok(folded_page)]),
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "lost send".into(),
                model_override: None,
            })
            .unwrap();
        let held = expect_pending_user_changed(&feed_rx);
        assert_eq!(held.len(), 1);
        let _ = handle.outcomes.recv_blocking();

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        expect_send_lost(&handle, "lost send");
        let cleared = recv_pending_user_changed_or_none(&feed_rx).expect("cleared pending_user");
        assert!(cleared.is_empty());
        handle.stop_and_join();
    }

    #[test]
    fn held_reconnect_stamps_from_plumbed_snapshot_pending_input_content() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = mock_fail_with_fetch(
            ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            },
            VecDeque::from([Ok(empty_item_list()), Ok(empty_item_list())]),
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "stamp me".into(),
                model_override: None,
            })
            .unwrap();
        let held = expect_pending_user_changed(&feed_rx);
        assert_eq!(held.len(), 1);
        assert!(held[0].server_pending_id.is_none());
        let _ = handle.outcomes.recv_blocking();

        let snap = snapshot_fixture(serde_json::json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "pending_inputs": [{"pending_id": "p9", "content": "stamp me"}],
            "items": []
        }));
        ev_tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .unwrap();
        while let Ok(frame) = feed_rx.try_recv() {
            // Detailed-mode setup drain: fail loud on a Summary rather than silently
            // truncate (codex final-review C2).
            assert!(
                matches!(frame, ActorFeed::Detailed(_)),
                "unexpected Summary in a Detailed-mode setup drain"
            );
        }

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        let stamped = recv_pending_user_changed_or_none(&feed_rx).expect("stamped pending_user");
        assert_eq!(stamped.len(), 1);
        assert_eq!(stamped[0].server_pending_id.as_deref(), Some("p9"));
        assert_eq!(stamped[0].content, "stamp me");
        handle.stop_and_join();
    }

    #[test]
    fn held_reconnect_without_fresh_snapshot_does_not_stamp_from_prior_episode() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let (api, _mock) = mock_fail_n_with_fetch(
            503,
            2,
            VecDeque::from([
                Ok(empty_item_list()),
                Ok(empty_item_list()),
                Ok(empty_item_list()),
            ]),
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        handle
            .commands
            .send(SessionCommand::Send {
                text: "stale".into(),
                model_override: None,
            })
            .unwrap();
        let _ = expect_pending_user_changed(&feed_rx);
        let _ = handle.outcomes.recv_blocking();

        let snap = snapshot_fixture(serde_json::json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "pending_inputs": [{"pending_id": "p_stale", "content": "stale"}],
            "items": []
        }));
        ev_tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .unwrap();
        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        let stamped = recv_pending_user_changed_or_none(&feed_rx).expect("first episode stamp");
        assert_eq!(stamped[0].server_pending_id.as_deref(), Some("p_stale"));
        while handle.outcomes.try_recv().is_ok() {}

        handle
            .commands
            .send(SessionCommand::Send {
                text: "stale".into(),
                model_override: None,
            })
            .unwrap();
        let held2 = expect_pending_user_changed(&feed_rx);
        let new_held = held2
            .iter()
            .find(|b| b.pending_id == "lens_pend_2")
            .expect("second held bubble");
        assert!(new_held.server_pending_id.is_none());
        let _ = handle.outcomes.recv_blocking();

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        loop {
            match handle.outcomes.recv_blocking().unwrap() {
                ActorOutcome::SendLost {
                    lens_pending_id,
                    content,
                } => {
                    assert_eq!(lens_pending_id, "lens_pend_2");
                    assert_eq!(content, "stale");
                    break;
                }
                ActorOutcome::TransportChanged { .. } => {}
                other => panic!("expected SendLost for lens_pend_2, got {other:?}"),
            }
        }
        let after = recv_pending_user_changed_or_none(&feed_rx).expect("after ep2 reconcile");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].pending_id, "lens_pend_1");
        assert_eq!(after[0].server_pending_id.as_deref(), Some("p_stale"));
        handle.stop_and_join();
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
            stores.transcript.store_frontier().unwrap(),
            Some((2, ItemId::new("item_2")))
        );

        let page1 = item_list_from_messages(&["item_3", "item_4"], true);
        let page2 = item_list_from_messages(&["item_5"], false);
        let (api, _mock) = MockApi::with_fetch_script(VecDeque::from([Ok(page1), Ok(page2)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_6","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();

        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        let mut saw_catchup_watermark = false;
        loop {
            let u = recv_detailed(&feed_rx);
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
            stores.transcript.store_frontier().unwrap(),
            Some((0, ItemId::new("item_0")))
        );

        // Spawn catch-up consumes page 1 (empty); Reconnected catch-up consumes page 2.
        let history_page = item_list_from_messages(&["item_1", "item_2"], false);
        let (api, _mock) =
            MockApi::with_fetch_script(VecDeque::from([Ok(empty_item_list()), Ok(history_page)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        // spawn_actor returns only after spawn catch-up completes (empty page).
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_3","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();

        let mut saw_deferred_commit = false;
        while let Ok(frame) = feed_rx.recv_blocking() {
            if let ActorFeed::Detailed(StreamUpdate::TranscriptAdvanced {
                committed_ordinal: 3,
            }) = frame
            {
                saw_deferred_commit = true;
                break;
            }
        }
        assert!(
            saw_deferred_commit,
            "finish_deferred_transcript_commit must emit ActorFeed::Detailed(TranscriptAdvanced) on the unified feed"
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

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

        loop {
            let u = recv_detailed(&feed_rx);
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

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
            match feed_rx.recv_blocking() {
                Ok(ActorFeed::Detailed(StreamUpdate::StatusChanged(
                    SessionStatusValue::Running,
                ))) => break,
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
            feed_rx.recv_blocking().is_ok(),
            "actor must survive and keep processing events"
        );

        handle.stop_and_join();
    }

    #[test]
    fn catchup_fetch_items_err_parks_without_send_lost_or_deferred_commit() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);

        let has_more_empty = serde_json::from_str(r#"{"data":[],"has_more":true}"#)
            .expect("empty page with has_more");
        let (api, fetch_done_rx, release_fetch2) = FailSendGateFetchMock::with_script(
            503,
            VecDeque::from([
                Ok(empty_item_list()),
                Ok(has_more_empty),
                Err(ClientError::Server {
                    status: 500,
                    body: serde_json::json!({}),
                }),
            ]),
            2,
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("spawn catch-up fetch"),
            1
        );

        handle
            .commands
            .send(SessionCommand::Send {
                text: "gone".into(),
                model_override: None,
            })
            .unwrap();
        let held = expect_pending_user_changed(&feed_rx);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].content, "gone");
        let _ = handle.outcomes.recv_blocking();

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("reconnect catch-up must block on fetch #2"),
            2
        );
        ev_tx
            .send(parse_response(
                "response.output_item.done",
                r#"{"item":{"id":"item_live","type":"message","role":"assistant","content":[{"type":"output_text","text":"live"}]}}"#,
            ))
            .unwrap();
        release_fetch2.send(()).expect("release blocked fetch");

        expect_parked_session_failed(&handle);
        drain_outcomes_without_send_lost(&handle);
        handle.join_without_stop();

        let reopened = SqliteTranscriptStore::open(
            &dir.path().join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(
            !ids.contains(&"item_live"),
            "deferred live tail must not commit after fetch_items Err"
        );
    }

    #[test]
    fn nested_reconnect_catchup_accum_scoped_per_episode() {
        let _dir = tempfile::tempdir().unwrap();
        let stores = test_stores(_dir.path());
        seed_connection(&stores);

        let ep_a_hi = item_list_from_user_messages(&[("item_ep_a", "hi")], true);
        let has_more_empty = serde_json::from_str(r#"{"data":[],"has_more":true}"#)
            .expect("empty page with has_more");
        let (api, fetch_done_rx, release_fetch3) = FailSendGateFetchMock::with_script(
            503,
            VecDeque::from([
                Ok(empty_item_list()),
                Ok(ep_a_hi),
                Ok(has_more_empty),
                Ok(empty_item_list()),
                Ok(empty_item_list()),
            ]),
            3,
        );

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("spawn catch-up fetch"),
            1
        );

        handle
            .commands
            .send(SessionCommand::Send {
                text: "hi".into(),
                model_override: None,
            })
            .unwrap();
        let held = expect_pending_user_changed(&feed_rx);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].content, "hi");
        let _ = handle.outcomes.recv_blocking();

        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();

        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("episode-A catch-up page 1"),
            2
        );
        assert_eq!(
            fetch_done_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("episode-A catch-up must block on fetch #3"),
            3
        );

        let snap = snapshot_fixture(serde_json::json!({
            "id": "conv_1",
            "status": "running",
            "agent_id": "ag_9",
            "created_at": 1_700_000_000,
            "pending_inputs": [],
            "items": []
        }));
        ev_tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .unwrap();
        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();
        release_fetch3.send(()).expect("release blocked fetch");

        expect_send_lost(&handle, "hi");
        let cleared = recv_pending_user_changed_or_none(&feed_rx).expect("held bubble cleared");
        assert!(cleared.is_empty());
        handle.stop_and_join();
    }

    fn item_provisional_flag(db_path: &Path, item_id: &str) -> i64 {
        item_provisional_opt(db_path, item_id)
            .unwrap_or_else(|| panic!("provisional lookup for {item_id}: row absent"))
    }

    /// `provisional` flag for `item_id`, or `None` if the row is not yet on disk.
    /// Safe to call against the live actor's WAL db from a fresh reader connection.
    fn item_provisional_opt(db_path: &Path, item_id: &str) -> Option<i64> {
        let conn = rusqlite::Connection::open(db_path).expect("open transcript db");
        match conn.query_row(
            "SELECT provisional FROM items WHERE item_id = ?1",
            [item_id],
            |r| r.get(0),
        ) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("provisional lookup for {item_id}: {e}"),
        }
    }

    /// Extract the FunctionCall/FunctionCallOutput domain items produced by reducing
    /// the golden live stream — the tool call as the actor's live path materializes it.
    fn golden_live_tool_items() -> Vec<Item> {
        let clock = test_clock();
        let mut state = fresh_state();
        for ev in decode_all(include_bytes!(
            "../../tests/fixtures/d30/tool_fold.stream.sse"
        )) {
            let _ = crate::reduce::reduce(&mut state, &ev, clock.as_ref());
        }
        state
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.kind,
                    ItemKind::FunctionCall { .. } | ItemKind::FunctionCallOutput { .. }
                )
            })
            .map(|i| i.as_ref().clone())
            .collect()
    }

    /// Store-layer D30 fold sentinel on real 0.5.1 bytes. Drives the exact fold path
    /// the actor's catch-up uses (`wire_to_domain_item` + `live_key_for_store_item` +
    /// `reconcile_store_item`) against a live provisional tool call — proving the
    /// two-id-space fold on captured bytes, isolated from catch-up ordering.
    ///
    /// Fixtures: tests/fixtures/d30/tool_fold.{stream.sse,items.json}, captured
    /// 2026-07-12 from omnigent 0.5.1 (source HEAD 08285468). The live stream splits
    /// the call across fc_1a36 (in_progress) -> fc_7ad9 (completed) sharing one
    /// call_id; /items carries the same call_id under a different store id (fc_9bb8).
    #[test]
    fn d30_golden_reconcile_folds_tool_call() {
        let clock = test_clock();
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        let db_path = dir.path().join("conv_1.db");

        // LIVE: commit the streamed tool call as PROVISIONAL (as the actor does).
        let live_items = golden_live_tool_items();
        assert!(
            live_items
                .iter()
                .any(|i| i.id.as_str() == "fc_7ad94742b335"),
            "stream must materialize the completed live function_call fc_7ad94742b335"
        );
        for (ord, item) in live_items.iter().enumerate() {
            stores
                .transcript
                .upsert_item(ord as i64, item, true)
                .unwrap();
        }
        assert_eq!(
            item_provisional_flag(&db_path, "fc_7ad94742b335"),
            1,
            "live tool-call committed provisional before the fold (fold is load-bearing)"
        );

        // CATCH-UP: reconcile the /items canonical tool rows — folds by call_id.
        let items: ItemList = serde_json::from_str(include_str!(
            "../../tests/fixtures/d30/tool_fold.items.json"
        ))
        .expect("golden /items JSON");
        for wire in items.items() {
            let Some(domain) = wire_to_domain_item(wire, clock.as_ref()) else {
                continue;
            };
            if !matches!(
                domain.kind,
                ItemKind::FunctionCall { .. } | ItemKind::FunctionCallOutput { .. }
            ) {
                continue;
            }
            let live_key = live_key_for_store_item(&domain);
            stores
                .transcript
                .reconcile_store_item(&domain, &live_key)
                .unwrap();
        }

        assert_d30_tool_fold(&db_path);
    }

    /// End-to-end D30 fold on real bytes through the FULL actor: startup catch-up
    /// seeds the pre-turn history as canonical, the live stream commits the tool call
    /// provisional, then a reconnect DELTA catch-up (only the new turn's rows) folds
    /// the provisional fc_* into the /items canonical. This is the production path
    /// (attach -> live turn -> reconnect); the catch-up here is a genuine delta, so
    /// the provisional row exists before the canonical arrives.
    #[test]
    fn d30_golden_end_to_end_attach_turn_reconnect_fold() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        let db_path = dir.path().join("conv_1.db");

        // Partition the golden /items at the turn boundary: history (everything up to
        // the first new-turn row) vs delta (the tool call + its surrounding messages).
        let (history, delta) = split_golden_items_at("msg_f6bac737d81e4fe4aebd48ad654cdbdc");
        let (api, _mock) = MockApi::with_fetch_script(VecDeque::from([Ok(history), Ok(delta)]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(1);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        // spawn_actor's startup catch-up consumes `history` -> canonical pre-turn state.
        let handle = spawn_actor(fresh_state(), ev_rx, feed_tx, stores, test_clock(), api);

        // LIVE turn: stream commits the tool call provisional above the history frontier.
        for ev in decode_all(include_bytes!(
            "../../tests/fixtures/d30/tool_fold.stream.sse"
        )) {
            ev_tx.send(ev).unwrap();
        }
        // Durable sync: block until the live provisional fc is on disk (acceptance !=
        // commit under greedy drain), using emitted updates as the wakeup.
        loop {
            if item_provisional_opt(&db_path, "fc_7ad94742b335") == Some(1) {
                break;
            }
            match feed_rx.recv_blocking() {
                Ok(_) => continue,
                // The actor thread can end (feed closes) right after its final commit;
                // disk is the real invariant, so re-check it before declaring failure
                // (avoids a parallel-load race where the close is observed before recv).
                Err(_) => {
                    assert_eq!(
                        item_provisional_opt(&db_path, "fc_7ad94742b335"),
                        Some(1),
                        "actor exited before committing the live provisional fc"
                    );
                    break;
                }
            }
        }

        // RECONNECT: delta catch-up fetches only the new-turn rows -> folds.
        ev_tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .unwrap();
        // Wait until the canonical folded row is durable on disk.
        loop {
            if item_provisional_opt(&db_path, "fc_9bb8ae52357c40a2a2f696e37b81d681") == Some(0) {
                break;
            }
            match feed_rx.recv_blocking() {
                Ok(_) => continue,
                // See the live-commit loop above: the actor can end right after the
                // fold commit; re-check disk before failing.
                Err(_) => {
                    assert_eq!(
                        item_provisional_opt(&db_path, "fc_9bb8ae52357c40a2a2f696e37b81d681"),
                        Some(0),
                        "actor exited before the reconnect delta fold landed"
                    );
                    break;
                }
            }
        }
        handle.stop_and_join();

        assert_d30_tool_fold(&db_path);
    }

    /// Deserialize the golden /items and split `data` at the first item whose id is
    /// `boundary_id` (inclusive on the delta side): everything before is the pre-turn
    /// history page; `boundary_id` onward is the reconnect delta page.
    fn split_golden_items_at(boundary_id: &str) -> (ItemList, ItemList) {
        let raw = include_str!("../../tests/fixtures/d30/tool_fold.items.json");
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        let data = v["data"].as_array().unwrap().clone();
        let cut = data
            .iter()
            .position(|it| it["id"].as_str() == Some(boundary_id))
            .expect("boundary id present in golden /items");
        let (hist, delta) = data.split_at(cut);
        let mut mk = |items: &[serde_json::Value]| -> ItemList {
            v["data"] = serde_json::Value::Array(items.to_vec());
            v["has_more"] = serde_json::Value::Bool(false);
            serde_json::from_value(v.clone()).unwrap()
        };
        (mk(hist), mk(delta))
    }

    /// Shared D30 fold assertion: exactly one function_call (= /items canonical id,
    /// provisional=0) and one function_call_output, with the live provisional fc_*/fco_*
    /// ids folded away.
    fn assert_d30_tool_fold(db_path: &Path) {
        let reopened = SqliteTranscriptStore::open(
            db_path,
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        let rows = reopened.load_items().unwrap().rows;
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str().to_string()).collect();

        let fc: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.kind, ItemKind::FunctionCall { .. }))
            .collect();
        assert_eq!(fc.len(), 1, "one function_call after fold; ids: {ids:?}");
        assert_eq!(
            fc[0].id.as_str(),
            "fc_9bb8ae52357c40a2a2f696e37b81d681",
            "/items canonical fc_* id wins the fold"
        );
        assert_eq!(
            item_provisional_flag(db_path, "fc_9bb8ae52357c40a2a2f696e37b81d681"),
            0,
            "folded function_call is durable (provisional=0)"
        );
        assert!(
            !ids.iter()
                .any(|id| id == "fc_1a365818d5b6" || id == "fc_7ad94742b335"),
            "live provisional fc_* ids folded away; ids: {ids:?}"
        );

        let fco: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.kind, ItemKind::FunctionCallOutput { .. }))
            .collect();
        assert_eq!(
            fco.len(),
            1,
            "one function_call_output after fold; ids: {ids:?}"
        );
        assert_eq!(
            fco[0].id.as_str(),
            "fco_7bd2ed51b55d42748c92428c30c8fdd1",
            "/items canonical fco_* id wins the fold"
        );
        assert!(
            !ids.iter().any(|id| id == "fco_a51bcc02b848"),
            "live provisional fco_* id folded away; ids: {ids:?}"
        );
    }
}
