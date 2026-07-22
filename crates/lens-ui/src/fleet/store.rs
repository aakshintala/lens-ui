use crate::card::model::{ConnectionOverlay, SessionCard};
use crate::clock::UiClock;
use crate::fleet::fake::{FEED_CAPACITY, FakeFleet};
use crate::fleet::live::{self, StreamBridge, WallClock};
use crate::fleet::poller::spawn_session_poller;
use crate::focused::FocusedTranscript;
use gpui::{App, AppContext, Context, Entity, Task};
use lens_client::{Client, Connection};
use lens_core::actor::{
    ActorFeed, ActorTransport, ClientSessionApi, FleetScheduler, OutputMode, SessionCommand,
};
use lens_core::domain::ids::{ConnectionId, SessionId};
use lens_core::persist::{PersistError, SqliteTranscriptReader};
use lens_core::reduce::StreamUpdate;
use smallvec::SmallVec;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileEpoch {
    pub epoch: u64,
    pub in_flight: bool,
}

#[derive(Clone)]
pub struct ReaderFactory {
    data_dir: PathBuf,
    #[allow(dead_code)] // Task 9 replica install reads connection context from here.
    conn_id: ConnectionId,
    session_id: SessionId,
}

impl ReaderFactory {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    #[cfg(test)]
    pub(crate) fn test_with_data_dir(data_dir: PathBuf, session_id: SessionId) -> Self {
        Self {
            data_dir,
            conn_id: ConnectionId::new("conn_reader_test"),
            session_id,
        }
    }

    pub fn open(&self, busy_timeout: Duration) -> Result<SqliteTranscriptReader, PersistError> {
        SqliteTranscriptReader::open_read_only(
            &self.data_dir.join(format!("{}.db", self.session_id)),
            busy_timeout,
        )
    }
}

pub struct FleetStore {
    pub cards: HashMap<SessionId, Entity<SessionCard>>,
    pub focused: Option<SessionId>,
    pub fake: Option<FakeFleet>,
    scheduler: Option<FleetScheduler>,
    clock: Arc<dyn UiClock>,
    store_notify_count: Cell<u64>,
    command_txs: HashMap<SessionId, crossbeam_channel::Sender<SessionCommand>>,
    pollers: HashMap<SessionId, Task<()>>,
    stream_bridges: HashMap<SessionId, StreamBridge>,
    reader_factories: HashMap<SessionId, ReaderFactory>,
    reconcile_epochs: HashMap<SessionId, ReconcileEpoch>,
    focused_replica: Option<(SessionId, Entity<FocusedTranscript>)>,
    focus_generation: u64,
    #[cfg(test)]
    focused_detailed_fanout_count: Cell<u64>,
}

