//! §9 skeletal fleet scheduler — spawn-on-wake / `Sleep`-command registry seam.
//! Timer, LRU, and focus policy are deferred; this module only routes lifecycle.

use crate::actor::api::SessionApi;
use crate::actor::feed::ActorFeed;
use crate::actor::runloop::{
    ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor_dual,
};
use crate::actor::transport::ParkReason;
use crate::clock::Clock;
use crate::domain::ids::{ConnectionId, SessionId};
use crate::domain::scalars::SessionLifecycle;
use crossbeam_channel::Receiver;
use lens_client::sessions::SessionStatus;
use lens_client::stream::ServerStreamEvent;
use std::collections::HashMap;

/// Thin registry for running session actors — §9 policy hooks land later.
pub struct FleetScheduler {
    registry: HashMap<SessionId, ActorHandle>,
    parked: HashMap<SessionId, ParkReason>,
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
            parked: HashMap::new(),
        }
    }

    /// Respawn a session actor from disk control scalars. `spawn_actor_dual` seeds
    /// `next_ordinal` from `transcript.next_ordinal_seed()` and runs forward catch-up.
    /// The registry retains ownership of the handle until `take_handle`.
    #[allow(clippy::too_many_arguments)]
    pub fn wake(
        &mut self,
        conn: &ConnectionId,
        session_id: &SessionId,
        events: Receiver<ServerStreamEvent>,
        feed: async_channel::Sender<ActorFeed>,
        initial_mode: OutputMode,
        stores: ActorStores,
        clock: Box<dyn Clock + Send>,
        api: Box<dyn SessionApi + Send>,
    ) -> Result<(), FleetSchedulerError> {
        if self.registry.contains_key(session_id) {
            return Err(FleetSchedulerError::AlreadyRunning);
        }
        let mut state =
            crate::persist::ControlStore::load_session(stores.control.as_ref(), conn, session_id)
                .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?
                .ok_or(FleetSchedulerError::SessionNotFound)?;
        state.lifecycle = SessionLifecycle::Active;
        let now = clock.now_millis();
        stores
            .control
            .upsert_session(&state, now)
            .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?;
        let handle = spawn_actor_dual(state, events, feed, initial_mode, stores, clock, api);
        self.registry.insert(session_id.clone(), handle);
        Ok(())
    }

    /// Respawn a disconnected session from disk without flipping lifecycle.
    /// Re-reads live server status (D26) and returns it; respawn proceeds regardless.
    #[allow(clippy::too_many_arguments)]
    pub fn reconnect(
        &mut self,
        conn: &ConnectionId,
        session_id: &SessionId,
        events: Receiver<ServerStreamEvent>,
        feed: async_channel::Sender<ActorFeed>,
        initial_mode: OutputMode,
        stores: ActorStores,
        clock: Box<dyn Clock + Send>,
        api: Box<dyn SessionApi + Send>,
    ) -> Result<Option<SessionStatus>, FleetSchedulerError> {
        if let Some(handle) = self.registry.remove(session_id) {
            if handle.is_exited() {
                handle.join_exited();
            } else {
                self.registry.insert(session_id.clone(), handle);
                return Err(FleetSchedulerError::AlreadyRunning);
            }
        }
        // D26: re-test reality. A `failed` server session resets to `idle` across a
        // server restart; never trust a pre-disconnect status. We fetch it and RETURN it
        // so a caller (lens-drive today, lens-ui later) can shape the reconnect message,
        // but the respawn proceeds regardless — nothing is auto-terminal (D25). A fetch
        // error is NOT a hard stop; it becomes `None` (honest "couldn't confirm").
        let live_status = api.fetch_status(session_id).ok();
        let state =
            crate::persist::ControlStore::load_session(stores.control.as_ref(), conn, session_id)
                .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?
                .ok_or(FleetSchedulerError::SessionNotFound)?;
        // lifecycle already Active for a Disconnected session — no flip.
        let handle = spawn_actor_dual(state, events, feed, initial_mode, stores, clock, api);
        self.parked.remove(session_id);
        self.registry.insert(session_id.clone(), handle);
        Ok(live_status)
    }

    /// Atomic park bookkeeping: reap the exited handle and record the park reason (R2-5).
    /// No-op when a live actor is already registered (stale `Parked` after early reconnect).
    pub fn mark_parked(&mut self, session_id: &SessionId, reason: ParkReason) {
        if let Some(handle) = self.registry.remove(session_id) {
            if handle.is_exited() {
                handle.join_exited();
                self.parked.insert(session_id.clone(), reason);
            } else {
                self.registry.insert(session_id.clone(), handle);
            }
        } else {
            self.parked.insert(session_id.clone(), reason);
        }
    }

    pub fn park_reason(&self, id: &SessionId) -> Option<ParkReason> {
        self.parked.get(id).copied()
    }

    /// Borrow a registered handle to send commands or read outcome/bridge channels.
    pub fn handle(&self, session_id: &SessionId) -> Option<&ActorHandle> {
        self.registry.get(session_id)
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

    /// Remove and return a running handle for final outcome drain / join.
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
    use crate::actor::feed::ActorFeed;
    use crate::actor::outcome::ActorOutcome;
    use crate::actor::runloop::OutputMode;
    use crate::clock::ManualClock;
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::{Role, SessionLifecycle};
    use crate::persist::{
        ConnectionRecord, ControlStore, SqliteControlStore, SqliteTranscriptStore, TranscriptStore,
    };
    use crate::reduce::StreamUpdate;
    use crate::reduce::testutil::fresh_state;
    use lens_client::error::ClientError;
    use lens_client::sessions::{ItemList, SendEventAck, SessionEventInput, SessionStatus};
    use lens_client::stream::{DisconnectReason, ServerStreamEvent};
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn test_clock() -> Box<dyn Clock + Send> {
        Box::new(ManualClock::new(1_700_000_000_000))
    }

    struct MockApi {
        send_script: Mutex<VecDeque<Result<SendEventAck, ClientError>>>,
        fetch_script: Mutex<VecDeque<Result<ItemList, ClientError>>>,
        status_script: Mutex<VecDeque<Result<SessionStatus, ClientError>>>,
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
                status_script: Mutex::new(VecDeque::new()),
            });
            (Box::new(Arc::clone(&mock)), mock)
        }

        fn with_status(
            status_script: VecDeque<Result<SessionStatus, ClientError>>,
            fetch_script: VecDeque<Result<ItemList, ClientError>>,
        ) -> (Box<dyn SessionApi + Send>, Arc<MockApi>) {
            let mock = Arc::new(Self {
                send_script: Mutex::new(VecDeque::new()),
                fetch_script: Mutex::new(fetch_script),
                status_script: Mutex::new(status_script),
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

        fn fetch_status(&self, _id: &SessionId) -> Result<SessionStatus, ClientError> {
            self.status_script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(SessionStatus::Idle))
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

    fn seed_active_session(stores: &ActorStores) {
        let mut state = fresh_state();
        state.lifecycle = SessionLifecycle::Active;
        stores
            .control
            .upsert_session(&state, 1_700_000_000_000)
            .unwrap();
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

    fn status_running_event() -> ServerStreamEvent {
        use lens_client::stream::{SessionEvent, SessionStatusValue as WireStatus};
        ServerStreamEvent::Session(SessionEvent::Status {
            status: WireStatus::Running,
            response_id: None,
            background_task_count: None,
        })
    }

    #[test]
    fn wake_in_summary_emits_summary_not_summary_consumer_gone() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");
        let mut scheduler = FleetScheduler::new();

        let (api, _mock) =
            MockApi::with_scripts(VecDeque::new(), VecDeque::from([Ok(empty_item_list())]));

        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx,
                OutputMode::Summary,
                stores,
                test_clock(),
                api,
            )
            .expect("wake in Summary");

        // Pre-Task-4: no seed yet — drive a live status event so Summary mode emits.
        // Post-Task-4: the first frame may be the seed; either is ActorFeed::Summary
        // and must NOT be accompanied by SummaryConsumerGone.
        ev_tx.send(status_running_event()).unwrap();

        let frame = feed_rx.recv_blocking().expect("summary frame");
        assert!(
            matches!(frame, ActorFeed::Summary(_)),
            "spawn-in-Summary must emit Summary, got {frame:?}"
        );

        let handle = scheduler.handle(&sid).expect("running");
        assert!(
            !matches!(
                handle.outcomes.try_recv(),
                Ok(crate::actor::ActorOutcome::SummaryConsumerGone)
            ),
            "live Summary consumer must not observe SummaryConsumerGone"
        );

        scheduler.take_handle(&sid).expect("handle").stop_and_join();
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
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let mut scheduler = FleetScheduler::new();
        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx,
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("wake from slept disk");
        assert!(
            scheduler.is_running(&sid),
            "registry must retain the handle after wake"
        );

        let mut saw_catchup_tail = false;
        while let Ok(ActorFeed::Detailed(update)) = feed_rx.recv_blocking() {
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

        let control = SqliteControlStore::open(&db_path).unwrap();
        let after_wake = control
            .load_session(&conn, &sid)
            .unwrap()
            .expect("session on disk");
        assert_eq!(
            after_wake.lifecycle,
            SessionLifecycle::Active,
            "wake must flip disk lifecycle Slept → Active"
        );

        scheduler
            .handle(&sid)
            .expect("handle registered")
            .commands
            .send(SessionCommand::Promote)
            .unwrap();
        match feed_rx.recv_blocking().unwrap() {
            ActorFeed::Detailed(StreamUpdate::Rebased(baseline)) => {
                assert_eq!(baseline.title.as_deref(), Some("slept-roundtrip"));
                assert_eq!(baseline.reasoning_effort.as_deref(), Some("high"));
                assert_eq!(baseline.lifecycle, SessionLifecycle::Active);
                assert!(baseline.items.is_empty(), "Rebased is scalars-only");
            }
            other => panic!("expected Rebased after Promote, got {other:?}"),
        }

        assert!(scheduler.is_running(&sid));
        scheduler.sleep(&sid).expect("sleep routed to actor");
        let handle = scheduler
            .take_handle(&sid)
            .expect("handle still registered");
        match handle.outcomes.recv_blocking().unwrap() {
            ActorOutcome::Slept => {}
            other => panic!("expected Slept outcome, got {other:?}"),
        }
        handle.join_without_stop();

        let loaded = control.list_sessions(&conn).unwrap();
        assert_eq!(
            loaded.rows[0].lifecycle,
            SessionLifecycle::Slept,
            "sleep must flip disk lifecycle Active → Slept"
        );

        let transcript =
            SqliteTranscriptStore::open(&dir.path().join("conv_1.db"), &conn, &sid).unwrap();
        let rows = transcript.load_items().unwrap().rows;
        assert_eq!(rows.len(), 5, "item_0..item_4 on disk");
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["item_0", "item_1", "item_2", "item_3", "item_4"]);
        assert!(!scheduler.is_running(&sid), "actor stopped after sleep");
    }

    #[test]
    fn second_wake_while_running_returns_already_running() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_slept_session(&stores);

        let (api, _mock) =
            MockApi::with_scripts(VecDeque::new(), VecDeque::from([Ok(empty_item_list())]));

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let mut scheduler = FleetScheduler::new();
        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx,
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("first wake");
        assert!(scheduler.is_running(&sid));

        let (_ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let (feed_tx2, _feed_rx2) = async_channel::bounded(64);
        let stores2 = test_stores(dir.path());
        let (api2, _mock2) = MockApi::with_scripts(VecDeque::new(), VecDeque::new());
        let err = scheduler.wake(
            &conn,
            &sid,
            ev_rx2,
            feed_tx2,
            OutputMode::Detailed,
            stores2,
            test_clock(),
            api2,
        );
        assert_eq!(err, Err(FleetSchedulerError::AlreadyRunning));

        scheduler
            .take_handle(&sid)
            .expect("still registered")
            .stop_and_join();
    }

    #[test]
    fn park_then_reconnect_respawns_and_refreshes_status() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);
        for (id, ord) in [("item_0", 0), ("item_1", 1)] {
            seed_message_item(&*stores.transcript, ord, id, id);
        }

        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        // fetch_status returns idle (a `failed` session that healed across restart);
        // fetch_items returns the forward tail.
        let tail = item_list_from_messages(&["item_2"], false);
        let (api, _mock) = MockApi::with_status(
            VecDeque::from([Ok(SessionStatus::Idle)]),
            VecDeque::from([Ok(tail)]),
        );

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, feed_rx) = async_channel::bounded(64);
        let mut scheduler = FleetScheduler::new();
        scheduler.mark_parked(&sid, ParkReason::Unauthorized);
        assert_eq!(
            scheduler.park_reason(&sid),
            Some(ParkReason::Unauthorized),
            "precondition: park reason set before reconnect"
        );
        let live = scheduler
            .reconnect(
                &conn,
                &sid,
                ev_rx,
                feed_tx,
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("reconnect respawns from Active disk");
        // D26: the returned status PROVES the re-read happened (not a false-green — a
        // discarded status would let the mock return anything and still pass).
        assert_eq!(
            live,
            Some(SessionStatus::Idle),
            "reconnect re-reads + returns live status"
        );
        assert!(scheduler.is_running(&sid));
        assert!(
            scheduler.park_reason(&sid).is_none(),
            "reconnect clears park reason"
        );

        // Forward catch-up materializes the tail past frontier 1.
        let mut saw_tail = false;
        while let Ok(ActorFeed::Detailed(u)) = feed_rx.recv_blocking() {
            if matches!(
                u,
                StreamUpdate::TranscriptAdvanced {
                    committed_ordinal: 2
                }
            ) {
                saw_tail = true;
                break;
            }
        }
        assert!(
            saw_tail,
            "reconnect runs forward catch-up from disk frontier"
        );

        scheduler.take_handle(&sid).unwrap().stop_and_join();
    }

    #[test]
    fn mark_parked_then_reconnect_clears_park_reason_and_respawns() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);

        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let (api, _mock) =
            MockApi::with_status(VecDeque::new(), VecDeque::from([Ok(empty_item_list())]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let mut scheduler = FleetScheduler::new();
        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx.clone(),
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("wake");

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized,
            })
            .unwrap();

        let parked_reason = match scheduler
            .handle(&sid)
            .expect("handle registered")
            .outcomes
            .recv_blocking()
            .unwrap()
        {
            ActorOutcome::Parked { reason } => reason,
            other => panic!("expected Parked, got {other:?}"),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if scheduler.handle(&sid).is_some_and(|h| h.is_exited()) {
                break;
            }
            std::thread::yield_now();
        }
        scheduler.mark_parked(&sid, parked_reason);
        assert_eq!(
            scheduler.park_reason(&sid),
            Some(ParkReason::Unauthorized),
            "mark_parked records reason"
        );
        assert!(
            !scheduler.is_running(&sid),
            "mark_parked reaps exited handle from registry"
        );

        let stores2 = test_stores(dir.path());
        let (api2, _mock2) = MockApi::with_status(
            VecDeque::from([Ok(SessionStatus::Idle)]),
            VecDeque::from([Ok(empty_item_list())]),
        );
        let (_ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        let live = scheduler
            .reconnect(
                &conn,
                &sid,
                ev_rx2,
                feed_tx,
                OutputMode::Detailed,
                stores2,
                test_clock(),
                api2,
            )
            .expect("reconnect after mark_parked");
        assert_eq!(live, Some(SessionStatus::Idle));
        assert!(
            scheduler.park_reason(&sid).is_none(),
            "reconnect clears park reason after mark_parked"
        );
        assert!(scheduler.is_running(&sid), "reconnect respawns actor");

        scheduler.take_handle(&sid).unwrap().stop_and_join();
    }

    #[test]
    fn early_reconnect_before_parked_drain_reaps_exited_handle() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);

        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let (api, _mock) =
            MockApi::with_status(VecDeque::new(), VecDeque::from([Ok(empty_item_list())]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let mut scheduler = FleetScheduler::new();
        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx.clone(),
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("wake");

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized,
            })
            .unwrap();

        match scheduler
            .handle(&sid)
            .expect("handle still registered")
            .outcomes
            .recv_blocking()
            .unwrap()
        {
            ActorOutcome::Parked { .. } => {}
            other => panic!("expected Parked, got {other:?}"),
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if scheduler.handle(&sid).is_some_and(|h| h.is_exited()) {
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            scheduler.is_running(&sid),
            "exited handle still registered before mark_parked drain"
        );
        assert!(
            scheduler.handle(&sid).unwrap().is_exited(),
            "actor thread finished before reconnect"
        );

        let stores2 = test_stores(dir.path());
        let (api2, _mock2) = MockApi::with_status(
            VecDeque::from([Ok(SessionStatus::Idle)]),
            VecDeque::from([Ok(empty_item_list())]),
        );
        let (_ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        scheduler
            .reconnect(
                &conn,
                &sid,
                ev_rx2,
                feed_tx,
                OutputMode::Detailed,
                stores2,
                test_clock(),
                api2,
            )
            .expect("early reconnect must not wedge on exited handle");
        assert!(
            scheduler.is_running(&sid),
            "reconnect respawns after reaping exited handle"
        );
        assert!(
            scheduler.park_reason(&sid).is_none(),
            "reconnect clears stale park bookkeeping"
        );

        scheduler.take_handle(&sid).unwrap().stop_and_join();
    }

    #[test]
    fn stale_mark_parked_after_early_reconnect_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);

        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let (api, _mock) =
            MockApi::with_status(VecDeque::new(), VecDeque::from([Ok(empty_item_list())]));

        let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let mut scheduler = FleetScheduler::new();
        scheduler
            .wake(
                &conn,
                &sid,
                ev_rx,
                feed_tx.clone(),
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("wake");

        ev_tx
            .send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized,
            })
            .unwrap();

        // Parked outcome left undrained — simulate caller queue lag.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if scheduler.handle(&sid).is_some_and(|h| h.is_exited()) {
                break;
            }
            std::thread::yield_now();
        }

        let stores2 = test_stores(dir.path());
        let (api2, _mock2) = MockApi::with_status(
            VecDeque::from([Ok(SessionStatus::Idle)]),
            VecDeque::from([Ok(empty_item_list())]),
        );
        let (_ev_tx2, ev_rx2) = crossbeam_channel::bounded(64);
        scheduler
            .reconnect(
                &conn,
                &sid,
                ev_rx2,
                feed_tx,
                OutputMode::Detailed,
                stores2,
                test_clock(),
                api2,
            )
            .expect("early reconnect respawns live actor");

        // Stale drain contract: late mark_parked must not wedge or re-stamp.
        scheduler.mark_parked(&sid, ParkReason::Unauthorized);

        assert!(
            scheduler.is_running(&sid),
            "live respawned actor must survive stale mark_parked"
        );
        assert!(
            scheduler.park_reason(&sid).is_none(),
            "stale mark_parked must not stamp park reason on live session"
        );
        assert!(
            !scheduler.handle(&sid).unwrap().is_exited(),
            "live actor thread must still be running"
        );

        scheduler.take_handle(&sid).unwrap().stop_and_join();
    }

    #[test]
    fn reconnect_fetch_status_error_still_respawns() {
        let dir = tempfile::tempdir().unwrap();
        let stores = test_stores(dir.path());
        seed_connection(&stores);
        seed_active_session(&stores);

        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");

        let (api, _mock) = MockApi::with_status(
            VecDeque::from([Err(ClientError::network_for_test())]),
            VecDeque::from([Ok(empty_item_list())]),
        );

        let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
        let (feed_tx, _feed_rx) = async_channel::bounded(64);
        let mut scheduler = FleetScheduler::new();
        let live = scheduler
            .reconnect(
                &conn,
                &sid,
                ev_rx,
                feed_tx,
                OutputMode::Detailed,
                stores,
                test_clock(),
                api,
            )
            .expect("reconnect proceeds despite fetch_status error");
        assert_eq!(
            live, None,
            "fetch_status Err becomes None (D25: not auto-terminal)"
        );
        assert!(
            scheduler.is_running(&sid),
            "respawn still happens when status fetch fails"
        );

        scheduler.take_handle(&sid).unwrap().stop_and_join();
    }
}
