use gpui::{
    AnyView, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, StyleRefinement,
    Window, canvas, div, prelude::*, px,
};
use lens_core::actor::SessionCommand;
use lens_core::domain::ids::SessionId;
use std::cell::Cell;
#[cfg(feature = "demo")]
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::clock::UiClock;
use crate::fleet::store::FleetStore;
use crate::theme::ActiveLensTheme as _;

use super::chrome::render_card_chrome;
use super::model::{CARD_HEIGHT_PX, CARD_WIDTH_PX, SessionCard};
use super::motion::{anim_tick_for, render_expanding_ring, ring_phase, sweep_phase};
use super::wave::derive_wave;

/// §4.4: skeleton card chrome uses gpui `div`/text only — no gpui-component widget
/// inside the tile — so cache-key/bounds risk from component internals is N/A.
pub struct SessionCardView {
    card: Entity<SessionCard>,
    clock: Arc<dyn UiClock>,
    fleet: Entity<FleetStore>,
    session_id: SessionId,
    kebab_open: bool,
    /// Per-wave self-notify driver (20fps sweep/spinner, 1Hz Scheduled), live only while
    /// the card's wave animates and is on-screen (approach ②).
    anim_task: Option<gpui::Task<()>>,
    /// Last-started driver interval; respawn when the wave's cadence class changes.
    anim_interval: Option<Duration>,
    pub render_count: Rc<Cell<usize>>,
    pub paint_count: Rc<Cell<usize>>,
    pub last_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl SessionCardView {
    pub fn new(
        card: Entity<SessionCard>,
        clock: Arc<dyn UiClock>,
        fleet: Entity<FleetStore>,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Self {
        // §4.4: observe ONLY this card entity — never FleetStore or sibling cards.
        cx.observe(&card, |_, _, cx| cx.notify()).detach();
        Self {
            card,
            clock,
            fleet,
            session_id,
            kebab_open: false,
            anim_task: None,
            anim_interval: None,
            render_count: Rc::new(Cell::new(0)),
            paint_count: Rc::new(Cell::new(0)),
            last_bounds: Rc::new(Cell::new(None)),
        }
    }

    fn send_command(&self, cmd: SessionCommand, cx: &mut Context<Self>) {
        let fleet = self.fleet.clone();
        let sid = self.session_id.clone();
        fleet.update(cx, |f, _| f.send_session_command(&sid, cmd));
    }
}

impl Render for SessionCardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        let card = self.card.read(cx);
        let now_ms = self.clock.now_millis();
        let wave = derive_wave(card, now_ms, card.is_focused);
        let kebab_open = self.kebab_open;
        let paint_count = self.paint_count.clone();
        let last_bounds = self.last_bounds.clone();
        // Viewport gate: gpui does not auto-cull off-screen Div children, so an off-screen
        // card's timer would keep re-rendering it. Only animate while visible. First frame
        // (no bounds yet) counts as visible; it self-corrects next frame.
        let visible = match self.last_bounds.get() {
            Some(b) => {
                let viewport = gpui::Bounds::new(gpui::Point::default(), window.viewport_size());
                b.intersects(&viewport)
            }
            None => true,
        };
        let desired = anim_tick_for(wave).filter(|_| visible);
        if desired != self.anim_interval {
            self.anim_task = None; // drop cancels the old timer
            self.anim_interval = desired;
            if let Some(interval) = desired {
                self.anim_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(interval).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
        }
        let sweep = sweep_phase(wave, now_ms);
        let ring_color = wave.status_color(cx.lens_theme());
        let ring = ring_phase(now_ms);

        div()
            .id(("session-card", self.card.entity_id()))
            .relative()
            .w(px(CARD_WIDTH_PX))
            .h(px(CARD_HEIGHT_PX))
            .child(render_expanding_ring(wave, ring_color, ring))
            .child(
                div()
                    .size_full()
                    .child(render_card_chrome(
                        card,
                        wave,
                        kebab_open,
                        sweep,
                        now_ms,
                        cx,
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = !view.kebab_open;
                            cx.notify();
                        }),
                        cx.listener(|view, _, _, cx| {
                            let fleet = view.fleet.clone();
                            let sid = view.session_id.clone();
                            fleet.update(cx, |f, _| f.wake_session(&sid));
                        }),
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = false;
                            view.send_command(SessionCommand::Sleep, cx);
                        }),
                        cx.listener(|view, _, _, cx| {
                            view.kebab_open = false;
                            view.send_command(
                                SessionCommand::Send {
                                    text: String::new(),
                                    model_override: None,
                                },
                                cx,
                            );
                        }),
                        cx.listener(|view, _, _, cx| {
                            let fleet = view.fleet.clone();
                            let sid = view.session_id.clone();
                            fleet.update(cx, |f, _| f.retry_session(&sid));
                        }),
                    ))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, _, _| {
                                paint_count.set(paint_count.get() + 1);
                                last_bounds.set(Some(bounds));
                            },
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
    }
}

