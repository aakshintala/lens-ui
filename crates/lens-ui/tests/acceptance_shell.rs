use gpui::Action;
use lens_core::actor::{ActorFeed, SessionCommand, SummaryUpdate};
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::usage::Cost;
use lens_core::reduce::StreamUpdate;
use lens_ui::PtyProbe;
use lens_ui::actions::BackToBoard;
use lens_ui::board::BoardView;
use lens_ui::card::model::{READY_DECAY_MS, RepoRef, SessionCard};
use lens_ui::card::wave::{Wave, derive_wave};
use lens_ui::clock::{ManualUiClock, UiClock};
use lens_ui::fleet::store::FleetStore;
use lens_ui::slot::placeholder_tab;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

fn summary(
    status: SessionStatusValue,
    title: &str,
    activity: &str,
    turn: u32,
    workspace: Option<&str>,
) -> SummaryUpdate {
    SummaryUpdate {
        status,
        title: Some(title.into()),
        last_total_tokens: Some(1_000),
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
        workspace: workspace.map(str::to_string),
        reasoning_effort: None,
        activity_summary: activity.into(),
        last_completed_turn: turn,
        harness: Some("claude-native".into()),
    }
}

/// Viewport re-entry freeze repro (handoff 2026-07-17): a Working card that lands
/// OFF-SCREEN in the focus-mode shrunk rail must resume animating after returning to
/// the board. The paint-time `last_bounds` gate (view.rs) drops the anim timer while
/// off-screen and, on a single board re-render reading stale off-screen bounds, never
/// respawns it → frozen spinner/pulse. RED before fix, GREEN after.
#[gpui::test]
async fn card_offscreen_in_focus_rail_resumes_animating_on_return(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 8;
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i}"))).collect();

    let fleet = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        fleet.update(cx, |f, cx| {
            for id in &ids {
                f.spawn_fake_session(id.clone(), cx);
            }
        });
        // All cards Working (Running → Wave::Working → animating tick).
        for id in &ids {
            let card = fleet.read(cx).card(id).unwrap();
            card.update(cx, |c, _| c.status = SessionStatusValue::Running);
        }
        fleet
    });

    let fleet_for_window = fleet.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, None, cx)
    });

    // Wide + short: board grid (horizontal wrap) keeps ALL cards on-screen; the focus
    // rail (280px vertical column, ~152px/card) overflows so bottom cards go off-screen.
    vcx.simulate_resize(Size {
        width: px(3000.0),
        height: px(700.0),
    });
    for id in &ids {
        vcx.update(|_, cx| {
            let card = fleet.read(cx).card(id).unwrap();
            card.update(cx, |_, cx| cx.notify());
        });
    }
    vcx.run_until_parked();

    let top = ids[1].clone(); // on-screen in the rail (healthy control)
    let bottom = ids[N - 1].clone(); // off-screen in the rail (freeze suspect)

    // Capture the per-card render_count handles once (no borrow of cx afterwards).
    let (rc_top, rc_bottom) = vcx.read(|cx| {
        let views = board_handle.read(cx).card_views_for_test();
        (
            views[&top].read(cx).render_count.clone(),
            views[&bottom].read(cx).render_count.clone(),
        )
    });
    let bottom_for_bounds = bottom.clone();
    let top_for_bounds = top.clone();
    let bounds = |vcx: &mut gpui::VisualTestContext, id: SessionId| {
        vcx.read(|cx| board_handle.read(cx).card_bounds_for_test(&id, cx))
    };

    // Sanity: on the board every Working card animates.
    let board_base_top = rc_top.get();
    let board_base_bottom = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_top.get() > board_base_top && rc_bottom.get() > board_base_bottom,
        "sanity: all Working cards animate on the board"
    );

    // Enter focus mode; let the off-screen card's timer fire and self-drop.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.focus_session(ids[0].clone(), cx)));
    vcx.run_until_parked();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    // Precondition: the suspect really did land off-screen in the rail (else the repro is
    // vacuous — it would pass without exercising the freeze path).
    let bottom_rail = bounds(vcx, bottom_for_bounds.clone()).expect("bottom card painted in rail");
    let vp = vcx.update(|window, _| window.viewport_size());
    assert!(
        bottom_rail.origin.y >= vp.height,
        "repro precondition: suspect must be off-screen in the focus rail \
         (y={:?}, viewport height={:?})",
        bottom_rail.origin.y,
        vp.height
    );
    // The viewport gate must actually CULL the off-screen card (driver dropped → no more
    // re-renders). Without this, a fix that simply deleted the gate would pass the recovery
    // assertion while silently reintroducing the off-screen-CPU regression.
    let bottom_settled = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert_eq!(
        rc_bottom.get(),
        bottom_settled,
        "gate must cull the off-screen rail card (render_count frozen while hidden)"
    );

    // Return to the board.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    vcx.run_until_parked();

    let top_after_blur = rc_top.get();
    let bottom_after_blur = rc_bottom.get();

    // Advance the animation clock; a Working card that is animating keeps re-rendering.
    for _ in 0..6 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    let top_delta = rc_top.get() - top_after_blur;
    let bottom_delta = rc_bottom.get() - bottom_after_blur;
    let _ = top_for_bounds;

    assert!(
        top_delta > 0,
        "control: on-screen card must keep animating on the board (delta={top_delta})"
    );
    assert!(
        bottom_delta > 0,
        "FREEZE BUG: card that was off-screen in the focus rail is frozen after return \
         (delta={bottom_delta}) — anim driver never respawned"
    );
}