impl FleetStore {
    pub fn new(clock: Arc<dyn UiClock>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            cards: HashMap::new(),
            focused: None,
            fake: Some(FakeFleet::new()),
            scheduler: None,
            clock,
            store_notify_count: Cell::new(0),
            command_txs: HashMap::new(),
            pollers: HashMap::new(),
            stream_bridges: HashMap::new(),
            reader_factories: HashMap::new(),
            reconcile_epochs: HashMap::new(),
            focused_replica: None,
            focus_generation: 0,
            #[cfg(test)]
            focused_detailed_fanout_count: Cell::new(0),
        })
    }

    pub fn new_live(clock: Arc<dyn UiClock>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            cards: HashMap::new(),
            focused: None,
            fake: None,
            scheduler: Some(FleetScheduler::new()),
            clock,
            store_notify_count: Cell::new(0),
            command_txs: HashMap::new(),
            pollers: HashMap::new(),
            stream_bridges: HashMap::new(),
            reader_factories: HashMap::new(),
            reconcile_epochs: HashMap::new(),
            focused_replica: None,
            focus_generation: 0,
            #[cfg(test)]
            focused_detailed_fanout_count: Cell::new(0),
        })
    }

    pub fn store_notify_count(&self) -> u64 {
        self.store_notify_count.get()
    }

    pub fn clock(&self) -> Arc<dyn UiClock> {
        Arc::clone(&self.clock)
    }

    pub fn send_session_command(&self, id: &SessionId, cmd: SessionCommand) {
        self.send_command(id, cmd);
    }

    /// Wake a Slept session. SEAM: real behavior = respawn the actor from the
    /// persisted connection context (state-model wake=respawn), which FleetStore
    /// does not yet retain. TODO(state-model P3+): re-run `spawn_live_session`.
    pub fn wake_session(&self, _id: &SessionId) {
        // Intentional no-op until the wake=respawn plumbing lands. The button is a
        // real affordance wired to this seam, not a dead element.
    }

    /// Retry a Failed session. SEAM: real behavior = re-poke / respawn the session.
    /// TODO(state-model P3+): route to the actual retry path once it exists.
    pub fn retry_session(&self, _id: &SessionId) {
        // Intentional no-op — see `wake_session`.
    }

    pub fn card(&self, id: &SessionId) -> Option<Entity<SessionCard>> {
        self.cards.get(id).cloned()
    }

    pub fn focused(&self) -> Option<&SessionId> {
        self.focused.as_ref()
    }

    pub fn focused_replica(&self) -> Option<Entity<FocusedTranscript>> {
        self.focused_replica
            .as_ref()
            .map(|(_, replica)| replica.clone())
    }

    pub fn reader_factory(&self, id: &SessionId) -> Option<&ReaderFactory> {
        self.reader_factories.get(id)
    }

    pub fn reconcile_epoch(&self, id: &SessionId) -> ReconcileEpoch {
        self.reconcile_epochs.get(id).cloned().unwrap_or_default()
    }

    #[cfg(test)]
    pub fn focused_detailed_fanout_count(&self) -> u64 {
        self.focused_detailed_fanout_count.get()
    }

    /// Drain a coalesced feed batch: Summary→card, Detailed→card-chrome and (when focused)
    /// the Task-9 replica hook.
    pub fn fold_session_feed(
        &mut self,
        id: &SessionId,
        mut batch: SmallVec<[ActorFeed; 8]>,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.cards.get(id).cloned() else {
            return;
        };
        let route_replica = self.focused.as_ref() == Some(id);
        let detailed_for_replica: SmallVec<[StreamUpdate; 8]> = if route_replica {
            batch
                .iter()
                .filter_map(|frame| match frame {
                    ActorFeed::Detailed(u) => Some(u.clone()),
                    _ => None,
                })
                .collect()
        } else {
            SmallVec::new()
        };
        let clock = Arc::clone(&self.clock);
        card.update(cx, |card, _| {
            for frame in batch.drain(..) {
                match frame {
                    ActorFeed::Summary(u) => card.fold_summary(&u, clock.as_ref()),
                    ActorFeed::Detailed(u) => card.fold_detailed(u),
                }
            }
        });
        let route_focused_detailed = route_replica && !detailed_for_replica.is_empty();
        if route_focused_detailed
            && let Some(replica) = self
                .focused_replica
                .as_ref()
                .filter(|(focused_id, _)| focused_id == id)
                .map(|(_, replica)| replica.clone())
        {
            for u in detailed_for_replica {
                replica.update(cx, |r, cx| r.fold_detailed(u, cx));
            }
        }
        if route_focused_detailed {
            #[cfg(test)]
            self.focused_detailed_fanout_count
                .set(self.focused_detailed_fanout_count.get().saturating_add(1));
        }
    }

    pub fn apply_transport(
        &mut self,
        id: &SessionId,
        transport: ActorTransport,
        reconcile_in_flight: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some((focused_id, replica)) = &self.focused_replica
            && focused_id == id
        {
            let replica = replica.clone();
            replica.update(cx, |r, cx| {
                r.set_reconcile_in_flight(reconcile_in_flight, cx)
            });
        }
        if let Some(card) = self.cards.get(id) {
            card.update(cx, |card, cx| {
                card.connection_overlay = match transport {
                    ActorTransport::Connected => ConnectionOverlay::Connected,
                    ActorTransport::Reconnecting => ConnectionOverlay::Reconnecting,
                };
                card.notify_count = card.notify_count.saturating_add(1);
                cx.notify();
            });
        }
        let entry = self.reconcile_epochs.entry(id.clone()).or_default();
        let was_in_flight = entry.in_flight;
        if reconcile_in_flight && !was_in_flight {
            entry.epoch = entry.epoch.saturating_add(1);
            entry.in_flight = true;
        } else if !reconcile_in_flight && was_in_flight {
            entry.in_flight = false;
            let epoch = entry.epoch;
            if let Some((focused_id, replica)) = &self.focused_replica
                && focused_id == id
            {
                let replica = replica.clone();
                replica.update(cx, |r, cx| r.on_reconcile_epoch_settled(epoch, cx));
            }
        } else if reconcile_in_flight {
            entry.in_flight = true;
        }
    }

    pub fn focus_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.focused.as_ref() == Some(&id) {
            self.blur_to_board(cx);
            return;
        }
        if let Some(prev) = self.focused.clone() {
            self.send_command(&prev, SessionCommand::Demote);
            self.set_card_focused(&prev, false, cx);
            self.focused_replica = None;
        }
        self.focus_generation = self.focus_generation.saturating_add(1);
        let generation = self.focus_generation;
        if let Some(factory) = self.reader_factories.get(&id).cloned() {
            let epoch = self.reconcile_epochs.get(&id).cloned().unwrap_or_default();
            let reconcile_in_flight = epoch.in_flight;
            let replica = cx.new(|cx| FocusedTranscript::new(factory, epoch, generation, cx));
            if reconcile_in_flight {
                replica.update(cx, |r, cx| r.set_reconcile_in_flight(true, cx));
            }
            self.focused_replica = Some((id.clone(), replica));
        }
        self.send_command(&id, SessionCommand::Promote);
        self.set_card_focused(&id, true, cx);
        self.focused = Some(id);
        self.store_notify_count
            .set(self.store_notify_count.get().saturating_add(1));
        cx.notify();
    }

    pub fn blur_to_board(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.focused.take() {
            self.send_command(&prev, SessionCommand::Demote);
            self.set_card_focused(&prev, false, cx);
            self.focused_replica = None;
            self.store_notify_count
                .set(self.store_notify_count.get().saturating_add(1));
            cx.notify();
        }
    }

    fn set_card_focused(&self, id: &SessionId, focused: bool, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.get(id) {
            card.update(cx, |c, cx| {
                c.is_focused = focused;
                cx.notify();
            });
        }
    }

    fn send_command(&self, id: &SessionId, cmd: SessionCommand) {
        if let Some(tx) = self.command_txs.get(id) {
            let _ = tx.try_send(cmd);
        }
    }

    pub fn spawn_fake_session(
        &mut self,
        id: SessionId,
        cx: &mut Context<Self>,
    ) -> Entity<SessionCard> {
        let fake = self.fake.as_mut().expect("fake mode");
        let handles = fake.spawn_session(id.clone());
        self.command_txs.insert(id.clone(), handles.commands_tx);
        let card = cx.new(|_| SessionCard::new(id.clone()));
        let store = cx.entity().downgrade();
        self.pollers.insert(
            id.clone(),
            spawn_session_poller(
                id.clone(),
                store,
                handles.feed_rx,
                handles.outcomes_rx,
                Arc::clone(&self.clock),
                &mut *cx,
            ),
        );
        self.cards.insert(id, card.clone());
        self.store_notify_count
            .set(self.store_notify_count.get().saturating_add(1));
        cx.notify();
        card
    }

    pub fn spawn_live_session(
        &mut self,
        conn: &Connection,
        client: &Client,
        session_id: SessionId,
        data_dir: &Path,
        cx: &mut Context<Self>,
    ) -> Result<Entity<SessionCard>, String> {
        let scheduler = self
            .scheduler
            .as_mut()
            .ok_or_else(|| "live mode required".to_string())?;

        let (feed_tx, feed_rx) = async_channel::bounded(FEED_CAPACITY);
        let stream = client
            .sessions()
            .stream(&session_id)
            .map_err(|e| format!("stream: {e}"))?;
        let (bridge, events_rx) = live::start_stream_bridge(stream);
        let stores = live::open_stores(data_dir, &conn.id, &session_id)?;
        self.reader_factories.insert(
            session_id.clone(),
            ReaderFactory {
                data_dir: data_dir.to_path_buf(),
                conn_id: conn.id.clone(),
                session_id: session_id.clone(),
            },
        );
        self.reconcile_epochs
            .insert(session_id.clone(), ReconcileEpoch::default());
        let api = Box::new(ClientSessionApi::new(
            Client::new(conn.clone()).map_err(|e| format!("client handshake: {e}"))?,
        ));
        let clock = Box::new(WallClock);
        scheduler
            .reconnect(
                &conn.id,
                &session_id,
                events_rx,
                feed_tx,
                OutputMode::Summary,
                stores,
                clock,
                api,
            )
            .map_err(|e| format!("{e:?}"))?;

        let handle = scheduler
            .handle(&session_id)
            .ok_or_else(|| "handle missing".to_string())?;
        let outcomes_rx = handle.outcomes.clone();
        let commands = handle.commands.clone();
        self.command_txs.insert(session_id.clone(), commands);

        let card = cx.new(|_| SessionCard::new(session_id.clone()));
        let store = cx.entity().downgrade();
        self.pollers.insert(
            session_id.clone(),
            spawn_session_poller(
                session_id.clone(),
                store,
                feed_rx,
                outcomes_rx,
                Arc::clone(&self.clock),
                &mut *cx,
            ),
        );
        self.stream_bridges.insert(session_id.clone(), bridge);
        self.cards.insert(session_id.clone(), card.clone());
        self.store_notify_count
            .set(self.store_notify_count.get().saturating_add(1));
        cx.notify();
        Ok(card)
    }
}

