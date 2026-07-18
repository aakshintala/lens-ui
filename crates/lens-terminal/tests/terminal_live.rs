//! Live terminal vertical rider (Slice 1d Task 9): `open()` → attach → input →
//! real-window paint → forced network loss → reconnect with `output_gap`.
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
//!
//! # Skip vs fail
//!
//! - **Skip (exit 0):** `LENS_OMNIGENT_URL` or `LENS_OMNIGENT_SESSION_ID` absent;
//!   or URL+session present but no valid terminal target.
//! - **Fail (exit 1):** env configured but client handshake or any driver phase times out.
//! - **Pass (exit 0):** all four phases complete.
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
    App, Application, Bounds, Entity, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use lens_client::ids::{ConnectionId, SessionId, TerminalId};
use lens_client::{Auth, Client, Connection};
use lens_terminal::{
    Frame, Lifecycle, TerminalKey, TerminalOpenOptions, TerminalTab, TerminalTarget, open,
};

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
                let tab = open(target, client, options, cx);
                tab.update(cx, |tab, _| tab.set_render_inspect_enabled(true));
                spawn_driver(tab.clone(), marker.clone(), cx);
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

fn spawn_driver(tab: Entity<TerminalTab>, marker: String, cx: &mut App) {
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

        eprintln!("terminal_live: PASS");
        std::process::exit(0);
    })
    .detach();
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
