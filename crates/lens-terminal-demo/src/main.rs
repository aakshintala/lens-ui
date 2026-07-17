//! Standalone GPUI demo for `lens-terminal` — opens a window hosting a
//! [`lens_terminal::TerminalTab`] attached to a live omnigent terminal.
//!
//! # Environment
//!
//! Read **before** any GPUI call (on the main thread).
//!
//! | Variable | Required | Role |
//! | --- | --- | --- |
//! | `LENS_OMNIGENT_URL` | yes | Omnigent base URL (e.g. `http://127.0.0.1:8080`) |
//! | `LENS_OMNIGENT_SESSION_ID` | yes | Session / conversation id |
//! | `LENS_OMNIGENT_TERMINAL_ID` | target A | Attach to an existing terminal (opaque id) |
//! | `LENS_OMNIGENT_TERMINAL_NAME` | target B | Logical terminal name (with `LENS_OMNIGENT_SESSION_KEY`) |
//! | `LENS_OMNIGENT_SESSION_KEY` | target B | Session key paired with terminal name |
//! | `LENS_OMNIGENT_TOKEN` | no | Optional bearer token (`Auth::Bearer`) |
//!
//! **Target (exactly one):**
//! - If `LENS_OMNIGENT_TERMINAL_ID` is set →
//!   [`lens_terminal::TerminalTarget::Existing`].
//! - Else if both `LENS_OMNIGENT_TERMINAL_NAME` and `LENS_OMNIGENT_SESSION_KEY`
//!   are set → [`lens_terminal::TerminalTarget::OpenOrCreate`].
//! - Otherwise the demo is **not configured** (see exit codes).
//!
//! # Handshake
//!
//! [`lens_client::Client::new`] runs on the main thread **before**
//! [`gpui::Application::new`]: health → version → info. No HTTP inside the GPUI
//! callback.
//!
//! # Exit codes
//!
//! - `0` — required env missing or no valid target; usage printed to stderr.
//! - `1` — env configured but omnigent handshake failed.

use std::process;
use std::sync::Arc;

use gpui::{App, AppContext, Application, WindowOptions};
use gpui_component::Root;
use lens_client::ids::{ConnectionId, SessionId, TerminalId};
use lens_client::{Auth, Client, Connection};
use lens_terminal::{TerminalKey, TerminalOpenOptions, TerminalTarget, open};

struct DemoConfig {
    base_url: url::Url,
    target: TerminalTarget,
}

fn main() {
    let config = match load_config() {
        Ok(Some(c)) => c,
        Ok(None) => {
            print_usage();
            process::exit(0);
        }
        Err(msg) => {
            eprintln!("lens-terminal-demo: {msg}");
            process::exit(1);
        }
    };

    let client = match build_client(&config.base_url) {
        Ok(client) => Arc::new(client),
        Err(err) => {
            eprintln!("lens-terminal-demo: {err}");
            process::exit(1);
        }
    };

    let target = config.target;
    let options = TerminalOpenOptions::default();

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);

        match cx.open_window(WindowOptions::default(), move |window, cx| {
            let tab = open(target, client, options, cx);
            let any: gpui::AnyView = tab.into();
            cx.new(|cx| Root::new(any, window, cx))
        }) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("lens-terminal-demo: open_window: {e}");
                std::process::exit(1);
            }
        }
        cx.activate(true);
    });
}

fn load_config() -> Result<Option<DemoConfig>, String> {
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

    let base_url = base_url_str
        .parse::<url::Url>()
        .map_err(|e| format!("invalid LENS_OMNIGENT_URL `{base_url_str}`: {e}"))?;

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
        return Ok(None);
    };

    Ok(Some(DemoConfig { base_url, target }))
}

fn build_client(base_url: &url::Url) -> lens_client::Result<Client> {
    let auth = match std::env::var("LENS_OMNIGENT_TOKEN") {
        Ok(token) if !token.is_empty() => Auth::Bearer { token },
        _ => Auth::None,
    };
    let conn = Connection::new(
        ConnectionId::new("lens-terminal-demo"),
        base_url.clone(),
        auth,
    );
    Client::new(conn)
}

fn print_usage() {
    eprintln!(
        "\
lens-terminal-demo — standalone GPUI window for a live omnigent terminal

Required environment:
  LENS_OMNIGENT_URL           omnigent base URL (e.g. http://127.0.0.1:8080)
  LENS_OMNIGENT_SESSION_ID    session / conversation id

Target (set exactly one):
  LENS_OMNIGENT_TERMINAL_ID   attach to an existing terminal (opaque id)
    — or —
  LENS_OMNIGENT_TERMINAL_NAME + LENS_OMNIGENT_SESSION_KEY
                              open-or-create by logical key

Optional:
  LENS_OMNIGENT_TOKEN         bearer token for authenticated servers

Exit 0 when env is not configured; exit 1 on handshake failure."
    );
}
