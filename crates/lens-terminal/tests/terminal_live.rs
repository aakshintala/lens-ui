//! Live terminal vertical rider (Slice 1d Task 9): `open()` → attach → input →
//! real-window paint → forced network loss → reconnect with `output_gap`.
//!
//! **Slice 2b (Task 5):** optional paste round-trip leg when
//! `LENS_LIVE_CLIPBOARD_PASTE=1` — sets the clipboard, dispatches the production
//! paste path (same `handle_paste` as Cmd+V intercept), asserts the pasted text
//! appears in the frame. OSC-52 program→clipboard has no hermetic live counterpart;
//! observe manually with `LENS_DEMO_ALLOW_CLIPBOARD=1` on `lens-terminal-demo`.
//!
//! **Not** under `#[gpui::test]`: gpui's test platform installs a `NoopTextSystem`
//! that fakes every font/shape/paint result, so paint assertions there are
//! false-green. This is a `harness = false` binary that opens a **real**
//! `Application::new().run()` window hosting a production [`lens_terminal::TerminalTab`].
//!
//! # Environment (read before any GPUI call)
//!
//! | Variable | Required | Role |
//! | --- | --- | --- |
//! | `LENS_OMNIGENT_URL` | yes | Omnigent base URL |
//! | `LENS_OMNIGENT_SESSION_ID` | yes | Session / conversation id |
//! | `LENS_OMNIGENT_TERMINAL_ID` | target A | Attach to existing terminal |
//! | `LENS_OMNIGENT_TERMINAL_NAME` + `LENS_OMNIGENT_SESSION_KEY` | target B | Open-or-create |
//! | `LENS_LIVE_CLIPBOARD_PASTE` | no | Set to `1` to run P5 paste round-trip |
//! | `LENS_LIVE_MOUSE_REPORT` | no | Set to `1` to run P6 mouse-report round-trip (Slice 2c) |
//! | `LENS_LIVE_SLEEP_WAKE` | no | Set to `1` to run P7 Sleep→Wake round-trip (Slice 4) |
//! | `LENS_LIVE_REATTACH` | no | Set to `1` to run P8 ClientDetached→Reattach (Slice 4) |
//! | `LENS_LIVE_TRANSFER` | no | Set to `1` to run P9 cross-session Transfer (Slice 5-A) |
//! | `LENS_LIVE_4404_FIRST` | no | Set to `1` to run P10 4404-first delete+create adopt (Slice 5-A) |
//!
//! **Slice 2c (Task 8):** optional P6 leg when `LENS_LIVE_MOUSE_REPORT=1` — enables
//! DEC mouse tracking (`?1000h?1006h`) via a shell `printf`, runs `cat -v` so stdin
//! echoes visibly, simulates a Left press through the engine (encode → egress → PTY),
//! and asserts the SGR report (`^[[<0;…`) round-trips back into the frame.
//!
//! **Slice 4 (Task 6):** optional P7 when `LENS_LIVE_SLEEP_WAKE=1` — deliberate
//! `Sleep` (engine released, `Sleeping`), then `Wake` (re-attaches to `Live` with a
//! fresh redraw). Optional P8 when `LENS_LIVE_REATTACH=1` — opens a competing attach
//! to provoke a source-derived `4405`/`ClientDetached`, then `Reattach` → `Live`.
//! Generation-guard **adoption** (`ReplacementWaiting` → successor) has no cheap live
//! trigger (needs a real agent switch / `reset-state`); it stays deterministic-test +
//! demo-covered.
//!
//! **Slice 5-A (rider):** the two branches whose *live* half nothing else reaches.
//!
//! - **P9** (`LENS_LIVE_TRANSFER=1`) — **cross-session Transfer.** Drives the
//!   server-side half of a real `/clear` rotation over REST (create a sibling session
//!   on the same runner, then `…/transfer` the live terminal into it; memory
//!   `omnigent-clear-rotation-wire-contract`). Asserts, in order: the attach **survives
//!   the transfer untouched** (a transfer is attach-transparent — it rebinds the
//!   resource's owning session and closes nothing, which is why the host's forwarded
//!   `resource.deleted` is the sole trigger); then that `resource.deleted` +
//!   `Transfer{new_session}` return the tab to `Live` **against the new session** with
//!   `output_gap` set and the pre-rotation marker still on screen. The in-crate test can
//!   only prove the reuse branch is *entered* (its offline attach then fails), so this
//!   is the only proof that cross-session engine reuse lands and keeps its scrollback.
//!   Leaves the terminal under a **new** session — `LENS_OMNIGENT_SESSION_ID` is stale
//!   afterwards; the driver tracks the current session across P9 → P10.
//! - **P10** (`LENS_LIVE_4404_FIRST=1`) — **`4404` before the host signals.** Deletes the
//!   terminal resource outright (the agent-switch shape, and the only cheap live source
//!   of a `4404` — a transfer produces none) and forwards **nothing**, asserting the tab
//!   parks in `ReplacementWaiting` holding its frozen engine on the close code alone.
//!   Then relaunches the same key and forwards the late `resource.deleted` +
//!   `resource.created`, which must still drive `adopt()` to `Live`. Same-session adopt
//!   is a fresh attach by design, so scrollback is *not* expected to survive.
//!   Requires an `OpenOrCreate` target: `Existing` has no successor semantics.
//!
//! P9 creates real sessions — run these against a scratch rider agent, not a session
//! you care about.
//!
//! # Skip vs fail
//!
//! - **Skip (exit 0):** `LENS_OMNIGENT_URL` or `LENS_OMNIGENT_SESSION_ID` absent;
//!   or URL+session present but no valid terminal target.
//! - **Fail (exit 1):** env configured but client handshake or any driver phase times out.
//! - **Pass (exit 0):** phases P1–P4 complete; P5–P10 run only when their env flag is `1`.
//!
//! Run manually against omnigent 0.5.1 — **not** part of `cargo test --workspace`:
//!
//! ```text
//! LENS_OMNIGENT_URL=http://127.0.0.1:<port> \
//! LENS_OMNIGENT_SESSION_ID=<conv_…> \
//! LENS_OMNIGENT_TERMINAL_NAME=shell LENS_OMNIGENT_SESSION_KEY=main \
//! cargo test -p lens-terminal --features live-tests,test-util --test terminal_live -- --nocapture
//! ```
#![cfg(feature = "live-tests")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, Application, Bounds, ClipboardItem, Entity, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};
use lens_client::ids::{ConnectionId, SessionId, TerminalId};
use lens_client::{AttachOptions, Auth, Client, Connection, TerminalCreate, attach};
use lens_terminal::{
    DetachedDetail, Frame, Lifecycle, TerminalHostEvent, TerminalKey, TerminalOpenOptions,
    TerminalTab, TerminalTarget, open,
};
use serde_json::Value;

