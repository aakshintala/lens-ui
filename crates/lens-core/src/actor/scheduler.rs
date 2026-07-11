//! §9 skeletal fleet scheduler — spawn-on-wake / `Sleep`-command registry seam.
//! Timer, LRU, and focus policy are deferred; this module only routes lifecycle.

use crate::actor::api::SessionApi;
use crate::actor::runloop::{ActorHandle, ActorStores, SessionCommand, spawn_actor};
use crate::clock::Clock;
use crate::domain::ids::{ConnectionId, SessionId};
use crate::reduce::StreamUpdate;
use crossbeam_channel::Receiver;
use lens_client::stream::ServerStreamEvent;
use std::collections::HashMap;

/// Thin registry for running session actors — §9 policy hooks land later.
pub struct FleetScheduler {
    registry: HashMap<SessionId, ActorHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FleetSchedulerError {
    SessionNotFound,
    SessionNotRunning,
    AlreadyRunning,
    Persist(String),
    CommandSendFailed,
}

impl FleetScheduler {
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
        }
    }

    /// Respawn a session actor from disk control scalars. `spawn_actor` seeds
    /// `next_ordinal` from `transcript.frontier()` and runs forward catch-up.
    #[allow(clippy::too_many_arguments)]
    pub fn wake(
        &mut self,
        conn: &ConnectionId,
        session_id: &SessionId,
        events: Receiver<ServerStreamEvent>,
        updates: async_channel::Sender<StreamUpdate>,
        stores: ActorStores,
        clock: Box<dyn Clock + Send>,
        api: Box<dyn SessionApi + Send>,
    ) -> Result<ActorHandle, FleetSchedulerError> {
        if self.registry.contains_key(session_id) {
            return Err(FleetSchedulerError::AlreadyRunning);
        }
        let state =
            crate::persist::ControlStore::load_session(stores.control.as_ref(), conn, session_id)
                .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?
                .ok_or(FleetSchedulerError::SessionNotFound)?;
        let handle = spawn_actor(state, events, updates, stores, clock, api);
        self.registry.insert(session_id.clone(), handle);
        Ok(self.take_handle(session_id).expect("just inserted"))
    }

    /// Route durable sleep to a running actor's command channel.
    pub fn sleep(&mut self, session_id: &SessionId) -> Result<(), FleetSchedulerError> {
        let handle = self
            .registry
            .get(session_id)
            .ok_or(FleetSchedulerError::SessionNotRunning)?;
        handle
            .commands
            .send(SessionCommand::Sleep)
            .map_err(|_| FleetSchedulerError::CommandSendFailed)
    }

    /// Re-register a handle after `wake` (or for tests that took ownership temporarily).
    pub fn register(&mut self, session_id: SessionId, handle: ActorHandle) {
        self.registry.insert(session_id, handle);
    }

    /// Remove and return a running handle (e.g. to await outcomes / join).
    pub fn take_handle(&mut self, session_id: &SessionId) -> Option<ActorHandle> {
        self.registry.remove(session_id)
    }

    pub fn is_running(&self, session_id: &SessionId) -> bool {
        self.registry.contains_key(session_id)
    }
}

