use gpui::Action;
use gpui::AppContext;
use lens_core::actor::{ActorFeed, SessionCommand, SummaryUpdate};
use lens_core::domain::ids::SessionId;
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::usage::Cost;
use lens_core::reduce::StreamUpdate;
use lens_ui::PtyProbe;
use lens_ui::actions::BackToBoard;
use lens_ui::board::{BoardReplica, BoardView};
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

    const N: usize = 12; // enough rail cards to overflow a short window at 1 col
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i:02}"))).collect();

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
            card.update(cx, |c, _| c.status = SessionStatusValue::Running); // Working → animates
        }
        fleet
    });

    let fleet_for_window = fleet.clone();
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx)));
    cx.run_until_parked();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            None,
            cx,
        )
    });
    // Wide + short: board (multi-col) keeps all cards on-screen; the 1-col focus
    // rail overflows so the bottom cards cull.
    vcx.simulate_resize(Size {
        width: px(3000.0),
        height: px(700.0),
    });
    vcx.run_until_parked();

    let top = ids[0].clone(); // on-screen control
    let bottom = ids[N - 1].clone(); // off-screen in the rail

    let (rc_top, rc_bottom) = vcx.read(|cx| {
        let views = board_handle.read(cx).card_views_for_test();
        (
            views[&top].read(cx).render_count.clone(),
            views[&bottom].read(cx).render_count.clone(),
        )
    });

    // Sanity: on the wide board every Working card is visible and animating.
    let base_top = rc_top.get();
    let base_bottom = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_top.get() > base_top && rc_bottom.get() > base_bottom,
        "sanity: all Working cards animate on the board"
    );

    // Enter focus mode; the bottom rail card culls → hidden → timer drops.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.focus_session(ids[0].clone(), cx)));
    vcx.run_until_parked();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    let (bottom_visible, bottom_timer) = vcx.read(|cx| {
        let v = &board_handle.read(cx).card_views_for_test()[&bottom];
        (v.read(cx).is_visible(), v.read(cx).timer_running_for_test())
    });
    assert!(!bottom_visible, "off-screen rail card must be gated hidden");
    assert!(!bottom_timer, "hidden card's anim timer must be dropped");
    let (top_visible, top_timer) = vcx.read(|cx| {
        let v = &board_handle.read(cx).card_views_for_test()[&top];
        (v.read(cx).is_visible(), v.read(cx).timer_running_for_test())
    });
    assert!(
        top_visible,
        "on-screen rail control must stay visible in focus mode (gate must not blanket-hide)"
    );
    assert!(
        top_timer,
        "on-screen rail control must keep animating in focus mode"
    );
    let settled = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert_eq!(
        rc_bottom.get(),
        settled,
        "culled card must not re-render (no off-screen CPU)"
    );

    // Return to the board — bottom card re-enters the visible band → timer respawns.
    vcx.update(|_, cx| fleet.update(cx, |f, cx| f.blur_to_board(cx)));
    vcx.run_until_parked();
    let after_blur = rc_bottom.get();
    for _ in 0..6 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert!(
        rc_bottom.get() > after_blur,
        "FREEZE BUG: card off-screen in the rail is frozen after return — timer never respawned"
    );
}

/// Same freeze, but the board *mounts already focused* (deep-link / session-restore shape).
#[gpui::test]
async fn card_offscreen_resumes_when_board_mounts_focused(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 12; // enough rail cards to overflow a short window at 1 col
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i:02}"))).collect();

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
        // Focus BEFORE the board window exists — mounts already in focus mode.
        fleet.update(cx, |f, cx| f.focus_session(ids[0].clone(), cx));
        fleet
    });

    let fleet_for_window = fleet.clone();
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx)));
    cx.run_until_parked();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            None,
            cx,
        )
    });
    vcx.simulate_resize(Size {
        width: px(3000.0),
        height: px(700.0),
    });
    vcx.run_until_parked();

    let top = ids[0].clone(); // on-screen control
    let bottom = ids[N - 1].clone();
    let rc_bottom = vcx.read(|cx| {
        board_handle.read(cx).card_views_for_test()[&bottom]
            .read(cx)
            .render_count
            .clone()
    });

    // Settle in focus mode; bottom rail card culls → hidden → timer drops.
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    let (bottom_visible, bottom_timer) = vcx.read(|cx| {
        let v = &board_handle.read(cx).card_views_for_test()[&bottom];
        (v.read(cx).is_visible(), v.read(cx).timer_running_for_test())
    });
    assert!(!bottom_visible, "off-screen rail card must be gated hidden");
    assert!(!bottom_timer, "hidden card's anim timer must be dropped");
    let (top_visible, top_timer) = vcx.read(|cx| {
        let v = &board_handle.read(cx).card_views_for_test()[&top];
        (v.read(cx).is_visible(), v.read(cx).timer_running_for_test())
    });
    assert!(
        top_visible,
        "on-screen rail control must stay visible in focus mode (gate must not blanket-hide)"
    );
    assert!(
        top_timer,
        "on-screen rail control must keep animating in focus mode"
    );
    let settled = rc_bottom.get();
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(50));
        vcx.run_until_parked();
    }
    assert_eq!(
        rc_bottom.get(),
        settled,
        "culled card must not re-render (no off-screen CPU)"
    );

    // Return to the board — bottom card re-enters the visible band → timer respawns.
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
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx)));
    cx.run_until_parked();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            Some(pty_for_window),
            cx,
        )
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

