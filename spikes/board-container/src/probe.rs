//! Automated GO/NO-GO harness. Drives scroll programmatically against the live
//! window and asserts the three behavioural unknowns. Prints a verdict table
//! and quits with a non-zero-ish signal in the text (spikes aren't gated, so the
//! human reads the table). Run with `cargo run -- --probe`.

use std::sync::mpsc;
use std::time::Duration;

use gpui::{AsyncWindowContext, Point, WeakEntity, px};

use crate::container::Container;

/// Copy of the sibling spike's frame-advance helper: gpui 0.2.2 has no
/// `await next_frame()`, so schedule `on_next_frame` + `window.refresh()`.
async fn wait_frames(wcx: &mut AsyncWindowContext, n: usize) {
    for _ in 0..n {
        let (tx, rx) = mpsc::sync_channel(1);
        wcx.on_next_frame(move |_, _| {
            let _ = tx.send(());
        });
        let _ = wcx.update(|window, _| window.refresh());
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            wcx.background_executor()
                .timer(Duration::from_millis(1))
                .await;
        }
    }
}

/// Let real wall-time pass so the 50ms anim timers can accumulate ticks, while
/// keeping the window painting.
async fn dwell(wcx: &mut AsyncWindowContext, ms: u64) {
    let frames = (ms / 16).max(1) as usize;
    wait_frames(wcx, frames).await;
    wcx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
    wait_frames(wcx, 2).await;
}

fn set_scroll(weak: &WeakEntity<Container>, wcx: &mut AsyncWindowContext, scroll_top: f32) {
    let _ = weak.update_in(wcx, |c, window, _| {
        c.scroll_handle().set_offset(Point {
            x: px(0.0),
            y: px(-scroll_top),
        });
        window.refresh();
    });
}

fn tick_of(weak: &WeakEntity<Container>, wcx: &mut AsyncWindowContext, id: usize) -> u64 {
    weak.update_in(wcx, |c, _, cx| c.card(id).read(cx).tick_count)
        .unwrap_or(0)
}

pub async fn run_probe(weak: WeakEntity<Container>, mut wcx: AsyncWindowContext) {
    let mut results: Vec<(bool, String)> = Vec::new();
    let mut push = |ok: bool, msg: String| results.push((ok, msg));

    // Settle the first frame(s): cull + gate apply.
    wait_frames(&mut wcx, 5).await;

    // --- geometry + a top/bottom card id we can watch ---
    let (total, ncards, content_h, viewport_h) = weak
        .update_in(&mut wcx, |c, _, _| {
            (
                c.total_tiles(),
                c.card_count(),
                c.content_height(),
                c.viewport_h(),
            )
        })
        .unwrap();
    let scroll_bottom = (content_h - viewport_h).max(0.0);
    // Card #1 is the first animating card near the top (id 0 is static: 0%5==0).
    let probe_id = 1usize;
    let last_id = ncards - 1;

    // --- Unknown 2a: culling builds only the visible band ---
    let (built, r_first, r_last) = weak
        .update_in(&mut wcx, |c, _, cx| {
            (
                c.built_tiles(),
                c.card(0).read(cx).render_count,
                c.card(last_id).read(cx).render_count,
            )
        })
        .unwrap();
    push(
        built > 0 && built < total,
        format!("CULL: built {built}/{total} tiles at top (want 0 < built < total)"),
    );
    // --- Unknown 2b: an off-screen card is never built by gpui ---
    push(
        r_last == 0 && r_first > 0,
        format!("CULL-BUILD: top card paints={r_first} (>0), bottom card paints={r_last} (want 0)"),
    );

    // --- Unknown 3a: a visible animating card actually ticks ---
    let a0 = tick_of(&weak, &mut wcx, probe_id);
    dwell(&mut wcx, 350).await;
    let a1 = tick_of(&weak, &mut wcx, probe_id);
    push(
        a1 > a0,
        format!("VISIBLE-ANIMATES: card#{probe_id} tick {a0}→{a1} while on-screen (want rising)"),
    );

    // --- Unknown 3b: scroll it off-screen → timer stops, ticks freeze ---
    set_scroll(&weak, &mut wcx, scroll_bottom);
    wait_frames(&mut wcx, 5).await; // let cull + defer(set_visible(false)) apply
    let (running, vis) = weak
        .update_in(&mut wcx, |c, _, cx| {
            let card = c.card(probe_id);
            let card = card.read(cx);
            (card.timer_running(), card.is_visible())
        })
        .unwrap();
    let f0 = tick_of(&weak, &mut wcx, probe_id);
    dwell(&mut wcx, 350).await;
    let f1 = tick_of(&weak, &mut wcx, probe_id);
    push(
        f1 == f0 && !running && !vis,
        format!(
            "HIDDEN-FREEZES: card#{probe_id} off-screen tick {f0}→{f1} (want equal), running={running} visible={vis} (want false/false)"
        ),
    );

    // --- Unknown 3c: scroll back → timer respawns, ticks resume (THE freeze fix) ---
    set_scroll(&weak, &mut wcx, 0.0);
    wait_frames(&mut wcx, 5).await;
    let g0 = tick_of(&weak, &mut wcx, probe_id);
    dwell(&mut wcx, 350).await;
    let g1 = tick_of(&weak, &mut wcx, probe_id);
    let reran = weak
        .update_in(&mut wcx, |c, _, cx| c.card(probe_id).read(cx).timer_running())
        .unwrap_or(false);
    push(
        g1 > g0 && reran,
        format!(
            "RESUME-ON-REENTRY (freeze fix): card#{probe_id} back on-screen tick {g0}→{g1} (want rising), running={reran}"
        ),
    );

    // --- report ---
    let all_ok = results.iter().all(|(ok, _)| *ok);
    eprintln!("\n================ BOARD CONTAINER SPIKE — PROBE ================");
    for (ok, msg) in &results {
        eprintln!("  [{}] {}", if *ok { "PASS" } else { "FAIL" }, msg);
    }
    eprintln!(
        "  VERDICT: {}",
        if all_ok {
            "GO — absolute-positioned masonry + container-driven cull/gate works"
        } else {
            "NO-GO — see failures above"
        }
    );
    eprintln!("==============================================================\n");

    let _ = wcx.update(|_, app| app.quit());
}
