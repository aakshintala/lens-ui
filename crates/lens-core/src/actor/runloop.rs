//! The actor run-loop: `crossbeam::Select` over events + commands, greedy drain,
//! persist write-through, coalesce, emit to the foreground bridge.

use crate::clock::Clock;
use crate::domain::SessionState;
use crate::persist::{ControlStore, TranscriptStore};
use crate::reduce::{StreamUpdate, Updates, reduce};
use crossbeam_channel::{Receiver, Select};
use lens_client::stream::ServerStreamEvent;
use std::collections::HashSet;
use std::mem::Discriminant;
use std::thread::JoinHandle;

/// Commands to the actor thread. Extended in P3-2.
#[derive(Debug)]
pub enum SessionCommand {
    Stop,
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

/// Spawn the actor thread. `events` is the crossbeam receiver from lens-client;
/// `updates` is the async-channel sender to the foreground bridge.
pub fn spawn_actor(
    state: SessionState,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
) -> ActorHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<SessionCommand>(64);
    let join = std::thread::Builder::new()
        .name("lens-actor".into())
        .spawn(move || run(state, events, cmd_rx, updates, stores, clock))
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
    updates: async_channel::Sender<StreamUpdate>,
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
            },
            i if i == ev_idx => match oper.recv(&events) {
                Ok(event) => {
                    let mut batch = reduce(&mut state, &event, clock.as_ref());
                    while let Ok(next) = events.try_recv() {
                        batch.extend(reduce(&mut state, &next, clock.as_ref()));
                    }
                    persist_write_through(&stores, &state, &batch, clock.now_millis());
                    for u in coalesce(batch) {
                        if updates.send_blocking(u).is_err() {
                            return;
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
    let mut touched_scalar = false;
    for u in batch {
        match u {
            StreamUpdate::ItemAppended(item) => {
                // TODO(P3-3): replace items.len() with the owned ordinal cursor once byte-window eviction lands (D11).
                let ordinal = (state.items.len() as i64) - 1;
                let _ = stores.transcript.upsert_item(ordinal, item.as_ref());
            }
            StreamUpdate::ItemUpdated { index, item } => {
                let _ = stores.transcript.upsert_item(*index as i64, item.as_ref());
            }
            StreamUpdate::ScratchChanged(_)
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
    use crate::domain::ids::{ConnectionId, SessionId};
    use crate::persist::{
        ConnectionRecord, SqliteControlStore, SqliteTranscriptStore, TranscriptStore,
    };
    use crate::reduce::testutil::{fresh_state, parse_response};
    use std::path::Path;

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