#[cfg(test)]
mod fold_session_feed_tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use lens_core::actor::ActorFeed;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::reduce::StreamUpdate;
    use std::sync::Arc;

    #[gpui::test]
    async fn focused_detailed_batch_reaches_card_unfocused_skips_replica_hook(
        cx: &mut gpui::TestAppContext,
    ) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let sid2 = SessionId::new("s2");
        let fleet = cx.update(|cx| {
            let f = FleetStore::new(clock, cx);
            f.update(cx, |f, cx| {
                f.spawn_fake_session(sid.clone(), cx);
                f.spawn_fake_session(sid2.clone(), cx);
            });
            f
        });

        cx.update(|cx| {
            let f = fleet.read(cx);
            let fake = f.fake.as_ref().expect("fake mode");
            fake.push_feed(
                &sid,
                ActorFeed::Detailed(StreamUpdate::StatusChanged(SessionStatusValue::Running)),
            );
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let f = fleet.read(cx);
            assert_eq!(f.focused_detailed_fanout_count(), 0);
            let card = f.card(&sid).unwrap().read(cx);
            assert_eq!(card.status, SessionStatusValue::Running);
        });

        cx.update(|cx| {
            fleet.update(cx, |f, cx| f.focus_session(sid2.clone(), cx));
            let f = fleet.read(cx);
            f.fake.as_ref().expect("fake mode").push_feed(
                &sid2,
                ActorFeed::Detailed(StreamUpdate::StatusChanged(SessionStatusValue::Failed)),
            );
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let f = fleet.read(cx);
            assert_eq!(f.focused_detailed_fanout_count(), 1);
            let card = f.card(&sid2).unwrap().read(cx);
            assert_eq!(card.status, SessionStatusValue::Failed);
        });

        cx.update(|cx| {
            let f = fleet.read(cx);
            f.fake.as_ref().expect("fake mode").push_feed(
                &sid,
                ActorFeed::Detailed(StreamUpdate::StatusChanged(SessionStatusValue::Idle)),
            );
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let f = fleet.read(cx);
            assert_eq!(f.focused_detailed_fanout_count(), 1);
            let card = f.card(&sid).unwrap().read(cx);
            assert_eq!(card.status, SessionStatusValue::Idle);
        });
    }

    #[gpui::test]
    async fn apply_transport_reconcile_edges_bump_epoch(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        cx.update(|cx| {
            fleet.update(cx, |f, cx| {
                f.apply_transport(&sid, ActorTransport::Connected, true, cx);
                let ep = f.reconcile_epoch(&sid);
                assert!(ep.in_flight);
                assert_eq!(ep.epoch, 1);

                f.apply_transport(&sid, ActorTransport::Connected, false, cx);
                let ep = f.reconcile_epoch(&sid);
                assert!(!ep.in_flight);
                assert_eq!(ep.epoch, 1);
            });
        });
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use gpui::{IntoElement, Render};
    use lens_core::actor::SessionCommand;
    use std::sync::Arc;
    use std::time::Duration;

    struct TimerBoard;

    impl Render for TimerBoard {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    #[gpui::test]
    async fn focus_session_reconcile_in_flight_arms_syncing_after_debounce(
        cx: &mut gpui::TestAppContext,
    ) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("sync_focus");
        let data_dir = std::env::temp_dir().join(format!(
            "lens-focus-sync-{}-{}",
            std::process::id(),
            sid.as_str()
        ));

        let fleet = cx.update(|cx| {
            let f = FleetStore::new(clock, cx);
            f.update(cx, |f, cx| {
                f.spawn_fake_session(sid.clone(), cx);
                f.reader_factories.insert(
                    sid.clone(),
                    ReaderFactory::test_with_data_dir(data_dir.clone(), sid.clone()),
                );
                f.reconcile_epochs.insert(
                    sid.clone(),
                    ReconcileEpoch {
                        epoch: 1,
                        in_flight: true,
                    },
                );
            });
            f
        });

        cx.update(|cx| {
            fleet.update(cx, |f, cx| f.focus_session(sid.clone(), cx));
        });

        let replica = cx.read(|cx| {
            fleet
                .read(cx)
                .focused_replica()
                .expect("focus_session must mount replica when reader factory is installed")
        });

        {
            let (_board, vcx) = cx.add_window_view(|_, _| TimerBoard);
            vcx.run_until_parked();
            vcx.executor().advance_clock(Duration::from_millis(200));
            vcx.run_until_parked();
        }

        assert!(
            replica.read_with(cx, |r, _| r.syncing()),
            "focus_session must seed in-flight reconcile and show syncing after 150 ms debounce"
        );
    }

    #[gpui::test]
    async fn click_focus_sends_promote_and_demote_previous(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let a = SessionId::new("a");
        let b = SessionId::new("b");
        let fleet = cx.update(|cx| {
            let f = FleetStore::new(clock, cx);
            f.update(cx, |f, cx| {
                f.spawn_fake_session(a.clone(), cx);
                f.spawn_fake_session(b.clone(), cx);
            });
            f
        });
        cx.update(|cx| {
            fleet.update(cx, |f, cx| f.focus_session(a.clone(), cx));
            fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx));
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let f = fleet.read(cx);
            let cmds_a = f.fake.as_ref().unwrap().take_commands(&a);
            let cmds_b = f.fake.as_ref().unwrap().take_commands(&b);
            assert!(
                cmds_a.iter().any(|c| matches!(c, SessionCommand::Promote)),
                "A promoted first"
            );
            assert!(
                cmds_a.iter().any(|c| matches!(c, SessionCommand::Demote)),
                "A demoted when B focused"
            );
            assert!(
                cmds_b.iter().any(|c| matches!(c, SessionCommand::Promote)),
                "B promoted"
            );
            assert_eq!(f.focused.as_ref(), Some(&b));
        });
    }

    #[test]
    fn live_spawn_api_exists() {
        let _spawn = FleetStore::spawn_live_session;
        let _new_live = FleetStore::new_live;
    }
}