/// Culling (spec §4 unknown 2): on a board tall enough to overflow, tiles below
/// the visible band + overdraw are NOT built (absent from the child vec).
#[gpui::test]
async fn board_culls_offscreen_tiles(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};

    const N: usize = 40; // 40 loose cards @ 200px cell / ~3 cols ⇒ ~14 rows ⇒ tall
    let clock = Arc::new(ManualUiClock::new(10_000));
    let ids: Vec<SessionId> = (0..N).map(|i| SessionId::new(format!("s{i:02}"))).collect();

    let fleet = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        fleet.update(cx, |f, cx| {
            for id in &ids {
                f.spawn_fake_session(id.clone(), cx);
            }
        });
        fleet
    });

    let fleet_for_window = fleet.clone();
    let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet_for_window.clone(), cx)));
    cx.run_until_parked();
    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            None,
            cx,
        )
    });
    // Normal-size window: only the top few rows fit; the rest overflow the band.
    vcx.simulate_resize(Size {
        width: px(1000.0),
        height: px(700.0),
    });
    vcx.run_until_parked();

    let built = vcx.read(|cx| board_handle.read(cx).visible_session_ids_for_test());
    assert!(!built.is_empty(), "some tiles must be built");
    assert!(
        built.len() < N,
        "off-screen tiles must be culled (built {} of {N})",
        built.len()
    );
    // The last card (bottom row) is far below the band → not built.
    assert!(
        !built.contains(&ids[N - 1]),
        "bottom card must be culled while scrolled to top"
    );
}

// B-4a migration of the retired B-3 `board_group_renders_chrome_and_rollup`: the group now
// comes from a persisted store loaded through the real BoardReplica (not the test_layout
// seam). Members are seeded under the group with the replica's conn so reconcile treats
// them as already-placed. Proves the store→replica→group-render path end to end.
#[gpui::test]
async fn board_group_renders_chrome_and_rollup_via_replica(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};
    use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
    use lens_core::domain::ids::{BoardId, ConnectionId};
    use lens_core::persist::{BoardStore, SqliteBoardStore};

    let clock = Arc::new(ManualUiClock::new(7_200_000)); // 2h past epoch → oldest member ages to "2h"
    let s1 = SessionId::new("s1");
    let s2 = SessionId::new("s2");
    let conn = ConnectionId::new("c");

    let (fleet, c1, c2) = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        let (c1, c2) = fleet.update(cx, |f, cx| {
            let c1 = f.spawn_fake_session(s1.clone(), cx);
            let c2 = f.spawn_fake_session(s2.clone(), cx);
            (c1, c2)
        });
        (fleet, c1, c2)
    });
    // Member card data: s1 = $1.50 @ 0s (oldest), s2 = $2.00 @ 100s.
    cx.update(|cx| {
        c1.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(1.50);
            card.created_at = Some(0);
        });
        c2.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(2.00);
            card.created_at = Some(100);
        });
    });

    // Seed the "Refactor" (blue) group + members into a REAL store, then load via BoardReplica.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let store = SqliteBoardStore::open(&path).unwrap();
    let board = BoardId::new(DEFAULT_BOARD_ID);
    let g1 = store.create_group(&board, None, 0, "Refactor").unwrap();
    store.set_color(&g1, "blue").unwrap();
    for (i, s) in [&s1, &s2].into_iter().enumerate() {
        store
            .place_session(
                &conn,
                s,
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g1.clone()),
                    ordinal: Some(i as i32),
                },
            )
            .unwrap();
    }
    let boxed: Box<dyn BoardStore + Send> = Box::new(store);

    let fleet_for_window = fleet.clone();
    let replica = cx.update(|cx| {
        cx.new(|cx| {
            BoardReplica::new(
                Some(boxed),
                path.clone(),
                conn.clone(),
                fleet_for_window.clone(),
                cx,
            )
        })
    });
    cx.run_until_parked(); // load the seeded group + members

    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            None,
            cx,
        )
    });
    vcx.simulate_resize(Size {
        width: px(1200.0),
        height: px(900.0),
    });
    vcx.run_until_parked();

    let chrome = vcx.read(|cx| board_handle.read(cx).group_chrome_for_test());
    assert_eq!(
        chrome.len(),
        1,
        "exactly one group tile rendered via the store→replica path"
    );
    let g = &chrome[0];
    assert_eq!(g.name, "Refactor");
    assert_eq!(
        g.accent,
        gpui::rgb(0x4c8dff).into(),
        "blue token → SSOT blue"
    );
    assert_eq!(g.rollup.spend_usd, Some(3.50), "spend sums members");
    assert_eq!(g.rollup.oldest_created_at, Some(0), "oldest member wins");
    assert_eq!(g.rollup.completed_count, 0, "✓N is 0 until B-6");
    assert_eq!(g.header, "Refactor · ~$3.50 · 2h · ✓0");
    assert_eq!(g.session_ids, vec![s1.clone(), s2.clone()]);

    let built = vcx.read(|cx| board_handle.read(cx).visible_session_ids_for_test());
    assert!(built.contains(&s1) && built.contains(&s2), "members built");
    drop(dir);
}