/// Same freeze, but the board *mounts already focused* (deep-link / session-restore shape).
/// The recovery must not depend on a fleet notification having established the focused mode
/// before the first blur — it tracks the last *rendered* mode, so the first render (focused)
/// records it and the blur recovers. Guards against the observer-only regression.
#[gpui::test]
async fn card_offscreen_resumes_when_board_mounts_focused(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 8;
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i}"))).collect();

    let fleet = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        fleet.update(cx, |f, cx| {
            for id in &ids {
                f.spawn_fake_session(id.clone(), cx);
            }
        });
        for id in &ids {
            let card = fleet.read(cx).card(id).unwrap();
            card.update(cx, |c, _| c.status = SessionStatusValue::Running);
        }
        // Focus BEFORE the board window exists — the board will mount already in focus mode,
        // and this notify predates the board's fleet observer (so it never sees the edge).
        fleet.update(cx, |f, cx| f.focus_session(ids[0].clone(), cx));
        fleet
    });

    let fleet_for_window = fleet.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, None, cx)
    });
    vcx.simulate_resize(Size {
        width: px(3000.0),
        height: px(700.0),
    });
    vcx.run_until_parked();

    let bottom = ids[N - 1].clone();
    let rc_bottom = vcx.read(|cx| {
        board_handle.read(cx).card_views_for_test()[&bottom]
            .read(cx)
            .render_count
            .clone()
    });

    // Settle in focus mode so the off-screen bottom card drops its driver.
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }

    // Return to the board and confirm the previously-off-screen card resumes animating.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    vcx.run_until_parked();
    let after_blur = rc_bottom.get();
    for _ in 0..6 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_bottom.get() > after_blur,
        "mount-focused → blur must recover the off-screen card's animation \
         (delta={})",
        rc_bottom.get() - after_blur
    );
}