#[cfg(test)]
mod reader_factory_tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use lens_client::{Auth, Client, Connection, PINNED_OMNIGENT_VERSION};
    use lens_core::domain::SessionState;
    use lens_core::domain::ids::AgentId;
    use lens_core::domain::scalars::{SessionLifecycle, SessionStatusValue};
    use lens_core::persist::{
        ConnectionRecord, ControlStore, ReadRange, SqliteControlStore, TranscriptReader,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};

    struct MockOmnigent {
        base_url: String,
        _server: JoinHandle<()>,
    }

    impl MockOmnigent {
        fn start(session_id: &SessionId) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            listener
                .set_nonblocking(true)
                .expect("mock listener nonblocking");
            let addr = listener.local_addr().expect("mock addr");
            let sid = session_id.as_str().to_string();
            let stop = Arc::new(AtomicBool::new(false));
            let stop_for_thread = Arc::clone(&stop);
            let server = thread::spawn(move || {
                while !stop_for_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let sid = sid.clone();
                            thread::spawn(move || handle_mock_conn(&mut stream, &sid));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("mock accept: {e}"),
                    }
                }
            });
            let base_url = format!("http://{addr}");
            Self {
                base_url,
                _server: server,
            }
        }
    }

    fn http_response(status: &str, content_type: &str, body: &str) -> String {
        // Keep-alive + Content-Length: the client reads exactly `Content-Length`
        // bytes and may reuse the socket for the next handshake request. The mock
        // loops to serve each one (see handle_mock_conn), so a pooled reqwest
        // connection never hits a mid-handshake close → no `IncompleteMessage`.
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
            body.len()
        )
    }

    fn session_json(session_id: &str, include_items: bool) -> String {
        if include_items {
            format!(
                r#"{{"id":"{session_id}","status":"idle","agent_id":"ag_test","created_at":1,"items":[]}}"#
            )
        } else {
            format!(
                r#"{{"id":"{session_id}","status":"idle","agent_id":"ag_test","created_at":1}}"#
            )
        }
    }

    /// Read one full HTTP request (headers terminated by CRLFCRLF) off a keep-alive
    /// socket. Returns None on EOF/timeout/error — i.e. the client is done and the
    /// connection can be dropped. GET requests carry no body, so header-end is
    /// request-end. A single `read()` can return a partial request line; looping
    /// until CRLFCRLF guarantees the matcher always sees the complete request.
    fn read_http_request(stream: &mut TcpStream) -> Option<String> {
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(n) => {
                    acc.extend_from_slice(&buf[..n]);
                    if acc.windows(4).any(|w| w == b"\r\n\r\n") {
                        return Some(String::from_utf8_lossy(&acc).into_owned());
                    }
                    if acc.len() > 64 * 1024 {
                        return None; // malformed / never-terminated request
                    }
                }
            }
        }
    }

    fn write_response(stream: &mut TcpStream, body: &str) -> bool {
        stream.write_all(body.as_bytes()).is_ok() && stream.flush().is_ok()
    }

    /// Keep-alive request loop: serve every request on the connection until the
    /// client closes it. A one-request-then-close mock races reqwest's connection
    /// pool (it reuses the socket for the 3-step /health→/api/version→/v1/info
    /// handshake ladder) → mid-handshake close → `IncompleteMessage`. Looping here
    /// makes the mock robust to reuse and deterministic.
    fn handle_mock_conn(stream: &mut TcpStream, session_id: &str) {
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
        loop {
            let Some(req) = read_http_request(stream) else {
                return;
            };
            let request_line = req.lines().next().unwrap_or("");
            if request_line.contains("/stream") {
                // SSE: send headers then stream heartbeats until the client drops.
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n";
                if stream.write_all(headers.as_bytes()).is_err() {
                    return;
                }
                let heartbeat = b"event: session.heartbeat\ndata: {}\n\n";
                loop {
                    if stream.write_all(heartbeat).is_err() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
            let response = if request_line.contains("/health") {
                http_response("200 OK", "text/plain", "ok")
            } else if request_line.contains("/api/version") {
                http_response(
                    "200 OK",
                    "application/json",
                    &format!(r#"{{"version":"{PINNED_OMNIGENT_VERSION}"}}"#),
                )
            } else if request_line.contains("/v1/info") {
                http_response("200 OK", "application/json", "{}")
            } else if request_line.contains("/items") {
                let json = r#"{"data":[{"id":"item_a","type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],"has_more":false}"#;
                http_response("200 OK", "application/json", json)
            } else if request_line.contains("/v1/sessions/") {
                let include_items = req.contains("include_items=true");
                http_response(
                    "200 OK",
                    "application/json",
                    &session_json(session_id, include_items),
                )
            } else {
                http_response("404 Not Found", "text/plain", "not found")
            };
            if !write_response(stream, &response) {
                return;
            }
        }
    }

    fn seed_control(data_dir: &Path, conn: &Connection, session_id: &SessionId) {
        let control = SqliteControlStore::open(&data_dir.join("lens.db")).unwrap();
        control
            .upsert_connection(&ConnectionRecord {
                id: conn.id.clone(),
                base_url: conn.base_url.to_string(),
                auth_kind: "none".into(),
                label: None,
                server_info: None,
                created_at: 1_700_000_000_000,
            })
            .unwrap();
        let mut state =
            SessionState::new(conn.id.clone(), session_id.clone(), AgentId::new("ag_test"));
        state.lifecycle = SessionLifecycle::Active;
        state.status = SessionStatusValue::Idle;
        control.upsert_session(&state, 1_700_000_000_000).unwrap();
    }

    #[gpui::test]
    async fn spawned_session_retains_reader_factory(cx: &mut gpui::TestAppContext) {
        let session_id = SessionId::new("conv_task7");
        let mock = MockOmnigent::start(&session_id);
        let conn_id = ConnectionId::new("conn_task7");
        let conn = Connection::new(
            conn_id,
            mock.base_url.parse().expect("mock base url"),
            Auth::None,
        );
        let data_dir = std::env::temp_dir().join(format!(
            "lens-task7-{}-{}",
            std::process::id(),
            session_id.as_str()
        ));
        std::fs::create_dir_all(&data_dir).expect("temp data dir");
        seed_control(&data_dir, &conn, &session_id);

        let clock: Arc<dyn UiClock> = Arc::new(ManualUiClock::new(1_700_000_000_000));
        let fleet = cx.update(|cx| FleetStore::new_live(Arc::clone(&clock), cx));

        // Bounded retry of the live setup. The mock is a real localhost HTTP+SSE
        // server; reqwest's connection pool can transiently drop a handshake
        // request (~1/120) — a benign infra hiccup, not a product bug. Both
        // fallible network ops (`Client::new` handshake, `spawn_live_session`'s
        // `.stream()`) fail BEFORE any store state is inserted (the reader-factory
        // insert follows `.stream()?`), so each attempt starts clean → the retry
        // makes the test deterministic without masking any real failure.
        let mut last_err = None;
        let mut spawned = false;
        for _ in 0..5 {
            let client = match Client::new(conn.clone()) {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(format!("client handshake: {e}"));
                    continue;
                }
            };
            let result = cx.update(|cx| {
                fleet.update(cx, |store, cx| {
                    store.spawn_live_session(&conn, &client, session_id.clone(), &data_dir, cx)
                })
            });
            match result {
                Ok(_) => {
                    spawned = true;
                    break;
                }
                Err(e) => last_err = Some(format!("spawn live session: {e}")),
            }
        }
        assert!(spawned, "live setup failed after 5 retries: {last_err:?}");
        cx.run_until_parked();
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(25));
            cx.run_until_parked();
        }

        let factory = fleet
            .read_with(cx, |store, _| store.reader_factory(&session_id).cloned())
            .expect("reader factory retained");
        let reader = factory
            .open(Duration::from_millis(200))
            .expect("factory opens reader");
        assert!(
            reader.read_range(ReadRange::All).is_ok(),
            "reader reads committed baseline"
        );

        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
