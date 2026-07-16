use gpui::{
    AnyView, Context, Entity, IntoElement, ParentElement, Render, StyleRefinement, Window, div,
    prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;

use super::model::{CARD_HEIGHT_PX, CARD_WIDTH_PX, SessionCard};

/// §4.4: skeleton card chrome uses gpui `div`/text only — no gpui-component widget
/// inside the tile — so cache-key/bounds risk from component internals is N/A.
pub struct SessionCardView {
    card: Entity<SessionCard>,
    pub render_count: Rc<Cell<usize>>,
    pub paint_count: Rc<Cell<usize>>,
}

impl SessionCardView {
    pub fn new(card: Entity<SessionCard>, cx: &mut Context<Self>) -> Self {
        // §4.4: observe ONLY this card entity — never FleetStore or sibling cards.
        cx.observe(&card, |_, _, cx| cx.notify()).detach();
        Self {
            card,
            render_count: Rc::new(Cell::new(0)),
            paint_count: Rc::new(Cell::new(0)),
        }
    }
}

impl Render for SessionCardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count.set(self.render_count.get() + 1);
        let card = self.card.read(cx);
        let title = card.title.clone().unwrap_or_default();
        // Chrome details land in Task 4 — fixed outer bounds are the §4.4 load-bearing bit.
        div()
            .id(("session-card", self.card.entity_id()))
            .w(px(CARD_WIDTH_PX))
            .h(px(CARD_HEIGHT_PX))
            .child(title)
    }
}

/// Mount as `AnyView` inside `.cached(...)` with stable bounds style.
pub fn mount_cached_card(view: Entity<SessionCardView>) -> impl IntoElement {
    let style = StyleRefinement::default();
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
        let sid_a = SessionId::new("a");
        let sid_b = SessionId::new("b");

        let (card_a, card_b, view_a, view_b, rc_a, rc_b) = cx.update(|cx| {
            let card_a = cx.new(|_| SessionCard::new(sid_a.clone()));
            let card_b = cx.new(|_| SessionCard::new(sid_b.clone()));
            let view_a = cx.new(|cx| SessionCardView::new(card_a.clone(), cx));
            let view_b = cx.new(|cx| SessionCardView::new(card_b.clone(), cx));
            let rc_a = view_a.read(cx).render_count.clone();
            let rc_b = view_b.read(cx).render_count.clone();
            (card_a, card_b, view_a, view_b, rc_a, rc_b)
        });

        let (_board, vcx) = cx.add_window_view(|_, _| DualCardBoard {
            view_a: view_a.clone(),
            view_b: view_b.clone(),
        });

        vcx.run_until_parked();

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