#[gpui::test]
async fn shell_skeleton_acceptance(cx: &mut gpui::TestAppContext) {
    let clock = Arc::new(ManualUiClock::new(10_000));
    let a = SessionId::new("a");
    let b = SessionId::new("b");
    let c = SessionId::new("c");

    let (fleet, pty) = cx.update(|cx| {
        // Card chrome reads `cx.lens_theme()` on render (A2 migration), so the theme global must
        // be installed before the board renders.
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        // BackToBoard is now an app-global action (not an element handler); register it so the
        // window.dispatch_action(BackToBoard) below routes to the global handler.
        lens_ui::shortcuts::register(&fleet, cx);
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(a.clone(), cx);
            f.spawn_fake_session(b.clone(), cx);
            f.spawn_fake_session(c.clone(), cx);
        });
        let pty = PtyProbe {
            bytes_sent: Rc::new(Cell::new(0)),
        };
        (fleet, pty)
    });

    let fleet_for_window = fleet.clone();
    let pty_for_window = pty.clone();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        let working_tab = placeholder_tab(cx);
        BoardView::mount(fleet_for_window, working_tab, Some(pty_for_window), cx)
    });

    // (1) Settle first frame — targeted notify only (never cx.refresh()).
    for id in [&a, &b, &c] {
        vcx.update(|_, cx| {
            let card = fleet.read(cx).card(id).unwrap();
            card.update(cx, |_, cx| cx.notify());
        });
    }
    vcx.run_until_parked();

    let (rc_a, rc_b, rc_c, paint_a, paint_b, paint_c) = vcx.read(|cx| {
        let views = board_handle.read(cx).card_views_for_test();
        (
            views[&a].read(cx).render_count.clone(),
            views[&b].read(cx).render_count.clone(),
            views[&c].read(cx).render_count.clone(),
            views[&a].read(cx).paint_count.clone(),
            views[&b].read(cx).paint_count.clone(),
            views[&c].read(cx).paint_count.clone(),
        )
    });
    let a0 = rc_a.get();
    let b0 = rc_b.get();
    let c0 = rc_c.get();
    let c_paint0 = paint_c.get();
    let store0 = vcx.read(|cx| fleet.read(cx).store_notify_count());

    let bounds_c_before = vcx
        .read(|cx| board_handle.read(cx).card_bounds_for_test(&c, cx))
        .expect("C must have canvas-captured bounds after first draw");

    // (2) Observe-isolation: inject Summary on B only.
    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Running,
                "B-live",
                "",
                1,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |_, cx| cx.notify());
    });
    vcx.run_until_parked();

    assert!(rc_b.get() > b0, "B must re-render on its Summary fold");
    assert_eq!(rc_a.get(), a0, "A sibling must not re-render");
    let store1 = vcx.read(|cx| fleet.read(cx).store_notify_count());
    assert_eq!(store1, store0, "FleetStore notify unchanged on scalar fold");

    // (3) Size-invariance: activity present + repos 1→3 on B; C bounds byte-equal, paint unchanged.
    // SummaryUpdate cannot carry N repos yet — drive growth via test hook + activity fold.
    vcx.update(|_, cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card, cx| {
            card.activity_summary = "wiring isolation".into();
            card.set_repos_for_test(vec![
                RepoRef {
                    name: "lens".into(),
                    branch: Some("main".into()),
                },
                RepoRef {
                    name: "omnigent".into(),
                    branch: Some("dev".into()),
                },
                RepoRef {
                    name: "other".into(),
                    branch: None,
                },
            ]);
            cx.notify();
        });
    });
    vcx.run_until_parked();
    let bounds_c_after = vcx
        .read(|cx| board_handle.read(cx).card_bounds_for_test(&c, cx))
        .expect("C bounds must remain captured");
    assert_eq!(
        bounds_c_after, bounds_c_before,
        "C bounds must be byte-equal after B content growth"
    );
    assert_eq!(
        paint_c.get(),
        c_paint0,
        "downstream C must not repaint when B grows inside fixed tile"
    );
    assert_eq!(rc_c.get(), c0, "downstream C render_count unchanged");

    // (4) Mode-switch order-safety on B's unified feed.
    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "stale-summary",
                "",
                2,
                Some("lens"),
            ))),
        );
        fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx));
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Detailed(StreamUpdate::StatusChanged(SessionStatusValue::Running)),
        );
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Detailed(StreamUpdate::TitleChanged(Some("detailed-title".into()))),
        );
    });
    vcx.run_until_parked();
    let title = vcx.read(|cx| fleet.read(cx).card(&b).unwrap().read(cx).title.clone());
    assert_eq!(
        title.as_deref(),
        Some("detailed-title"),
        "must end on Detailed projection, not stale Summary"
    );
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "post-demote",
                "",
                2,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    let title = vcx.read(|cx| fleet.read(cx).card(&b).unwrap().read(cx).title.clone());
    assert_eq!(title.as_deref(), Some("post-demote"));

    // (5) Ready trigger + decay (dual-clock).
    clock.set(20_000);
    vcx.update(|_, cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card: &mut SessionCard, _| {
            card.seeded = true;
            card.seen_turn = 2;
            card.last_completed_at = None;
            card.status = SessionStatusValue::Idle;
        });
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "done",
                "",
                9,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(20_000));
        assert_eq!(
            derive_wave(card, 20_000 + 1_000, false),
            Wave::Ready,
            "monotonic turn jump lights Ready"
        );
        assert_ne!(
            derive_wave(card, 20_000 + 1_000, true),
            Wave::Ready,
            "glow suppressed when focused"
        );
    });

    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Running,
                "busy",
                "working",
                9,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_ne!(derive_wave(card, 21_000, false), Wave::Ready);
    });
    clock.set(22_000);
    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "done2",
                "",
                10,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(derive_wave(card, 22_000 + 1_000, false), Wave::Ready);
    });

    let store_before_decay = vcx.read(|cx| fleet.read(cx).store_notify_count());
    let card_notifies_before =
        vcx.read(|cx| fleet.read(cx).card(&b).unwrap().read(cx).notify_count);
    clock.set(22_000 + READY_DECAY_MS + 1);
    vcx.executor()
        .advance_clock(Duration::from_millis((READY_DECAY_MS + 1) as u64));
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_ne!(
            derive_wave(card, clock.now_millis(), false),
            Wave::Ready,
            "Ready clears after READY_DECAY"
        );
        assert!(card.notify_count > card_notifies_before);
        assert_eq!(
            fleet.read(cx).store_notify_count(),
            store_before_decay,
            "decay timer must not notify FleetStore"
        );
    });

    clock.set(30_000);
    vcx.update(|_, cx| {
        let card = fleet.read(cx).card(&b).unwrap();
        card.update(cx, |card, _| {
            card.last_completed_at = Some(29_000);
            card.seen_turn = 10;
            card.seeded = true;
            card.status = SessionStatusValue::Idle;
        });
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "reconn",
                "",
                10,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(29_000));
        assert_eq!(derive_wave(card, 30_000, false), Wave::Ready);
    });
    vcx.update(|_, cx| {
        fleet.read(cx).fake.as_ref().unwrap().push_feed(
            &b,
            ActorFeed::Summary(Box::new(summary(
                SessionStatusValue::Idle,
                "reconn2",
                "",
                11,
                Some("lens"),
            ))),
        );
    });
    vcx.run_until_parked();
    vcx.read(|cx| {
        let card = fleet.read(cx).card(&b).unwrap().read(cx);
        assert_eq!(card.last_completed_at, Some(30_000));
        assert_eq!(card.seen_turn, 11);
    });

    // (6) Focus doesn't repaint unrelated siblings (intra-focused-mode switch).
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx)));
    vcx.run_until_parked();
    let c_paint_before_focus = paint_c.get();
    let a_paint_before = paint_a.get();
    let b_paint_before = paint_b.get();
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.focus_session(a.clone(), cx)));
    vcx.run_until_parked();
    assert!(
        paint_a.get() > a_paint_before,
        "newly focused card A must repaint on focus transition"
    );
    assert!(
        paint_b.get() > b_paint_before,
        "previously focused card B must repaint on demote-from-focus"
    );
    assert_eq!(
        paint_c.get(),
        c_paint_before_focus,
        "sibling C paint_count unchanged — cache-reused when layout mode is stable"
    );

    // (7) BackToBoard — Demote + zero PTY bytes.
    vcx.update(|window, cx| {
        fleet.update(cx, |f, cx| f.focus_session(b.clone(), cx));
        board_handle.read(cx).focus_working_tab_for_test(window, cx);
        pty.bytes_sent.set(0);
        window.dispatch_action(BackToBoard.boxed_clone(), cx);
    });
    vcx.run_until_parked();
    // WEAK proxy: placeholder tab has no real PTY handler — proves dispatch+Demote, not
    // true routing priority (that lands with the terminal slice).
    assert_eq!(pty.bytes_sent.get(), 0, "⌘. must not send PTY bytes");
    vcx.read(|cx| {
        assert!(fleet.read(cx).focused().is_none());
        let cmds = fleet.read(cx).fake.as_ref().unwrap().take_commands(&b);
        assert!(
            cmds.iter().any(|c| matches!(c, SessionCommand::Demote)),
            "BackToBoard must Demote"
        );
    });
}
