use gpui::{
    AnyView, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, StyleRefinement,
    Window, canvas, div, prelude::*, px,
};
use lens_core::actor::SessionCommand;
use lens_core::domain::ids::SessionId;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::clock::UiClock;
use crate::fleet::store::FleetStore;

use super::chrome::render_card_chrome;
use super::model::{CARD_HEIGHT_PX, CARD_WIDTH_PX, SessionCard};
use super::wave::derive_wave;

/// §4.4: skeleton card chrome uses gpui `div`/text only — no gpui-component widget
/// inside the tile — so cache-key/bounds risk from component internals is N/A.
pub struct SessionCardView {
    card: Entity<SessionCard>,
    clock: Arc<dyn UiClock>,
    fleet: Entity<FleetStore>,
    session_id: SessionId,
    kebab_open: bool,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        let card = self.card.read(cx);
        let now_ms = self.clock.now_millis();
        let wave = derive_wave(card, now_ms, card.is_focused);
        let kebab_open = self.kebab_open;
        let paint_count = self.paint_count.clone();
        let last_bounds = self.last_bounds.clone();

        div()
            .id(("session-card", self.card.entity_id()))
            .w(px(CARD_WIDTH_PX))
            .h(px(CARD_HEIGHT_PX))
            .child(render_card_chrome(
                card,
                wave,
                kebab_open,
                cx.listener(|view, _, _, cx| {
                    view.kebab_open = !view.kebab_open;
                    cx.notify();
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
                    view.send_command(
                        SessionCommand::Send {
                            text: String::new(),
                            model_override: None,
                        },
                        cx,
                    );
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

    #[gpui::test]
    async fn session_card_view_observes_own_card_only(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(crate::clock::ManualUiClock::new(0));
        let sid_a = SessionId::new("a");
        let sid_b = SessionId::new("b");

        let (fleet, card_a, card_b, view_a, view_b, rc_a, rc_b) = cx.update(|cx| {
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
}