const OVERALL_DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

struct LiveConfig {
    base_url: String,
    target: TerminalTarget,
}

fn main() {
    let config = match load_config() {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!("terminal_live: not configured, skipping");
            std::process::exit(0);
        }
        Err(msg) => {
            eprintln!("terminal_live: {msg}");
            std::process::exit(1);
        }
    };

    let client = match build_client(&config.base_url) {
        Ok(client) => Arc::new(client),
        Err(err) => {
            eprintln!("terminal_live: {err}");
            std::process::exit(1);
        }
    };

    let target = config.target;
    let base_url = config.base_url;
    let options = TerminalOpenOptions::default();
    let marker = format!("LENSMARK_{}", std::process::id());

    Application::new().run(move |cx: &mut App| {
        match cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal terminal_live".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(800.0), px(600.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |_window, cx| {
                let tab = open(target.clone(), Arc::clone(&client), options, cx);
                tab.update(cx, |tab, _| tab.set_render_inspect_enabled(true));
                spawn_driver(
                    tab.clone(),
                    marker.clone(),
                    target,
                    client,
                    base_url.clone(),
                    cx,
                );
                tab
            },
        ) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("terminal_live FAIL: open_window: {e}");
                std::process::exit(1);
            }
        }
        cx.activate(true);
    });
}

