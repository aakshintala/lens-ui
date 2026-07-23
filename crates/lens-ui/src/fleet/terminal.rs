//! Terminal membership + fleet policy (Terminal Slice 5 sub-slice C).

use crate::fleet::store::FleetStore;
use gpui::{Context, Entity, Subscription};
use lens_client::Client;
use lens_core::actor::TerminalResourceSignal;
use lens_core::domain::ids::SessionId;
use lens_terminal::{
    Lifecycle, TerminalEvent, TerminalHostEvent, TerminalKey, TerminalOpenOptions, TerminalTab,
    TerminalTarget,
};
use std::sync::Arc;

/// Provisional hidden-idle threshold (~10 min) in [`crate::clock::UiClock`] millis.
pub const TERMINAL_IDLE_SLEEP_THRESHOLD_MS: u64 = 10 * 60 * 1000;

/// Session-level control signals `FleetStore` owns (design §4.1). The poller
/// converts the two `ActorOutcome` control variants into this typed form so the
/// store never matches outcomes it does not own, and so the
/// `target_conversation_id: String -> SessionId` conversion happens once.
pub(crate) enum SessionControl {
    Superseded { target: SessionId, reason: String },
    TerminalResource(TerminalResourceSignal),
}

/// Fleet memory-pressure signal (Slice 6 wires the OS source).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemoryPressure {
    Warning { free_fraction: f32 },
    Critical,
}

/// A terminal owned by a session in [`FleetStore`].
pub struct TerminalMember {
    pub tab: Entity<TerminalTab>,
    pub last_viewed: u64,
    pub hidden: bool,
    pub pending_sleep: bool,
    _sub: Subscription,
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) struct TerminalKeyId {
    terminal_name: String,
    session_key: String,
}

impl TerminalKeyId {
    fn from_key(key: &TerminalKey) -> Self {
        Self {
            terminal_name: key.terminal_name.clone(),
            session_key: key.session_key.clone(),
        }
    }
}

fn is_sleepable(lifecycle: Lifecycle) -> bool {
    matches!(
        lifecycle,
        Lifecycle::Live | Lifecycle::Reconnecting | Lifecycle::ReplacementWaiting
    )
}

fn is_policy_eligible(hidden: bool, lifecycle: Lifecycle) -> bool {
    hidden && lifecycle == Lifecycle::Live
}

