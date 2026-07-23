use crate::card::model::{ConnectionOverlay, READY_DECAY_MS, SessionCard};
use crate::clock::UiClock;
use crate::fleet::store::FleetStore;
use futures::future::{Either, select};
use futures::pin_mut;
use gpui::{App, Task, WeakEntity};
use lens_core::actor::{ActorFeed, ActorOutcome, ActorTransport};
use lens_core::domain::ids::SessionId;
use std::sync::Arc;
use std::time::Duration;

pub fn spawn_session_poller(
    session_id: SessionId,
    store: WeakEntity<FleetStore>,
    feed_rx: async_channel::Receiver<ActorFeed>,
    outcomes_rx: async_channel::Receiver<ActorOutcome>,
    clock: Arc<dyn UiClock>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        // Dual-clock: Ready DECISION uses UiClock; decay WAKE uses gpui's executor timer.
        // Task 5 advances both consistently in tests (ManualUiClock + advance_clock).
        let mut decay_timer: Option<Task<()>> = None;
        loop {
            let feed_wait = feed_rx.recv();
            let out_wait = outcomes_rx.recv();
            pin_mut!(feed_wait);
            pin_mut!(out_wait);
            match select(feed_wait, out_wait).await {
                Either::Left((Ok(first), _)) => {
                    let mut batch = smallvec::SmallVec::<[ActorFeed; 8]>::new();
                    batch.push(first);
                    while let Ok(more) = feed_rx.try_recv() {
                        batch.push(more);
                    }
                    let clock = Arc::clone(&clock);
                    let fold = store.update(cx, |store, cx| {
                        store.fold_session_feed(&session_id, batch, cx);
                        let card = store.cards.get(&session_id)?.clone();
                        card.update(cx, |card, cx| {
                            card.notify_count = card.notify_count.saturating_add(1);
                            cx.notify();
                            let stamp_at = if card.ready_reschedule {
                                card.last_completed_at
                            } else {
                                None
                            };
                            card.ready_reschedule = false;
                            stamp_at
                        })
                    });
                    match fold {
                        Ok(Some(stamp_at)) => {
                            let card_t = store
                                .read_with(cx, |s, _| s.cards.get(&session_id).cloned())
                                .ok()
                                .flatten();
                            if let Some(card_t) = card_t {
                                let delay = stamp_at
                                    .saturating_add(READY_DECAY_MS)
                                    .saturating_sub(clock.now_millis())
                                    .max(0) as u64;
                                // replace cancels any prior timer (Task cancels on drop).
                                drop(decay_timer.replace(cx.spawn(async move |cx| {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(delay))
                                        .await;
                                    let _ = card_t.update(cx, |card, cx| {
                                        card.notify_count = card.notify_count.saturating_add(1);
                                        cx.notify();
                                    });
                                })));
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
                Either::Right((Ok(first), _)) => {
                    let mut batch = smallvec::SmallVec::<[ActorOutcome; 4]>::new();
                    batch.push(first);
                    while let Ok(more) = outcomes_rx.try_recv() {
                        batch.push(more);
                    }
                    if store
                        .update(cx, |store, cx| {
                            let card = store.cards.get(&session_id).cloned();
                            let mut card_notify = false;
                            for o in batch.drain(..) {
                                match o {
                                    ActorOutcome::TransportChanged {
                                        transport,
                                        reconcile_in_flight,
                                    } => store.apply_transport(
                                        &session_id,
                                        transport,
                                        reconcile_in_flight,
                                        cx,
                                    ),
                                    other => {
                                        if let Some(card) = &card {
                                            card.update(cx, |card, _cx| {
                                                apply_outcome(card, other);
                                            });
                                            card_notify = true;
                                        }
                                    }
                                }
                            }
                            if card_notify && let Some(card) = card {
                                card.update(cx, |card, cx| {
                                    card.notify_count = card.notify_count.saturating_add(1);
                                    cx.notify();
                                });
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Either::Left((Err(_), _)) | Either::Right((Err(_), _)) => break,
            }
        }
    })
}

fn apply_outcome(card: &mut SessionCard, outcome: ActorOutcome) {
    match outcome {
        ActorOutcome::Parked { reason: _ } => {
            card.connection_overlay = ConnectionOverlay::Disconnected;
        }
        ActorOutcome::TransportChanged { transport, .. } => {
            card.connection_overlay = match transport {
                ActorTransport::Connected => ConnectionOverlay::Connected,
                ActorTransport::Reconnecting => ConnectionOverlay::Reconnecting,
            };
        }
        ActorOutcome::FeedConsumerGone
        | ActorOutcome::PersistError { .. }
        | ActorOutcome::SendLost { .. }
        | ActorOutcome::Slept
        | ActorOutcome::SleepDeclined
        | ActorOutcome::Command(_) => {}
        // Terminal-5 sub-slice B added these control outcomes; sub-slice D routes
        // them into FleetStore (load-B/re-parent/Transfer + resource forwarding).
        // No-op interim so lens-ui compiles between B and D.
        ActorOutcome::Superseded { .. } | ActorOutcome::TerminalResource(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use crate::fleet::store::FleetStore;
    use lens_core::actor::{ActorFeed, ActorOutcome, SummaryUpdate};
    use lens_core::domain::ids::SessionId;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::usage::Cost;

    fn summary(
        status: SessionStatusValue,
        title: &str,
        activity: &str,
        turn: u32,
    ) -> SummaryUpdate {
        SummaryUpdate {
            status,
            title: Some(title.into()),
            last_total_tokens: None,
            host_id: None,
            needs_attention: false,
            subagent_active: false,
            llm_model: Some("opus".into()),
            model_override: None,
            agent_name: None,
            cumulative_cost: Cost::default(),
            context_window: Some(200_000),
            sandbox_status: None,
            git_branch: Some("main".into()),
            workspace: Some("lens".into()),
            reasoning_effort: None,
            activity_summary: activity.into(),
            last_completed_turn: turn,
            harness: Some("claude-native".into()),
        }
    }

    #[gpui::test]
    async fn poller_dispatches_feed_to_card_and_coalesces_burst(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let (fleet_entity, card) = cx.update(|cx| {
            let fleet = FleetStore::new(clock.clone(), cx);
            let card = fleet.update(cx, |f, cx| f.spawn_fake_session(sid.clone(), cx));
            (fleet, card)
        });

        cx.update(|cx| {
            let fleet = fleet_entity.read(cx);
            let fake = fleet.fake.as_ref().expect("fake mode");
            for i in 0..50u32 {
                fake.push_feed(
                    &sid,
                    ActorFeed::Summary(Box::new(summary(
                        SessionStatusValue::Running,
                        &format!("t{i}"),
                        "working",
                        i,
                    ))),
                );
            }
        });
        cx.run_until_parked();

        let (title, notifies, store_n) = cx.read(|cx| {
            let c = card.read(cx);
            let f = fleet_entity.read(cx);
            (c.title.clone(), c.notify_count, f.store_notify_count())
        });
        assert_eq!(title.as_deref(), Some("t49"));
        assert!(
            notifies < 50,
            "burst must coalesce: notify_count={notifies}"
        );
        assert_eq!(
            store_n, 1,
            "FleetStore notified only on membership spawn, not on scalar folds"
        );
    }

    #[gpui::test]
    async fn poller_coalesces_outcome_batch_card_notify(cx: &mut gpui::TestAppContext) {
        use lens_core::actor::ParkReason;

        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let (fleet_entity, card) = cx.update(|cx| {
            let fleet = FleetStore::new(clock.clone(), cx);
            let card = fleet.update(cx, |f, cx| f.spawn_fake_session(sid.clone(), cx));
            (fleet, card)
        });

        let before = cx.read(|cx| card.read(cx).notify_count);

        cx.update(|cx| {
            let fleet = fleet_entity.read(cx);
            let fake = fleet.fake.as_ref().expect("fake mode");
            for _ in 0..5 {
                fake.push_outcome(
                    &sid,
                    ActorOutcome::Parked {
                        reason: ParkReason::SessionFailed,
                    },
                );
            }
        });
        cx.run_until_parked();

        let after = cx.read(|cx| card.read(cx).notify_count);
        assert_eq!(
            after - before,
            1,
            "outcome batch must coalesce card notify: before={before} after={after}"
        );
        assert_eq!(
            cx.read(|cx| card.read(cx).connection_overlay),
            ConnectionOverlay::Disconnected,
            "last Parked outcome must still apply"
        );
    }
}