fn load_config() -> Result<Option<LiveConfig>, String> {
    let base_url_str = match std::env::var("LENS_OMNIGENT_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(s) => s,
        None => return Ok(None),
    };

    let session_id_str = match std::env::var("LENS_OMNIGENT_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(s) => s,
        None => return Ok(None),
    };

    let session_id = SessionId::new(session_id_str);

    let terminal_id = std::env::var("LENS_OMNIGENT_TERMINAL_ID")
        .ok()
        .filter(|s| !s.is_empty());
    let terminal_name = std::env::var("LENS_OMNIGENT_TERMINAL_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let session_key = std::env::var("LENS_OMNIGENT_SESSION_KEY")
        .ok()
        .filter(|s| !s.is_empty());

    let target = if let Some(tid) = terminal_id {
        TerminalTarget::Existing {
            session_id,
            terminal_id: TerminalId::new(tid),
        }
    } else if let (Some(terminal_name), Some(session_key)) = (terminal_name, session_key) {
        TerminalTarget::OpenOrCreate {
            session_id,
            key: TerminalKey {
                terminal_name,
                session_key,
            },
        }
    } else {
        eprintln!("terminal_live: no valid terminal target, skipping");
        std::process::exit(0);
    };

    Ok(Some(LiveConfig {
        base_url: base_url_str,
        target,
    }))
}

fn build_client(base_url_str: &str) -> Result<Client, String> {
    let base_url: url::Url = base_url_str
        .parse()
        .map_err(|e| format!("invalid LENS_OMNIGENT_URL `{base_url_str}`: {e}"))?;
    let auth = match std::env::var("LENS_OMNIGENT_TOKEN") {
        Ok(token) if !token.is_empty() => Auth::Bearer { token },
        _ => Auth::None,
    };
    Client::new(Connection::new(
        ConnectionId::new("terminal-live"),
        base_url,
        auth,
    ))
    .map_err(|e| format!("handshake failed: {e}"))
}

fn fail_phase(phase: &str) -> ! {
    eprintln!("terminal_live FAIL: {phase}");
    std::process::exit(1);
}

fn frame_contains_marker(frame: &Frame, marker: &str) -> bool {
    for row in &frame.grid {
        let text: String = row.cells.iter().map(|c| c.grapheme.as_str()).collect();
        if text.contains(marker) {
            return true;
        }
    }
    false
}

fn spawn_driver(
    tab: Entity<TerminalTab>,
    marker: String,
    target: TerminalTarget,
    client: Arc<Client>,
    base_url: String,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let weak = tab.downgrade();
        let deadline = Instant::now() + OVERALL_DEADLINE;

        // P1 — attach: poll until Live.
        while Instant::now() < deadline {
            let live = weak
                .update(cx, |tab, _| tab.presentation().lifecycle == Lifecycle::Live)
                .unwrap_or(false);
            if live {
                break;
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }
        if weak
            .update(cx, |tab, _| tab.presentation().lifecycle != Lifecycle::Live)
            .unwrap_or(true)
        {
            fail_phase("P1 attach: lifecycle did not reach Live");
        }

        // P2 — input + paint: echo marker and prove a new paint landed.
        let sent = weak
            .update(cx, |tab, _| {
                tab.debug_send_input_for_test(format!("echo {marker}\r").into_bytes())
            })
            .unwrap_or(false);
        if !sent {
            fail_phase("P2 input+paint: debug_send_input_for_test failed");
        }

        let mut paints_at_marker = None;
        let mut p2_ok = false;
        while Instant::now() < deadline {
            let (has_marker, paints) = weak
                .update(cx, |tab, _| {
                    let paints = tab.render_inspect().frames_painted;
                    let has_marker = tab
                        .debug_latest_frame_for_test()
                        .is_some_and(|f| frame_contains_marker(&f, &marker));
                    (has_marker, paints)
                })
                .unwrap_or((false, 0));

            if has_marker && paints_at_marker.is_none() {
                paints_at_marker = Some(paints);
            }
            if let Some(at_marker) = paints_at_marker
                && paints > at_marker
            {
                p2_ok = true;
                break;
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }
        if !p2_ok {
            fail_phase("P2 input+paint: marker not painted");
        }

        // P3 — network loss: abort attach; poll until Reconnecting.
        let _ = weak.update(cx, |tab, cx| tab.debug_abort_attach_for_test(cx));
        let mut p3_ok = false;
        while Instant::now() < deadline {
            let reconnecting = weak
                .update(cx, |tab, _| {
                    tab.presentation().lifecycle == Lifecycle::Reconnecting
                })
                .unwrap_or(false);
            if reconnecting {
                p3_ok = true;
                break;
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }
        if !p3_ok {
            fail_phase("P3 network loss: lifecycle did not reach Reconnecting");
        }

        // P4 — reattach: Live again with output_gap set.
        let mut p4_ok = false;
        while Instant::now() < deadline {
            let ok = weak
                .update(cx, |tab, _| {
                    let p = tab.presentation();
                    p.lifecycle == Lifecycle::Live && p.output_gap
                })
                .unwrap_or(false);
            if ok {
                p4_ok = true;
                break;
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }
        if !p4_ok {
            fail_phase("P4 reattach: lifecycle not Live with output_gap");
        }

        if live_clipboard_paste_enabled() {
            let paste_marker = format!("PASTE_{}", std::process::id());
            let dispatched = weak
                .update(cx, |tab, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(paste_marker.clone()));
                    tab.debug_paste_for_test(cx);
                    true
                })
                .unwrap_or(false);
            if !dispatched {
                fail_phase("P5 paste round-trip: paste dispatch failed");
            }

            let mut p5_ok = false;
            while Instant::now() < deadline {
                let visible = weak
                    .update(cx, |tab, _| {
                        tab.debug_latest_frame_for_test()
                            .is_some_and(|f| frame_contains_marker(&f, &paste_marker))
                    })
                    .unwrap_or(false);
                if visible {
                    p5_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p5_ok {
                fail_phase("P5 paste round-trip: pasted text not visible in frame");
            }
            eprintln!("terminal_live: P5 paste round-trip OK");
        }

        if live_mouse_report_enabled() {
            // Enable mouse tracking (as terminal OUTPUT) then run `cat -v` so the shell
            // echoes subsequent stdin VISIBLY (ESC shown as `^[`), including the SGR mouse
            // report the engine encodes and sends when we simulate a press.
            let started = weak
                .update(cx, |tab, _| {
                    tab.debug_send_input_for_test(
                        b"printf '\\033[?1000h\\033[?1006h'; cat -v\r".to_vec(),
                    )
                })
                .unwrap_or(false);
            if !started {
                fail_phase("P6 mouse report: enable-tracking/cat dispatch failed");
            }
            // Settle: let the printf enable tracking on the engine terminal and cat start.
            let settle = Instant::now() + Duration::from_secs(2);
            while Instant::now() < settle {
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            // Simulate a Left press; engine encodes SGR -> egress -> PTY -> cat -v echoes.
            let pressed = weak
                .update(cx, |tab, _| {
                    tab.debug_mouse_down_at_cell_for_test(2, 0, 20.0, 4.0)
                })
                .unwrap_or(false);
            if !pressed {
                fail_phase("P6 mouse report: mouse-down dispatch failed");
            }
            // cat -v renders the report's ESC as `^[`, so the frame shows `[<0;...`.
            let p6_deadline = Instant::now() + OVERALL_DEADLINE;
            let mut p6_ok = false;
            while Instant::now() < p6_deadline {
                let visible = weak
                    .update(cx, |tab, _| {
                        tab.debug_latest_frame_for_test()
                            .is_some_and(|f| frame_contains_marker(&f, "[<0;"))
                    })
                    .unwrap_or(false);
                if visible {
                    p6_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p6_ok {
                fail_phase("P6 mouse report: SGR report not echoed back in frame");
            }
            eprintln!("terminal_live: P6 mouse report round-trip OK");
        }

        if live_sleep_wake_enabled() {
            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(TerminalHostEvent::Sleep, cx);
            });
            let mut p7_sleep_ok = false;
            while Instant::now() < deadline {
                let ok = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        let snap = tab.inspect();
                        p.lifecycle == Lifecycle::Sleeping
                            && snap.engine.is_none()
                            && !snap.bridge_alive
                    })
                    .unwrap_or(false);
                if ok {
                    p7_sleep_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p7_sleep_ok {
                fail_phase("P7 sleep: lifecycle not Sleeping with engine released");
            }

            let paints_before_wake = weak
                .update(cx, |tab, _| tab.render_inspect().frames_painted)
                .unwrap_or(0);
            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(TerminalHostEvent::Wake, cx);
            });
            let mut p7_wake_ok = false;
            while Instant::now() < deadline {
                let ok = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        let paints = tab.render_inspect().frames_painted;
                        p.lifecycle == Lifecycle::Live && paints > paints_before_wake
                    })
                    .unwrap_or(false);
                if ok {
                    p7_wake_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p7_wake_ok {
                fail_phase("P7 wake: lifecycle not Live with fresh redraw");
            }
            eprintln!("terminal_live: P7 sleep→wake OK");
        }

        if live_reattach_enabled() {
            let (session_id, terminal_id) = match resolve_terminal_ids(&target, &client) {
                Ok(ids) => ids,
                Err(msg) => fail_phase(&format!("P8 reattach: {msg}")),
            };
            let client_steal = Arc::clone(&client);
            cx.background_executor()
                .spawn(async move {
                    if let Ok(handle) = attach(
                        client_steal.as_ref(),
                        &session_id,
                        &terminal_id,
                        AttachOptions { read_only: false },
                    ) {
                        std::thread::sleep(Duration::from_secs(1));
                        drop(handle);
                    }
                })
                .detach();

            let mut p8_detached_ok = false;
            while Instant::now() < deadline {
                let ok = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        let snap = tab.inspect();
                        p.lifecycle == Lifecycle::Detached
                            && p.reattach_available
                            && p.detached_detail == Some(DetachedDetail::ClientDetached)
                            && snap.engine.is_some()
                            && !snap.bridge_alive
                    })
                    .unwrap_or(false);
                if ok {
                    p8_detached_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p8_detached_ok {
                fail_phase(
                    "P8 reattach: tab did not reach Detached(ClientDetached) with retained engine",
                );
            }

            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(TerminalHostEvent::Reattach, cx);
            });
            let mut p8_live_ok = false;
            while Instant::now() < deadline {
                let live = weak
                    .update(cx, |tab, _| tab.presentation().lifecycle == Lifecycle::Live)
                    .unwrap_or(false);
                if live {
                    p8_live_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p8_live_ok {
                fail_phase("P8 reattach: lifecycle did not return to Live");
            }
            eprintln!("terminal_live: P8 ClientDetached→Reattach OK");
        }

        // P9/P10 both consume a rotation and leave the terminal under a *new*
        // session, so the session the tab is attached to has to be tracked
        // across them — the env-derived `target` is stale after the first one.
        let mut current_ids: Option<(SessionId, TerminalId)> = None;

        if live_transfer_enabled() {
            let (session_a, terminal_id) = match resolve_terminal_ids(&target, &client) {
                Ok(ids) => ids,
                Err(msg) => fail_phase(&format!("P9 transfer: {msg}")),
            };

            // A distinct marker so survival can't be confused with P2's echo,
            // which the frozen engine also carries.
            let transfer_marker = format!("XFERMARK_{}", std::process::id());
            if !write_and_await_marker(&weak, cx, &transfer_marker).await {
                fail_phase("P9 transfer: pre-rotation marker never painted");
            }

            let session_b =
                match rotate_off_foreground(cx, &base_url, &session_a, &terminal_id).await {
                    Ok(b) => b,
                    Err(msg) => fail_phase(&format!("P9 transfer: {msg}")),
                };
            eprintln!("terminal_live: P9 rotated {session_a} -> {session_b}");

            // A `…/transfer` is **attach-transparent**: it rebinds the resource's
            // owning session without touching the terminal WS, so nothing is
            // observable from the transport alone. Assert that directly — it is
            // why the host's forwarded `resource.deleted` is load-bearing rather
            // than a belt-and-braces duplicate of a close code.
            cx.background_executor().timer(POLL_INTERVAL * 10).await;
            let still_live = weak
                .update(cx, |tab, _| {
                    tab.presentation().lifecycle == Lifecycle::Live && tab.inspect().bridge_alive
                })
                .unwrap_or(false);
            if !still_live {
                fail_phase(
                    "P9 transfer: the attach did NOT survive the transfer — a close code beat the \
                     host signal, so `resource.deleted` is no longer the sole trigger and this \
                     phase's premise needs revisiting",
                );
            }

            // Live order is `resource.deleted` then `session.superseded` (memory
            // `omnigent-clear-rotation-wire-contract`); this crate has no
            // session-SSE consumer, so the host's forwarding is driven by hand.
            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::ResourceDeleted {
                        terminal_id: terminal_id.clone(),
                    },
                    cx,
                );
            });

            if !await_replacement_waiting(&weak, cx).await {
                fail_phase("P9 transfer: tab did not reach ReplacementWaiting with a retained engine");
            }

            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::Transfer {
                        new_session: session_b.clone(),
                    },
                    cx,
                );
            });

            // The deterministic test can only prove the reuse branch is *taken*
            // (its offline attach then fails). This is the only place the reuse
            // is proven to actually land: Live against B, on the retained
            // engine, with the pre-rotation scrollback intact.
            let phase_deadline = Instant::now() + ROTATION_PHASE_BUDGET;
            let mut p9_ok = false;
            while Instant::now() < phase_deadline {
                let ok = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        p.lifecycle == Lifecycle::Live
                            && p.output_gap
                            && tab
                                .debug_latest_frame_for_test()
                                .is_some_and(|f| frame_contains_marker(&f, &transfer_marker))
                    })
                    .unwrap_or(false);
                if ok {
                    p9_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p9_ok {
                let (lifecycle, gap, detail) = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        (p.lifecycle, p.output_gap, p.detached_detail)
                    })
                    .unwrap_or((Lifecycle::Detached, false, None));
                fail_phase(&format!(
                    "P9 transfer: did not converge to Live on {session_b} with output_gap and \
                     surviving scrollback (lifecycle={lifecycle:?} output_gap={gap} detail={detail:?})"
                ));
            }
            eprintln!("terminal_live: P9 cross-session Transfer OK (scrollback survived, output_gap set)");
            current_ids = Some((session_b, terminal_id));
        }

        if live_4404_first_enabled() {
            // A `4404` on an `Existing` target is terminal by design (no successor
            // semantics — `existing_4404_while_live_hard_detaches`). Only the
            // discover-or-create shape waits for a replacement, so running this
            // phase against `Existing` would assert the wrong contract.
            if !matches!(target, TerminalTarget::OpenOrCreate { .. }) {
                fail_phase(
                    "P10 4404-first: requires an OpenOrCreate target \
                     (LENS_OMNIGENT_TERMINAL_NAME + LENS_OMNIGENT_SESSION_KEY)",
                );
            }
            let (session, old_terminal_id) = match current_ids
                .clone()
                .map(Ok)
                .unwrap_or_else(|| resolve_terminal_ids(&target, &client))
            {
                Ok(ids) => ids,
                Err(msg) => fail_phase(&format!("P10 4404-first: {msg}")),
            };
            let key = match &target {
                TerminalTarget::OpenOrCreate { key, .. } => key.clone(),
                TerminalTarget::Existing { .. } => unreachable!("guarded above"),
            };

            // The agent-switch shape: destroy the resource, then relaunch the same
            // key. Unlike a `…/transfer` (which P9 shows is attach-transparent), a
            // real delete tears the terminal down, so the server closes the WS —
            // this is the only cheap live source of a `4404`.
            let delete_client = Arc::clone(&client);
            let deleted = {
                let session = session.clone();
                let tid = old_terminal_id.clone();
                cx.background_executor()
                    .spawn(async move { delete_client.terminals(session).delete(&tid) })
                    .await
            };
            if let Err(e) = deleted {
                fail_phase(&format!("P10 4404-first: delete terminal: {e}"));
            }

            // Deliberately forward NOTHING yet. Reaching `ReplacementWaiting` now
            // can only be the transport's own `4404` — that IS the assertion: the
            // close code lands *before* any host signal and parks the tab (holding
            // the frozen engine) instead of hard-detaching it.
            if !await_replacement_waiting(&weak, cx).await {
                let (lifecycle, detail) = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        (p.lifecycle, p.detached_detail)
                    })
                    .unwrap_or((Lifecycle::Detached, None));
                fail_phase(&format!(
                    "P10 4404-first: the transport close did not park the tab in \
                     ReplacementWaiting (no host event was forwarded, so nothing else could \
                     have) — lifecycle={lifecycle:?} detail={detail:?}"
                ));
            }
            eprintln!("terminal_live: P10 transport 4404 parked the tab (host events withheld)");

            // Relaunch the same key. The successor's id is the *same*
            // deterministic `terminal_{name}_{key}` (memory
            // `terminal-resource-event-granularity`) — which is precisely why the
            // generation guard keys on the delete/create *events* and not on an id
            // change. Live-confirmed here: the delete demonstrably took (the 4404
            // above proves it) yet the recreate returns `terminal_shell_main` again.
            let create_client = Arc::clone(&client);
            let successor = {
                let session = session.clone();
                let req = TerminalCreate {
                    terminal: key.terminal_name.clone(),
                    session_key: key.session_key.clone(),
                };
                cx.background_executor()
                    .spawn(async move { create_client.terminals(session).create(&req) })
                    .await
            };
            let successor = match successor {
                Ok(r) => r,
                Err(e) => fail_phase(&format!("P10 4404-first: recreate terminal: {e}")),
            };
            // The host's signals arrive *after* the close, in server order.
            let _ = weak.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::ResourceDeleted {
                        terminal_id: old_terminal_id.clone(),
                    },
                    cx,
                );
                tab.on_host_event(
                    TerminalHostEvent::ResourceCreated {
                        session_id: session.clone(),
                        terminal_id: successor.id.clone(),
                        terminal_name: key.terminal_name.clone(),
                        session_key: key.session_key.clone(),
                    },
                    cx,
                );
            });

            // Same-session adopt is a *fresh* attach by design (the retained engine
            // belongs to the dead generation and is dropped), so scrollback is NOT
            // expected to survive here — only convergence to `Live` is.
            let phase_deadline = Instant::now() + ROTATION_PHASE_BUDGET;
            let mut p10_ok = false;
            while Instant::now() < phase_deadline {
                let ok = weak
                    .update(cx, |tab, _| tab.presentation().lifecycle == Lifecycle::Live)
                    .unwrap_or(false);
                if ok {
                    p10_ok = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !p10_ok {
                let (lifecycle, detail) = weak
                    .update(cx, |tab, _| {
                        let p = tab.presentation();
                        (p.lifecycle, p.detached_detail)
                    })
                    .unwrap_or((Lifecycle::Detached, None));
                fail_phase(&format!(
                    "P10 4404-first: adoption of successor {} did not converge to Live \
                     (lifecycle={lifecycle:?} detail={detail:?})",
                    successor.id
                ));
            }
            eprintln!(
                "terminal_live: P10 4404-first → adopt OK ({old_terminal_id} -> {})",
                successor.id
            );
        }

        eprintln!("terminal_live: PASS");
        std::process::exit(0);
    })
    .detach();
}