impl FleetStore {
    pub fn open_terminal(
        &mut self,
        target: TerminalTarget,
        client: Arc<Client>,
        options: TerminalOpenOptions,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalTab> {
        let tab = lens_terminal::open(target.clone(), client, options, cx);
        let key = terminal_key_from_target(&target);
        let session_id = session_id_from_target(&target);
        self.register_terminal_member(session_id, key, tab.clone(), cx);
        tab
    }

    pub fn set_terminal_visible(
        &mut self,
        session_id: &SessionId,
        terminal_key: &TerminalKey,
        visible: bool,
        _cx: &mut Context<Self>,
    ) {
        let now = self.clock().now_millis().max(0) as u64;
        let Some(member) = self
            .terminals
            .get_mut(session_id)
            .and_then(|m| m.get_mut(&TerminalKeyId::from_key(terminal_key)))
        else {
            return;
        };
        member.last_viewed = now;
        member.hidden = !visible;
        if visible {
            member.pending_sleep = false;
        }
    }

    pub fn close_terminal(
        &mut self,
        session_id: &SessionId,
        terminal_key: &TerminalKey,
        cx: &mut Context<Self>,
    ) {
        let removed = self
            .terminals
            .get_mut(session_id)
            .and_then(|map| map.remove(&TerminalKeyId::from_key(terminal_key)));
        if let Some(member) = removed {
            end_member_tab(&member, cx);
        }
        if self.terminals.get(session_id).is_some_and(|m| m.is_empty()) {
            self.terminals.remove(session_id);
        }
    }

    pub fn cascade_sleep(&mut self, session_id: &SessionId, cx: &mut Context<Self>) {
        let Some(map) = self.terminals.get_mut(session_id) else {
            return;
        };
        for member in map.values_mut() {
            let lifecycle = member.tab.read(cx).presentation().lifecycle;
            if is_sleepable(lifecycle) {
                member.tab.update(cx, |tab, cx| {
                    tab.on_host_event(TerminalHostEvent::Sleep, cx);
                });
                member.pending_sleep = false;
            } else {
                member.pending_sleep = true;
            }
        }
    }

    pub fn cascade_wake(&mut self, session_id: &SessionId, cx: &mut Context<Self>) {
        let Some(map) = self.terminals.get_mut(session_id) else {
            return;
        };
        for member in map.values_mut() {
            if !member.hidden {
                // Cancel any sleep deferred by an earlier `cascade_sleep` while this
                // member was still transient — otherwise it would sleep under an awake
                // session once it reaches Live (see `on_terminal_presentation_changed`).
                member.pending_sleep = false;
                member.tab.update(cx, |tab, cx| {
                    tab.on_host_event(TerminalHostEvent::Wake, cx);
                });
            }
        }
    }

    pub fn cascade_end(&mut self, session_id: &SessionId, cx: &mut Context<Self>) {
        if let Some(map) = self.terminals.remove(session_id) {
            for member in map.into_values() {
                end_member_tab(&member, cx);
            }
        }
    }

    /// Entry point for session-level control outcomes routed by the poller.
    pub(crate) fn on_session_control(
        &mut self,
        session_id: &SessionId,
        signal: SessionControl,
        cx: &mut Context<Self>,
    ) {
        match signal {
            SessionControl::TerminalResource(signal) => {
                self.forward_terminal_resource(session_id, signal, cx);
            }
            SessionControl::Superseded { target, reason } => {
                // Task 5 replaces this body with load-B + move + Transfer.
                let _ = (target, reason);
            }
        }
    }

    /// Fan a resource signal out to every terminal owned by `session_id`. The
    /// tab filters to its own identity (Slice-4 contract), so a broadcast to
    /// the session's terminals is correct and keeps the store free of
    /// terminal-identity logic.
    fn forward_terminal_resource(
        &mut self,
        session_id: &SessionId,
        signal: TerminalResourceSignal,
        cx: &mut Context<Self>,
    ) {
        let Some(inner) = self.terminals.get(session_id) else {
            return;
        };
        let event = match signal {
            TerminalResourceSignal::Created {
                terminal_id,
                terminal_name,
                session_key,
                session_id,
            } => TerminalHostEvent::ResourceCreated {
                session_id,
                terminal_id,
                terminal_name,
                session_key,
            },
            TerminalResourceSignal::Deleted { terminal_id } => {
                TerminalHostEvent::ResourceDeleted { terminal_id }
            }
        };
        let tabs: Vec<_> = inner.values().map(|m| m.tab.clone()).collect();
        for tab in tabs {
            tab.update(cx, |tab, cx| {
                tab.on_host_event(event.clone(), cx);
            });
        }
    }

    pub fn on_memory_pressure(&mut self, pressure: MemoryPressure, cx: &mut Context<Self>) {
        let mut eligible: Vec<(SessionId, TerminalKey, u64, usize)> = self
            .terminals
            .iter()
            .flat_map(|(session_id, map)| {
                map.iter().filter_map(|(key_id, member)| {
                    let lifecycle = member.tab.read(cx).presentation().lifecycle;
                    if is_policy_eligible(member.hidden, lifecycle) {
                        Some((
                            session_id.clone(),
                            terminal_key_from_id(key_id),
                            member.last_viewed,
                            member.tab.read(cx).retained_bytes_estimate(),
                        ))
                    } else {
                        None
                    }
                })
            })
            .collect();

        match pressure {
            MemoryPressure::Critical => {
                for (session_id, key, _, _) in eligible {
                    self.policy_sleep_terminal(&session_id, &key, cx);
                }
            }
            MemoryPressure::Warning { free_fraction } => {
                let total_estimate: usize = eligible.iter().map(|(_, _, _, est)| *est).sum();
                if total_estimate == 0 {
                    return;
                }
                let target_free =
                    (total_estimate as f64 * f64::from(free_fraction)).floor() as usize;
                eligible.sort_by_key(|(_, _, last_viewed, _)| *last_viewed);
                let mut freed = 0usize;
                for (session_id, key, _, estimate) in eligible {
                    if freed >= target_free {
                        break;
                    }
                    self.policy_sleep_terminal(&session_id, &key, cx);
                    freed = freed.saturating_add(estimate);
                }
            }
        }
    }

    /// Sleep hidden terminals idle past [`TERMINAL_IDLE_SLEEP_THRESHOLD_MS`].
    ///
    /// DRIVER DEFERRED (codex B+C review, finding 5): this is the policy body only.
    /// No production ~30s periodic caller exists yet — FleetStore's terminal API has
    /// no production driver until Slice 6/D, which must install the bounded periodic
    /// tick (and the OS memory-warning source) when it wires the fleet in.
    pub fn idle_tick(&mut self, cx: &mut Context<Self>) {
        let now = self.clock().now_millis().max(0) as u64;
        let threshold = TERMINAL_IDLE_SLEEP_THRESHOLD_MS;
        let to_sleep: Vec<(SessionId, TerminalKey)> = self
            .terminals
            .iter()
            .flat_map(|(session_id, map)| {
                map.iter().filter_map(|(key_id, member)| {
                    let lifecycle = member.tab.read(cx).presentation().lifecycle;
                    if is_policy_eligible(member.hidden, lifecycle)
                        && now.saturating_sub(member.last_viewed) >= threshold
                    {
                        Some((session_id.clone(), terminal_key_from_id(key_id)))
                    } else {
                        None
                    }
                })
            })
            .collect();
        for (session_id, key) in to_sleep {
            self.policy_sleep_terminal(&session_id, &key, cx);
        }
    }

    fn register_terminal_member(
        &mut self,
        session_id: SessionId,
        key: TerminalKey,
        tab: Entity<TerminalTab>,
        cx: &mut Context<Self>,
    ) {
        let now = self.clock().now_millis().max(0) as u64;
        let key_id = TerminalKeyId::from_key(&key);
        let session_for_sub = session_id.clone();
        let key_for_sub = key.clone();
        let sub = cx.subscribe(&tab, move |store, _tab, event, cx| {
            if matches!(event, TerminalEvent::PresentationChanged) {
                store.on_terminal_presentation_changed(&session_for_sub, &key_for_sub, cx);
            }
        });
        let member = TerminalMember {
            tab,
            last_viewed: now,
            hidden: false,
            pending_sleep: false,
            _sub: sub,
        };
        // Re-opening the same logical key replaces the tracked member. Tear the
        // previous tab down so it can't linger with a live engine/transport outside
        // fleet accounting (codex finding 3).
        let previous = self
            .terminals
            .entry(session_id)
            .or_default()
            .insert(key_id, member);
        if let Some(previous) = previous {
            end_member_tab(&previous, cx);
        }
    }

    fn on_terminal_presentation_changed(
        &mut self,
        session_id: &SessionId,
        key: &TerminalKey,
        cx: &mut Context<Self>,
    ) {
        let Some(member) = self
            .terminals
            .get_mut(session_id)
            .and_then(|m| m.get_mut(&TerminalKeyId::from_key(key)))
        else {
            return;
        };
        if !member.pending_sleep {
            return;
        }
        let lifecycle = member.tab.read(cx).presentation().lifecycle;
        if is_sleepable(lifecycle) {
            member.tab.update(cx, |tab, cx| {
                tab.on_host_event(TerminalHostEvent::Sleep, cx);
            });
            member.pending_sleep = false;
        }
    }

    fn policy_sleep_terminal(
        &mut self,
        session_id: &SessionId,
        key: &TerminalKey,
        cx: &mut Context<Self>,
    ) {
        let Some(member) = self
            .terminals
            .get_mut(session_id)
            .and_then(|m| m.get_mut(&TerminalKeyId::from_key(key)))
        else {
            return;
        };
        let lifecycle = member.tab.read(cx).presentation().lifecycle;
        if !is_policy_eligible(member.hidden, lifecycle) {
            return;
        }
        member.tab.update(cx, |tab, cx| {
            tab.on_host_event(TerminalHostEvent::Sleep, cx);
        });
    }

    #[cfg(test)]
    pub(crate) fn insert_terminal_for_test(
        &mut self,
        session_id: SessionId,
        key: TerminalKey,
        tab: Entity<TerminalTab>,
        cx: &mut Context<Self>,
    ) {
        self.register_terminal_member(session_id, key, tab, cx);
    }

    #[cfg(test)]
    pub(crate) fn terminal_member_for_test(
        &self,
        session_id: &SessionId,
        key: &TerminalKey,
        _cx: &gpui::App,
    ) -> Option<&TerminalMember> {
        self.terminals
            .get(session_id)
            .and_then(|m| m.get(&TerminalKeyId::from_key(key)))
    }

    #[cfg(test)]
    pub(crate) fn set_member_pending_sleep_for_test(
        &mut self,
        session_id: &SessionId,
        key: &TerminalKey,
        pending: bool,
    ) {
        if let Some(member) = self
            .terminals
            .get_mut(session_id)
            .and_then(|m| m.get_mut(&TerminalKeyId::from_key(key)))
        {
            member.pending_sleep = pending;
        }
    }

    #[cfg(test)]
    pub(crate) fn lrv_hidden_last_viewed_for_test(&self) -> Vec<u64> {
        let mut stamps: Vec<u64> = self
            .terminals
            .values()
            .flat_map(|m| m.values())
            .filter(|member| member.hidden)
            .map(|member| member.last_viewed)
            .collect();
        stamps.sort_unstable();
        stamps
    }

    #[cfg(test)]
    pub(crate) fn terminal_count_for_test(&self) -> usize {
        self.terminals.values().map(|m| m.len()).sum()
    }
}

/// Host-driven teardown of a member's tab: releases the engine + transport so a
/// caller-held entity clone cannot keep the runtime alive after the member is
/// dropped from fleet accounting.
fn end_member_tab(member: &TerminalMember, cx: &mut Context<FleetStore>) {
    member.tab.update(cx, |tab, cx| {
        tab.on_host_event(TerminalHostEvent::End, cx);
    });
}

fn session_id_from_target(target: &TerminalTarget) -> SessionId {
    match target {
        TerminalTarget::Existing { session_id, .. }
        | TerminalTarget::OpenOrCreate { session_id, .. } => session_id.clone(),
    }
}

fn terminal_key_from_target(target: &TerminalTarget) -> TerminalKey {
    match target {
        TerminalTarget::OpenOrCreate { key, .. } => key.clone(),
        // PROVISIONAL (codex B+C review, finding 7): an `Existing` target has no
        // logical key, so we synthesize one with an empty `session_key`. This is a
        // private sentinel — the public `TerminalKey`-addressed APIs
        // (`set_terminal_visible`/`close_terminal`) cannot round-trip it, so an
        // `Existing`-opened terminal is not addressable by logical metadata today.
        // No production path opens an `Existing` target via `FleetStore` yet; Slice
        // 6 must replace this + the inner map key with an honest identity enum
        // (`Existing(TerminalId) | Logical(TerminalKey)`) once its addressing API is
        // concrete. Deferred to avoid premature layer-boundary binding.
        TerminalTarget::Existing { terminal_id, .. } => TerminalKey {
            terminal_name: terminal_id.to_string(),
            session_key: String::new(),
        },
    }
}

fn terminal_key_from_id(id: &TerminalKeyId) -> TerminalKey {
    TerminalKey {
        terminal_name: id.terminal_name.clone(),
        session_key: id.session_key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use lens_client::ids::TerminalId;
    use lens_core::actor::TerminalResourceSignal;
    use lens_terminal::{EngineConfig, EngineHandle, PER_CELL_BYTES, TerminalHostEvent};
    use std::sync::Arc;

    fn test_engine_cfg() -> EngineConfig {
        EngineConfig {
            cols: 10,
            rows: 3,
            max_scrollback: 10_000,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    fn spawn_tab_with_rows(
        cx: &mut gpui::TestAppContext,
        feed_newlines: usize,
    ) -> (Arc<EngineHandle>, Entity<TerminalTab>) {
        let engine = Arc::new(EngineHandle::spawn(test_engine_cfg()).expect("engine"));
        for _ in 0..feed_newlines {
            engine.feed(b"\n".to_vec()).expect("feed");
        }
        // `retained_bytes_estimate` is sampled by the worker on build, ASYNC to this
        // thread. When we fed rows, `feed`+`build_now` are channel-ordered, so the BuildNow
        // runs after all feeds; wait until `frames_built` advances past its pre-build value
        // to guarantee `total_rows`/estimate are fully sampled before any policy reads them.
        // Without this the estimate races to 0 and pressure tests flake. Mirrors
        // lens-terminal's `handle_inspect_reports_retained_estimate_after_streaming`.
        // Skipped when nothing was fed: `build_now` is a no-op when the engine is not dirty
        // (so `frames_built` would never advance), and those tests don't read the estimate.
        if feed_newlines > 0 {
            let pre_frames = engine.inspect().frames_built;
            engine.build_now().expect("build_now");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while engine.inspect().frames_built <= pre_frames {
                assert!(
                    std::time::Instant::now() < deadline,
                    "engine worker did not build after feed within deadline"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        cx.run_until_parked();
        let tab = cx.update(|cx| TerminalTab::open_with_engine_for_test(Arc::clone(&engine), cx));
        (engine, tab)
    }

    fn estimate_for_rows(rows: usize, cols: u16) -> usize {
        rows.saturating_mul(cols as usize)
            .saturating_mul(PER_CELL_BYTES)
    }

    fn test_key(name: &str, session_key: &str) -> TerminalKey {
        TerminalKey {
            terminal_name: name.into(),
            session_key: session_key.into(),
        }
    }

    fn test_target(session_id: &SessionId, key: &TerminalKey) -> TerminalTarget {
        TerminalTarget::OpenOrCreate {
            session_id: session_id.clone(),
            key: key.clone(),
        }
    }

    #[gpui::test]
    async fn register_and_close_terminal(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab, cx);
                assert_eq!(store.terminal_count_for_test(), 1);
            });
        });

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.close_terminal(&sid, &key, cx);
                assert_eq!(store.terminal_count_for_test(), 0);
            });
        });
    }

    #[gpui::test]
    async fn open_terminal_visible_on_open(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(5_000));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab, cx);
            });
        });

        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(!member.hidden);
            assert_eq!(member.last_viewed, 5_000);
            assert!(!member.pending_sleep);
        });
    }

    #[gpui::test]
    async fn set_terminal_visible_stamps_last_viewed(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock.clone(), cx));
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab, cx);
            });
        });

        clock.set(9_000);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&sid, &key, false, cx);
            });
        });
        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(member.hidden);
            assert_eq!(member.last_viewed, 9_000);
        });

        clock.set(12_000);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&sid, &key, true, cx);
            });
        });
        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(!member.hidden);
            assert_eq!(member.last_viewed, 12_000);
        });
    }

    #[gpui::test]
    async fn lrv_order_across_sessions(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let s1 = SessionId::new("s1");
        let s2 = SessionId::new("s2");
        let k1 = test_key("a", "k1");
        let k2 = test_key("b", "k2");
        let k3 = test_key("c", "k3");
        let fleet = cx.update(|cx| FleetStore::new(clock.clone(), cx));

        let (_e1, tab1) = spawn_tab_with_rows(cx, 0);
        let (_e2, tab2) = spawn_tab_with_rows(cx, 0);
        let (_e3, tab3) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(s1.clone(), k1.clone(), tab1, cx);
                store.insert_terminal_for_test(s2.clone(), k2.clone(), tab2, cx);
                store.insert_terminal_for_test(s1.clone(), k3.clone(), tab3, cx);
                store.set_terminal_visible(&s1, &k1, false, cx);
                store.set_terminal_visible(&s2, &k2, false, cx);
                store.set_terminal_visible(&s1, &k3, false, cx);
            });
        });

        clock.set(100);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&s1, &k1, false, cx);
            });
        });
        clock.set(200);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&s2, &k2, false, cx);
            });
        });
        clock.set(300);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&s1, &k3, false, cx);
            });
        });

        cx.read(|cx| {
            let store = fleet.read(cx);
            assert_eq!(store.lrv_hidden_last_viewed_for_test(), vec![100, 200, 300]);
        });
    }

    #[gpui::test]
    async fn cascade_sleep_all_and_wake_non_hidden(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let visible_key = test_key("vis", "k1");
        let hidden_key = test_key("hid", "k2");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        let (_e1, tab1) = spawn_tab_with_rows(cx, 0);
        let (_e2, tab2) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), visible_key.clone(), tab1, cx);
                store.insert_terminal_for_test(sid.clone(), hidden_key.clone(), tab2, cx);
                store.set_terminal_visible(&sid, &hidden_key, false, cx);
            });
        });

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.cascade_sleep(&sid, cx);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            let vis = store
                .terminal_member_for_test(&sid, &visible_key, cx)
                .unwrap();
            let hid = store
                .terminal_member_for_test(&sid, &hidden_key, cx)
                .unwrap();
            assert_eq!(
                vis.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
            assert_eq!(
                hid.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
        });

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.cascade_wake(&sid, cx);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            let vis = store
                .terminal_member_for_test(&sid, &visible_key, cx)
                .unwrap();
            let hid = store
                .terminal_member_for_test(&sid, &hidden_key, cx)
                .unwrap();
            assert_ne!(
                vis.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping,
                "visible terminal must receive wake"
            );
            assert_eq!(
                hid.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping,
                "hidden terminal must stay sleeping on cascade wake"
            );
        });
    }

    #[gpui::test]
    async fn cascade_sleep_defers_for_starting_then_applies_on_live(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let target = test_target(&sid, &key);
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        let starting_tab = cx.update(|cx| {
            lens_terminal::open(
                target,
                Arc::new(Client::stub_for_test()),
                TerminalOpenOptions::default(),
                cx,
            )
        });
        assert_eq!(
            starting_tab.read_with(cx, |tab, _| tab.presentation().lifecycle),
            Lifecycle::Starting
        );

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), starting_tab.clone(), cx);
                store.cascade_sleep(&sid, cx);
            });
        });

        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(member.pending_sleep);
            assert_ne!(
                member.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
        });

        let (_engine, live_tab) = spawn_tab_with_rows(cx, 0);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.close_terminal(&sid, &key, cx);
                store.insert_terminal_for_test(sid.clone(), key.clone(), live_tab.clone(), cx);
                store.set_member_pending_sleep_for_test(&sid, &key, true);
                live_tab.update(cx, |_tab, cx| {
                    cx.emit(TerminalEvent::PresentationChanged);
                });
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(!member.pending_sleep);
            assert_eq!(
                member.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
        });
    }

    #[gpui::test]
    async fn cascade_wake_cancels_pending_sleep_for_visible_starting(
        cx: &mut gpui::TestAppContext,
    ) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let target = test_target(&sid, &key);
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        let starting_tab = cx.update(|cx| {
            lens_terminal::open(
                target,
                Arc::new(Client::stub_for_test()),
                TerminalOpenOptions::default(),
                cx,
            )
        });
        assert_eq!(
            starting_tab.read_with(cx, |tab, _| tab.presentation().lifecycle),
            Lifecycle::Starting
        );

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), starting_tab.clone(), cx);
                // Session sleeps while the terminal is still Starting → sleep is deferred.
                store.cascade_sleep(&sid, cx);
                // Session wakes again *before* attach completes → the deferred sleep must be
                // cancelled, else the member sleeps under an awake session when it reaches Live.
                store.cascade_wake(&sid, cx);
            });
        });

        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert!(
                !member.pending_sleep,
                "cascade_wake must cancel a deferred cascade sleep for a visible member"
            );
        });
    }

    #[gpui::test]
    async fn memory_pressure_warning_fraction_freed(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let fleet = cx.update(|cx| FleetStore::new(clock.clone(), cx));

        let (_e1, tab1) = spawn_tab_with_rows(cx, 50);
        let (_e2, tab2) = spawn_tab_with_rows(cx, 10);
        let k1 = test_key("big", "k1");
        let k2 = test_key("small", "k2");

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), k1.clone(), tab1, cx);
                store.insert_terminal_for_test(sid.clone(), k2.clone(), tab2, cx);
                store.set_terminal_visible(&sid, &k1, false, cx);
                store.set_terminal_visible(&sid, &k2, false, cx);
            });
        });

        clock.set(100);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&sid, &k1, false, cx);
            });
        });
        clock.set(200);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.set_terminal_visible(&sid, &k2, false, cx);
            });
        });

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.on_memory_pressure(MemoryPressure::Warning { free_fraction: 0.5 }, cx);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            let big = store.terminal_member_for_test(&sid, &k1, cx).unwrap();
            let small = store.terminal_member_for_test(&sid, &k2, cx).unwrap();
            assert_eq!(
                big.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
            assert_eq!(small.tab.read(cx).presentation().lifecycle, Lifecycle::Live);
        });
    }

    #[gpui::test]
    async fn memory_pressure_critical_sleeps_all_eligible(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        let (_e1, tab1) = spawn_tab_with_rows(cx, 5);
        let (_e2, tab2) = spawn_tab_with_rows(cx, 5);
        let k1 = test_key("a", "k1");
        let k2 = test_key("b", "k2");

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), k1.clone(), tab1, cx);
                store.insert_terminal_for_test(sid.clone(), k2.clone(), tab2, cx);
                store.set_terminal_visible(&sid, &k1, false, cx);
                store.set_terminal_visible(&sid, &k2, false, cx);
            });
        });

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.on_memory_pressure(MemoryPressure::Critical, cx);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            assert_eq!(
                store
                    .terminal_member_for_test(&sid, &k1, cx)
                    .unwrap()
                    .tab
                    .read(cx)
                    .presentation()
                    .lifecycle,
                Lifecycle::Sleeping
            );
            assert_eq!(
                store
                    .terminal_member_for_test(&sid, &k2, cx)
                    .unwrap()
                    .tab
                    .read(cx)
                    .presentation()
                    .lifecycle,
                Lifecycle::Sleeping
            );
        });
    }

    #[gpui::test]
    async fn memory_pressure_skips_non_hidden_and_transient(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));

        let (_visible, visible_tab) = spawn_tab_with_rows(cx, 20);
        let starting_tab = cx.update(|cx| {
            lens_terminal::open(
                test_target(&sid, &test_key("starting", "k0")),
                Arc::new(Client::stub_for_test()),
                TerminalOpenOptions::default(),
                cx,
            )
        });

        let visible_key = test_key("visible", "k1");
        let starting_key = test_key("starting", "k0");

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), visible_key.clone(), visible_tab, cx);
                store.insert_terminal_for_test(sid.clone(), starting_key.clone(), starting_tab, cx);
                store.on_memory_pressure(MemoryPressure::Critical, cx);
            });
        });

        cx.read(|cx| {
            let store = fleet.read(cx);
            let visible = store
                .terminal_member_for_test(&sid, &visible_key, cx)
                .unwrap();
            let starting = store
                .terminal_member_for_test(&sid, &starting_key, cx)
                .unwrap();
            assert_eq!(
                visible.tab.read(cx).presentation().lifecycle,
                Lifecycle::Live,
                "non-hidden must never be policy-slept"
            );
            assert_eq!(
                starting.tab.read(cx).presentation().lifecycle,
                Lifecycle::Starting,
                "transient lifecycle must be exempt"
            );
        });
    }

    #[gpui::test]
    async fn idle_tick_sleeps_hidden_over_threshold(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock.clone(), cx));
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab, cx);
                store.set_terminal_visible(&sid, &key, false, cx);
            });
        });

        clock.set(TERMINAL_IDLE_SLEEP_THRESHOLD_MS as i64);
        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.idle_tick(cx);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            let store = fleet.read(cx);
            let member = store
                .terminal_member_for_test(&sid, &key, cx)
                .expect("member");
            assert_eq!(
                member.tab.read(cx).presentation().lifecycle,
                Lifecycle::Sleeping
            );
        });
    }

    #[gpui::test]
    async fn cascade_end_removes_session_terminals(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab, cx);
                store.cascade_end(&sid, cx);
                assert_eq!(store.terminal_count_for_test(), 0);
            });
        });
    }

    #[gpui::test]
    async fn close_terminal_tears_down_the_tab(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        // Caller holds a strong entity clone (as the UI would) alongside the store's.
        let (_engine, tab) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), tab.clone(), cx);
                store.close_terminal(&sid, &key, cx);
                assert_eq!(store.terminal_count_for_test(), 0);
            });
        });
        cx.run_until_parked();

        // The caller-held tab must be torn down — membership removal alone leaves the
        // engine/transport alive on the lingering entity (codex finding 3).
        cx.read(|cx| {
            assert_eq!(
                tab.read(cx).presentation().lifecycle,
                Lifecycle::Ended,
                "close_terminal must tear the tab down, not just drop the map entry"
            );
        });
    }

    #[gpui::test]
    async fn cascade_end_tears_down_each_tab(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let k1 = test_key("a", "k1");
        let k2 = test_key("b", "k2");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        let (_e1, tab1) = spawn_tab_with_rows(cx, 0);
        let (_e2, tab2) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), k1.clone(), tab1.clone(), cx);
                store.insert_terminal_for_test(sid.clone(), k2.clone(), tab2.clone(), cx);
                store.cascade_end(&sid, cx);
                assert_eq!(store.terminal_count_for_test(), 0);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            assert_eq!(tab1.read(cx).presentation().lifecycle, Lifecycle::Ended);
            assert_eq!(tab2.read(cx).presentation().lifecycle, Lifecycle::Ended);
        });
    }

    #[gpui::test]
    async fn double_open_ends_the_previous_tab(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let sid = SessionId::new("s1");
        let key = test_key("main", "k1");
        let fleet = cx.update(|cx| FleetStore::new(clock, cx));
        let (_e1, first) = spawn_tab_with_rows(cx, 0);
        let (_e2, second) = spawn_tab_with_rows(cx, 0);

        cx.update(|cx| {
            fleet.update(cx, |store, cx| {
                store.insert_terminal_for_test(sid.clone(), key.clone(), first.clone(), cx);
                // Re-open the same logical key: the prior member must be torn down, not
                // silently replaced and leaked outside fleet accounting.
                store.insert_terminal_for_test(sid.clone(), key.clone(), second.clone(), cx);
                assert_eq!(store.terminal_count_for_test(), 1);
            });
        });
        cx.run_until_parked();

        cx.read(|cx| {
            assert_eq!(
                first.read(cx).presentation().lifecycle,
                Lifecycle::Ended,
                "re-opening a live key must end the previous tab"
            );
            assert_ne!(
                second.read(cx).presentation().lifecycle,
                Lifecycle::Ended,
                "the replacement tab must stay live"
            );
        });
    }

    #[gpui::test]
    async fn resource_signal_forwards_to_owned_terminals_only(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));

        let sess_a = SessionId::new("conv_a");
        let sess_b = SessionId::new("conv_b");
        let key_a = test_key("main", "sk_a");
        let key_b = test_key("main", "sk_b");

        let (_e1, tab_a) = spawn_tab_with_rows(cx, 0);
        let (_e2, tab_b) = spawn_tab_with_rows(cx, 0);
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.insert_terminal_for_test(sess_a.clone(), key_a.clone(), tab_a.clone(), cx);
                store.insert_terminal_for_test(sess_b.clone(), key_b.clone(), tab_b.clone(), cx);
            });
        });

        let signal = TerminalResourceSignal::Deleted {
            terminal_id: TerminalId::new("term_1"),
        };
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.on_session_control(&sess_a, SessionControl::TerminalResource(signal), cx);
            });
        });

        cx.update(|cx| {
            let a_events = tab_a.read(cx).host_events_for_test().to_vec();
            assert_eq!(
                a_events.len(),
                1,
                "owned terminal got exactly one host event"
            );
            assert!(
                matches!(a_events[0], TerminalHostEvent::ResourceDeleted { .. }),
                "owned terminal got ResourceDeleted, got {:?}",
                a_events[0]
            );
            assert!(
                tab_b.read(cx).host_events_for_test().is_empty(),
                "a terminal owned by a DIFFERENT session must not be forwarded to"
            );
        });
    }

    #[gpui::test]
    async fn resource_created_forwards_full_identity(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(1_000));
        let store = cx.update(|cx| FleetStore::new(clock.clone(), cx));
        let sess = SessionId::new("conv_a");
        let key = test_key("main", "sk_a");
        let (_e, tab) = spawn_tab_with_rows(cx, 0);
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.insert_terminal_for_test(sess.clone(), key.clone(), tab.clone(), cx);
            });
        });

        let signal = TerminalResourceSignal::Created {
            terminal_id: TerminalId::new("term_9"),
            terminal_name: "main".into(),
            session_key: "sk_a".into(),
            session_id: sess.clone(),
        };
        cx.update(|cx| {
            store.update(cx, |store, cx| {
                store.on_session_control(&sess, SessionControl::TerminalResource(signal), cx);
            });
        });

        cx.update(|cx| {
            let events = tab.read(cx).host_events_for_test().to_vec();
            assert_eq!(events.len(), 1);
            match &events[0] {
                TerminalHostEvent::ResourceCreated {
                    session_id,
                    terminal_id,
                    terminal_name,
                    session_key,
                } => {
                    assert_eq!(session_id, &sess);
                    assert_eq!(terminal_id.as_str(), "term_9");
                    assert_eq!(terminal_name, "main");
                    assert_eq!(session_key, "sk_a");
                }
                other => panic!("expected ResourceCreated, got {other:?}"),
            }
        });
    }

    #[allow(dead_code)]
    fn _estimate_helper_doc(rows: usize) -> usize {
        estimate_for_rows(rows, 10)
    }
}
