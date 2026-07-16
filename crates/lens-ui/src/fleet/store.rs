use crate::card::model::SessionCard;
use crate::clock::UiClock;
use crate::fleet::fake::FakeFleet;
use crate::fleet::poller::spawn_session_poller;
use gpui::{App, AppContext, Context, Entity, Task};
use lens_core::domain::ids::SessionId;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

pub struct FleetStore {
    pub cards: HashMap<SessionId, Entity<SessionCard>>,
    pub focused: Option<SessionId>,
    pub fake: Option<FakeFleet>,
    clock: Arc<dyn UiClock>,
    store_notify_count: Cell<u64>,
    command_txs: HashMap<SessionId, crossbeam_channel::Sender<lens_core::actor::SessionCommand>>,
    pollers: HashMap<SessionId, Task<()>>,
}

impl FleetStore {
    pub fn new(clock: Arc<dyn UiClock>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            cards: HashMap::new(),
            focused: None,
            fake: Some(FakeFleet::new()),
            clock,
            store_notify_count: Cell::new(0),
            command_txs: HashMap::new(),
            pollers: HashMap::new(),
        })
    }

    pub fn store_notify_count(&self) -> u64 {
        self.store_notify_count.get()
    }

    pub fn card(&self, id: &SessionId) -> Option<Entity<SessionCard>> {
        self.cards.get(id).cloned()
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
}