fn live_mouse_report_enabled() -> bool {
    std::env::var("LENS_LIVE_MOUSE_REPORT")
        .ok()
        .is_some_and(|v| v == "1")
}

fn live_sleep_wake_enabled() -> bool {
    std::env::var("LENS_LIVE_SLEEP_WAKE")
        .ok()
        .is_some_and(|v| v == "1")
}

fn live_reattach_enabled() -> bool {
    std::env::var("LENS_LIVE_REATTACH")
        .ok()
        .is_some_and(|v| v == "1")
}

fn resolve_terminal_ids(
    target: &TerminalTarget,
    client: &Client,
) -> Result<(SessionId, TerminalId), String> {
    match target {
        TerminalTarget::Existing {
            session_id,
            terminal_id,
        } => Ok((session_id.clone(), terminal_id.clone())),
        TerminalTarget::OpenOrCreate { session_id, key } => {
            let terminals = client
                .terminals(session_id.clone())
                .list()
                .map_err(|e| format!("list terminals: {e}"))?;
            terminals
                .into_iter()
                .find(|t| {
                    t.metadata.terminal_name.as_deref() == Some(key.terminal_name.as_str())
                        && t.metadata.session_key.as_deref() == Some(key.session_key.as_str())
                })
                .map(|t| (session_id.clone(), t.id))
                .ok_or_else(|| format!("no terminal for {}:{}", key.terminal_name, key.session_key))
        }
    }
}