// I5 (design §7 freshness): after mount, a member's cost change must refresh the group
// rollup. Also proves the I2 narrow projection reads LIVE member data (not a stale snapshot)
// and that the member-notify → board re-render dependency is intact.
#[gpui::test]
async fn group_rollup_refreshes_on_member_cost_change(cx: &mut gpui::TestAppContext) {
    use gpui::{Size, px};
    use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
    use lens_core::domain::ids::{BoardId, ConnectionId};
    use lens_core::persist::{BoardStore, SqliteBoardStore};

    let clock = Arc::new(ManualUiClock::new(7_200_000));
    let s1 = SessionId::new("s1");
    let s2 = SessionId::new("s2");
    let conn = ConnectionId::new("c");

    let (fleet, c1, _c2) = cx.update(|cx| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);
        let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
        let (c1, c2) = fleet.update(cx, |f, cx| {
            (
                f.spawn_fake_session(s1.clone(), cx),
                f.spawn_fake_session(s2.clone(), cx),
            )
        });
        (fleet, c1, c2)
    });
    cx.update(|cx| {
        c1.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(1.50)
        });
        _c2.update(cx, |card, _| {
            card.cumulative_cost.total_cost_usd = Some(2.00)
        });
    });

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let store = SqliteBoardStore::open(&path).unwrap();
    let board = BoardId::new(DEFAULT_BOARD_ID);
    let g1 = store.create_group(&board, None, 0, "G").unwrap();
    for (i, s) in [&s1, &s2].into_iter().enumerate() {
        store
            .place_session(
                &conn,
                s,
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g1.clone()),
                    ordinal: Some(i as i32),
                },
            )
            .unwrap();
    }
    let boxed: Box<dyn BoardStore + Send> = Box::new(store);

    let fleet_for_window = fleet.clone();
    let replica = cx.update(|cx| {
        cx.new(|cx| {
            BoardReplica::new(
                Some(boxed),
                path.clone(),
                conn.clone(),
                fleet_for_window.clone(),
                cx,
            )
        })
    });
    cx.run_until_parked();

    let (board_handle, vcx) = cx.add_window_view(|_, cx| {
        BoardView::mount(
            fleet_for_window,
            replica.clone(),
            placeholder_tab(cx),
            None,
            cx,
        )
    });
    vcx.simulate_resize(Size {
        width: px(1200.0),
        height: px(900.0),
    });
    vcx.run_until_parked();

    // Initial rollup = $1.50 + $2.00.
    let before = vcx.read(|cx| board_handle.read(cx).group_chrome_for_test());
    assert_eq!(before[0].rollup.spend_usd, Some(3.50), "initial rollup");

    // Change a member's cost AFTER mount + notify → board (which reads the member in render)
    // must re-render and re-fold a FRESH rollup.
    vcx.update(|_, cx| {
        c1.update(cx, |card, cx| {
            card.cumulative_cost.total_cost_usd = Some(5.00);
            cx.notify();
        });
    });
    vcx.run_until_parked();

    let after = vcx.read(|cx| board_handle.read(cx).group_chrome_for_test());
    assert_eq!(
        after[0].rollup.spend_usd,
        Some(7.00),
        "rollup refreshed after member cost change (5.00 + 2.00)"
    );
    drop(dir);
}