/// Mount as `AnyView` inside `.cached(...)` with stable bounds style.
pub fn mount_cached_card(view: Entity<SessionCardView>) -> AnyView {
    // §4.4 pt4: pin the CACHED WRAPPER to the fixed tile size — the cache key IS the
    // wrapper's bounds, so it must be 280×148 independent of parent packing (a default
    // style lets a flex/grid parent resize it). Styled is impl'd for StyleRefinement.
    let style = StyleRefinement::default()
        .w(px(CARD_WIDTH_PX))
        .h(px(CARD_HEIGHT_PX));
    AnyView::from(view).cached(style)
}

/// Demo spike: stderr dump of per-card render/paint counters every ~2s.
#[cfg(feature = "demo")]
pub fn spawn_demo_paint_instrumentation(
    card_views: &HashMap<SessionId, Entity<SessionCardView>>,
    cx: &mut gpui::App,
) {
    let views: Vec<_> = card_views.values().cloned().collect();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            for view in &views {
                let _ = view.update(cx, |v, _| {
                    eprintln!(
                        "paint-instr session={} render={} paint={}",
                        v.session_id.as_str(),
                        v.render_count.get(),
                        v.paint_count.get(),
                    );
                });
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Render};
    use lens_core::domain::ids::SessionId;

    /// Parent that mounts two cached card tiles for isolation smoke.
    struct DualCardBoard {
        view_a: Entity<SessionCardView>,
        view_b: Entity<SessionCardView>,
    }

    impl Render for DualCardBoard {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .child(mount_cached_card(self.view_a.clone()))
                .child(mount_cached_card(self.view_b.clone()))
        }
    }

    struct SingleCardBoard {
        view: Entity<SessionCardView>,
    }

    impl Render for SingleCardBoard {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(mount_cached_card(self.view.clone()))
        }
    }

    #[gpui::test]
    async fn session_card_view_observes_own_card_only(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(crate::clock::ManualUiClock::new(0));
        let sid_a = SessionId::new("a");
        let sid_b = SessionId::new("b");

        let (fleet, card_a, card_b, view_a, view_b, rc_a, rc_b) = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            let card_a = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_a.clone(), cx));
            let card_b = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_b.clone(), cx));
            let clock = fleet.read(cx).clock();
            let view_a = cx.new(|cx| {
                SessionCardView::new(
                    card_a.clone(),
                    clock.clone(),
                    fleet.clone(),
                    sid_a.clone(),
                    cx,
                )
            });
            let view_b = cx.new(|cx| {
                SessionCardView::new(card_b.clone(), clock, fleet.clone(), sid_b.clone(), cx)
            });
            let rc_a = view_a.read(cx).render_count.clone();
            let rc_b = view_b.read(cx).render_count.clone();
            (fleet, card_a, card_b, view_a, view_b, rc_a, rc_b)
        });

        let (_board, vcx) = cx.add_window_view(|_, _| DualCardBoard {
            view_a: view_a.clone(),
            view_b: view_b.clone(),
        });

        vcx.run_until_parked();
        let _ = fleet;

        let after_first = (rc_a.get(), rc_b.get());
        assert_eq!(after_first, (1, 1), "initial mount renders both tiles once");

        // Notify card A only — B must not re-render.
        vcx.update(|_, cx| {
            card_a.update(cx, |card, cx| {
                card.title = Some("updated-a".into());
                cx.notify();
            });
        });
        vcx.run_until_parked();
        let a_after_notify = rc_a.get();
        assert!(
            a_after_notify > after_first.0,
            "card A view re-rendered after own entity notify"
        );
        assert_eq!(
            rc_b.get(),
            after_first.1,
            "card B view must not observe card A entity"
        );

        // Sanity: notifying card B does re-render B only.
        vcx.update(|_, cx| {
            card_b.update(cx, |card, cx| {
                card.title = Some("updated-b".into());
                cx.notify();
            });
        });
        vcx.run_until_parked();
        assert!(
            rc_b.get() > after_first.1,
            "card B view re-rendered after own notify"
        );
        assert_eq!(
            rc_a.get(),
            a_after_notify,
            "card A view count unchanged after card B notify"
        );
    }

    #[gpui::test]
    async fn animating_card_does_not_render_a_static_sibling(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::scalars::SessionStatusValue;

        let clock = Arc::new(crate::clock::ManualUiClock::new(0));
        let sid_a = SessionId::new("anim");
        let sid_b = SessionId::new("static");

        let (fleet, view_a, view_b, rc_a, rc_b) = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            let card_a = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_a.clone(), cx));
            let card_b = fleet.update(cx, |f, cx| f.spawn_fake_session(sid_b.clone(), cx));
            card_a.update(cx, |c, _| c.status = SessionStatusValue::Running);
            let ui_clock = fleet.read(cx).clock();
            let view_a = cx.new(|cx| {
                SessionCardView::new(
                    card_a.clone(),
                    ui_clock.clone(),
                    fleet.clone(),
                    sid_a.clone(),
                    cx,
                )
            });
            let view_b = cx.new(|cx| {
                SessionCardView::new(card_b.clone(), ui_clock, fleet.clone(), sid_b.clone(), cx)
            });
            let rc_a = view_a.read(cx).render_count.clone();
            let rc_b = view_b.read(cx).render_count.clone();
            (fleet, view_a, view_b, rc_a, rc_b)
        });

        let (_board, vcx) = cx.add_window_view(|_, _| DualCardBoard {
            view_a: view_a.clone(),
            view_b: view_b.clone(),
        });

        vcx.run_until_parked();
        let baseline_a = rc_a.get();
        let baseline_b = rc_b.get();
        assert_eq!(
            (baseline_a, baseline_b),
            (1, 1),
            "initial mount renders both tiles once"
        );

        for _ in 0..5 {
            vcx.executor().advance_clock(Duration::from_millis(40));
            vcx.run_until_parked();
        }

        let count_a = view_a.read_with(cx, |v, _| v.render_count.get());
        assert!(
            count_a > baseline_a,
            "animating card A must have re-rendered (baseline={baseline_a}, now={count_a})"
        );
        assert_eq!(
            rc_b.get(),
            baseline_b,
            "static sibling B must NOT re-render when A animates (§4.4)"
        );
        let _ = fleet;
    }

    #[gpui::test]
    async fn anim_driver_respawns_on_cadence_class_change(cx: &mut gpui::TestAppContext) {
        use lens_core::domain::scalars::{SessionLifecycle, SessionStatusValue};

        let clock = Arc::new(crate::clock::ManualUiClock::new(0));
        let sid = SessionId::new("cadence");

        let (card, view, rc) = cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install_at_startup(cx);
            let fleet = FleetStore::new(clock, cx);
            let card = fleet.update(cx, |f, cx| f.spawn_fake_session(sid.clone(), cx));
            card.update(cx, |c, _| {
                c.status = SessionStatusValue::Idle;
                c.lifecycle = SessionLifecycle::Active;
                c.scheduled_wake_at = Some(60_000);
                c.scheduled_started_at = Some(0);
            });
            let ui_clock = fleet.read(cx).clock();
            let view =
                cx.new(|cx| SessionCardView::new(card.clone(), ui_clock, fleet, sid.clone(), cx));
            let rc = view.read(cx).render_count.clone();
            (card, view, rc)
        });

        let (_board, vcx) = cx.add_window_view(|_, _| SingleCardBoard { view: view.clone() });

        vcx.run_until_parked();
        assert_eq!(rc.get(), 1, "initial mount");

        // 1 Hz Scheduled driver: one extra render per second.
        vcx.executor().advance_clock(Duration::from_millis(1000));
        vcx.run_until_parked();
        let after_1hz = rc.get();
        assert!(
            after_1hz > 1,
            "Scheduled 1Hz driver should tick at least once (now={after_1hz})"
        );

        // Direct Scheduled → Working: driver must respawn at ~50ms (20fps), not keep 1Hz.
        vcx.update(|_, cx| {
            card.update(cx, |c, cx| {
                c.status = SessionStatusValue::Running;
                cx.notify();
            });
        });
        vcx.run_until_parked();
        let after_transition = rc.get();

        for _ in 0..5 {
            vcx.executor().advance_clock(Duration::from_millis(50));
            vcx.run_until_parked();
        }

        let after_fast = rc.get();
        let fast_delta = after_fast - after_transition;
        assert!(
            fast_delta > 3,
            "Working ~20fps driver should produce >3 renders in 250ms (got {fast_delta}; \
             after_1hz={after_1hz}, after_transition={after_transition}, after_fast={after_fast})"
        );
        let _ = view;
    }
}
