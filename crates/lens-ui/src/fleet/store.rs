use crate::card::model::SessionCard;
use crate::clock::UiClock;
use crate::fleet::fake::{FEED_CAPACITY, FakeFleet};
use crate::fleet::live::{self, StreamBridge, WallClock};
use crate::fleet::poller::spawn_session_poller;
use gpui::{App, AppContext, Context, Entity, Task};
use lens_client::{Client, Connection};
use lens_core::actor::{ClientSessionApi, FleetScheduler, OutputMode, SessionCommand};
use lens_core::domain::ids::SessionId;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

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

    pub fn card(&self, id: &SessionId) -> Option<Entity<SessionCard>> {
        self.cards.get(id).cloned()
    }

    pub fn focused(&self) -> Option<&SessionId> {
        self.focused.as_ref()
    }

    pub fn focus_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if self.focused.as_ref() == Some(&id) {
            self.blur_to_board(cx);
            return;
        }
        if let Some(prev) = self.focused.clone() {
            self.send_command(&prev, SessionCommand::Demote);
            self.set_card_focused(&prev, false, cx);
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
        self.pollers.insert(
            id.clone(),
            spawn_session_poller(
                card.clone(),
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
        self.pollers.insert(
            session_id.clone(),
            spawn_session_poller(
                card.clone(),
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
mod focus_tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use lens_core::actor::SessionCommand;
    use std::sync::Arc;

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