fn live_clipboard_paste_enabled() -> bool {
    std::env::var("LENS_LIVE_CLIPBOARD_PASTE")
        .ok()
        .is_some_and(|v| v == "1")
}

fn live_transfer_enabled() -> bool {
    std::env::var("LENS_LIVE_TRANSFER")
        .ok()
        .is_some_and(|v| v == "1")
}

fn live_4404_first_enabled() -> bool {
    std::env::var("LENS_LIVE_4404_FIRST")
        .ok()
        .is_some_and(|v| v == "1")
}

// ---------------------------------------------------------------------------
// P9/P10 helpers — rotation (server side) + shared waits
// ---------------------------------------------------------------------------

/// Per-phase budget for P9/P10. Deliberately under `TerminalTab`'s 30s
/// `REPLACEMENT_WAIT` so a stall here is reported as *our* timeout rather than
/// racing the tab's own `ReplacementTimedOut` detach.
const ROTATION_PHASE_BUDGET: Duration = Duration::from_secs(20);

/// Echo `marker` into the pane and wait for it to reach the rendered frame.
async fn write_and_await_marker(
    weak: &gpui::WeakEntity<TerminalTab>,
    cx: &mut gpui::AsyncApp,
    marker: &str,
) -> bool {
    let sent = weak
        .update(cx, |tab, _| {
            tab.debug_send_input_for_test(format!("echo {marker}\r").into_bytes())
        })
        .unwrap_or(false);
    if !sent {
        return false;
    }
    let deadline = Instant::now() + ROTATION_PHASE_BUDGET;
    while Instant::now() < deadline {
        let seen = weak
            .update(cx, |tab, _| {
                tab.debug_latest_frame_for_test()
                    .is_some_and(|f| frame_contains_marker(&f, marker))
            })
            .unwrap_or(false);
        if seen {
            return true;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    false
}

/// Wait for `ReplacementWaiting` **with the frozen engine still retained** —
/// retention is what makes the subsequent `Transfer` a reuse rather than a
/// fresh attach, so asserting the lifecycle alone would be too weak.
async fn await_replacement_waiting(
    weak: &gpui::WeakEntity<TerminalTab>,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let deadline = Instant::now() + ROTATION_PHASE_BUDGET;
    while Instant::now() < deadline {
        let ok = weak
            .update(cx, |tab, _| {
                tab.presentation().lifecycle == Lifecycle::ReplacementWaiting
                    && tab.inspect().engine.is_some()
            })
            .unwrap_or(false);
        if ok {
            return true;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    false
}

async fn rotate_off_foreground(
    cx: &mut gpui::AsyncApp,
    base_url: &str,
    from: &SessionId,
    terminal_id: &TerminalId,
) -> Result<SessionId, String> {
    let base_url = base_url.to_string();
    let from = from.clone();
    let terminal_id = terminal_id.clone();
    cx.background_executor()
        .spawn(async move { rotate_terminal_to_new_session(&base_url, &from, &terminal_id) })
        .await
}

fn send_json(req: reqwest::blocking::RequestBuilder, what: &str) -> Result<Value, String> {
    let req = match std::env::var("LENS_OMNIGENT_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        Some(token) => req.bearer_auth(token),
        None => req,
    };
    let resp = req.send().map_err(|e| format!("{what}: {e}"))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{what}: HTTP {status}: {body}"));
    }
    // Some of these routes answer with an empty body on success.
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body).map_err(|e| format!("{what}: unparseable body ({e}): {body}"))
}

/// Drive the server-side half of a `/clear` rotation: create a sibling session
/// bound to the same runner, then move `terminal_id` into it.
///
/// Mirrors the claude-native forwarder's REST sequence (memory
/// `omnigent-clear-rotation-wire-contract`) **minus** its final
/// `external_session_superseded` event: that event is `lens-ui`'s input — this
/// crate has no session-SSE consumer, so posting it here would have no observer.
/// The two routes used are `include_in_schema=False` upstream, hence absent from
/// `openapi.json` and unmodeled in `lens-client`; they are issued raw.
fn rotate_terminal_to_new_session(
    base_url: &str,
    from: &SessionId,
    terminal_id: &TerminalId,
) -> Result<SessionId, String> {
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build rest client: {e}"))?;
    let base = base_url.trim_end_matches('/');

    let source = send_json(
        http.get(format!("{base}/v1/sessions/{from}")),
        "GET source session",
    )?;
    // `agent_id`, not `agent` — the latter is silently accepted and yields a
    // session with no id.
    let agent_id = source
        .get("agent_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("source session {from} has no agent_id: {source}"))?
        .to_string();
    let runner_id = source
        .get("runner_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let created = send_json(
        http.post(format!("{base}/v1/sessions"))
            .json(&serde_json::json!({ "agent_id": agent_id })),
        "POST replacement session",
    )?;
    let new_session = created
        .get("id")
        .or_else(|| created.get("session_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("created session has no id: {created}"))?
        .to_string();

    if let Some(runner_id) = runner_id {
        send_json(
            http.patch(format!("{base}/v1/sessions/{new_session}"))
                .json(&serde_json::json!({ "runner_id": runner_id })),
            "PATCH bind runner",
        )?;
    }

    send_json(
        http.post(format!(
            "{base}/v1/sessions/{from}/resources/terminals/{terminal_id}/transfer"
        ))
        .json(&serde_json::json!({ "target_session_id": new_session })),
        "POST transfer terminal",
    )?;

    Ok(SessionId::new(new_session))
}

// ---------------------------------------------------------------------------
// Manual IME checklist (Slice 2a Task 5 — not automatable in CI)
// ---------------------------------------------------------------------------
//
// With a CJK input source active on macOS:
// 1. Focus the live terminal tab; confirm IME candidate window tracks the cursor.
// 2. Compose preedit (e.g. pinyin "nihao") — preedit overlay appears at cursor;
//    no PTY bytes until commit.
// 3. Commit (space/enter) — exactly one UTF-8 commit hits the remote shell.
// 4. Cancel composition (escape) — preedit overlay clears; no egress.
// 5. Switch input source back to ABC; plain typing still single-emits via InputHandler.