impl Default for FleetScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::api::SessionApi;
    use crate::actor::outcome::ActorOutcome;
    use crate::clock::ManualClock;
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::{Role, SessionLifecycle};
    use crate::persist::{
        ConnectionRecord, ControlStore, SqliteControlStore, SqliteTranscriptStore, TranscriptStore,
    };
    use crate::reduce::testutil::fresh_state;
    use lens_client::error::ClientError;
    use lens_client::sessions::{ItemList, SendEventAck, SessionEventInput};
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn test_clock() -> Box<dyn Clock + Send> {
        Box::new(ManualClock::new(1_700_000_000_000))
    }

    struct MockApi {
        send_script: Mutex<VecDeque<Result<SendEventAck, ClientError>>>,
        fetch_script: Mutex<VecDeque<Result<ItemList, ClientError>>>,
    }

    fn empty_item_list() -> ItemList {
        serde_json::from_str(r#"{"data":[],"has_more":false}"#).expect("empty item list")
    }

    impl MockApi {
        fn with_scripts(
            send_script: VecDeque<Result<SendEventAck, ClientError>>,
            fetch_script: VecDeque<Result<ItemList, ClientError>>,
        ) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(send_script),
                fetch_script: Mutex::new(fetch_script),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }
    }

    impl SessionApi for Arc<MockApi> {
        fn send_event(
            &self,
            _id: &SessionId,
            _evt: &SessionEventInput,
        ) -> Result<SendEventAck, ClientError> {
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

    fn seed_message_item(transcript: &dyn TranscriptStore, ordinal: i64, id: &str, text: &str) {
        let item = Item {
            id: crate::domain::ids::ItemId::new(id),
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

    fn seed_slept_session(stores: &ActorStores) {
        let mut state = fresh_state();
        state.lifecycle = SessionLifecycle::Slept;
        state.title = Some("slept-roundtrip".into());
        state.reasoning_effort = Some("high".into());
        stores
            .control
            .upsert_session(&state, 1_700_000_000_000)
            .unwrap();
        for (id, ord) in [("item_0", 0), ("item_1", 1), ("item_2", 2)] {
            seed_message_item(&*stores.transcript, ord, id, id);
        }
    }

    #[test]
    fn wake_roundtrip_sleep_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("lens.db");
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_slept_session(&stores);

        let tail_page = item_list_from_messages(&["item_3", "item_4"], false);
        let stop_ack = SendEventAck {
            queued: true,
            ..Default::default()
        };
        let (api, _mock) = MockApi::with_scripts(
            VecDeque::from([Ok(stop_ack)]),
            VecDeque::from([Ok(tail_page)]),
        );

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (up_tx, up_rx) = async_channel::bounded(64);
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let mut scheduler = FleetScheduler::new();
        let handle = scheduler
            .wake(&conn, &sid, ev_rx, up_tx, stores, test_clock(), api)
            .expect("wake from slept disk");

        let mut saw_catchup_tail = false;
        while let Ok(update) = up_rx.recv_blocking() {
            if matches!(
                update,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 3
                } | StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 4
                }
            ) {
                saw_catchup_tail = true;
            }
            if matches!(
                update,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 4
                }
            ) {
                break;
            }
        }
        assert!(
            saw_catchup_tail,
            "catch-up must materialize tail ordinals 3..=4 after frontier 2"
        );

        handle.commands.send(SessionCommand::Promote).unwrap();
        match up_rx.recv_blocking().unwrap() {
            StreamUpdate::Rebased(baseline) => {
                assert_eq!(baseline.title.as_deref(), Some("slept-roundtrip"));
                assert_eq!(baseline.reasoning_effort.as_deref(), Some("high"));
                assert_eq!(baseline.lifecycle, SessionLifecycle::Slept);
                assert!(baseline.items.is_empty(), "Rebased is scalars-only");
            }
            other => panic!("expected Rebased after Promote, got {other:?}"),
        }

        scheduler.register(sid.clone(), handle);
        scheduler.sleep(&sid).expect("sleep routed to actor");
        let handle = scheduler
            .take_handle(&sid)
            .expect("handle still registered");
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Slept => {}
            other => panic!("expected Slept outcome, got {other:?}"),
        }
        handle.join_without_stop();

        let control = SqliteControlStore::open(&db_path).unwrap();
        let loaded = control.list_sessions(&conn).unwrap();
        assert_eq!(loaded.rows[0].lifecycle, SessionLifecycle::Slept);

        let transcript =
            SqliteTranscriptStore::open(&dir.path().join("conv_1.db"), &conn, &sid).unwrap();
        let rows = transcript.load_items().unwrap().rows;
        assert_eq!(rows.len(), 5, "item_0..item_4 on disk");
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["item_0", "item_1", "item_2", "item_3", "item_4"]);
        assert!(!scheduler.is_running(&sid), "actor stopped after sleep");
    }
}
