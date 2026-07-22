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

use std::time::Duration;

use gpui::{App, AppContext, Application, Entity, KeyBinding, WindowOptions, actions};
use gpui_component::Root;
use lens_client::ids::{ConnectionId, SessionId, TerminalId};
use lens_client::{Auth, Client, Connection};
use lens_terminal::{
    ClipboardLocation, ClipboardMimePart, HostRequestDecision, TerminalEvent, TerminalHostEvent,
    TerminalKey, TerminalOpenOptions, TerminalTab, TerminalTarget, open,
};

actions!(
    lens_terminal_demo,
    [
        DemoSleep,
        DemoWake,
        DemoReattach,
        DemoResetAdopt,
        DemoResetTimeout,
    ]
);

struct DemoConfig {
    base_url: url::Url,
    session_id: SessionId,
    target: TerminalTarget,
}

struct DemoHostBindings {
    tab: Entity<TerminalTab>,
    client: Arc<Client>,
    session_id: SessionId,
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
    let session_id = config.session_id;
    let options = TerminalOpenOptions::default();

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        print_host_action_help();

        match cx.open_window(WindowOptions::default(), move |window, cx| {
            let tab = open(target.clone(), client.clone(), options, cx);
            register_host_action_bindings(
                DemoHostBindings {
                    tab: tab.clone(),
                    client,
                    session_id,
                    target,
                },
                cx,
            );
            let tab_for_events = tab.clone();
            let _ = cx.subscribe(&tab_for_events, move |this, event, cx| {
                match event {
                    TerminalEvent::OpenUrlRequest { id, url } => {
                        eprintln!("demo: OpenUrlRequest id={id:?} url={url}");
                        let id = *id;
                        this.update(cx, |tab, cx| {
                            tab.on_host_event(
                                TerminalHostEvent::HostRequestResponse {
                                    id,
                                    decision: HostRequestDecision::Allow,
                                },
                                cx,
                            );
                        });
                    }
                    TerminalEvent::ClipboardWriteRequest {
                        id,
                        location,
                        contents,
                    } => {
                        let bytes: usize = contents.iter().map(|p: &ClipboardMimePart| p.data.len()).sum();
                        let decision = clipboard_write_decision();
                        eprintln!(
                            "demo: ClipboardWriteRequest id={id:?} loc={location:?} bytes={bytes} — {decision:?} by policy"
                        );
                        let id = *id;
                        this.update(cx, |tab, cx| {
                            tab.on_host_event(
                                TerminalHostEvent::HostRequestResponse { id, decision },
                                cx,
                            );
                        });
                    }
                    TerminalEvent::PasteWarnRequest { id, line_count } => {
                        let decision = paste_warn_decision();
                        eprintln!(
                            "demo: PasteWarnRequest id={id:?} lines={line_count} — {decision:?} by policy"
                        );
                        let id = *id;
                        this.update(cx, |tab, cx| {
                            tab.on_host_event(
                                TerminalHostEvent::HostRequestResponse { id, decision },
                                cx,
                            );
                        });
                    }
                    TerminalEvent::ClipboardWriteNotice { location, bytes } => {
                        let _loc: &ClipboardLocation = location;
                        eprintln!("demo: ClipboardWriteNotice loc={location:?} bytes={bytes}");
                    }
                    TerminalEvent::PresentationChanged => {
                        let p = this.read(cx).presentation();
                        eprintln!(
                            "demo: presentation identity={} reported={:?}",
                            p.identity_title, p.reported_title
                        );
                    }
                    _ => {
                        // Forward-compat: never auto-allow unknown host requests.
                        // `#[non_exhaustive]` — catch-all required.
                        eprintln!("demo: unhandled TerminalEvent — Deny by policy");
                    }
                }
            });
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

fn demo_allow_clipboard() -> bool {
    std::env::var("LENS_DEMO_ALLOW_CLIPBOARD")
        .ok()
        .is_some_and(|v| v == "1")
}

fn clipboard_write_decision() -> HostRequestDecision {
    if demo_allow_clipboard() {
        HostRequestDecision::Allow
    } else {
        HostRequestDecision::Deny
    }
}

fn paste_warn_decision() -> HostRequestDecision {
    if demo_allow_clipboard() {
        HostRequestDecision::AllowSession
    } else {
        HostRequestDecision::Deny
    }
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
            session_id: session_id.clone(),
            terminal_id: TerminalId::new(tid),
        }
    } else if let (Some(terminal_name), Some(session_key)) = (terminal_name, session_key) {
        TerminalTarget::OpenOrCreate {
            session_id: session_id.clone(),
            key: TerminalKey {
                terminal_name,
                session_key,
            },
        }
    } else {
        return Ok(None);
    };

    Ok(Some(DemoConfig {
        base_url,
        session_id,
        target,
    }))
}

fn print_host_action_help() {
    eprintln!(
        "lens-terminal-demo host chords: ctrl-alt-s=Sleep, ctrl-alt-w=Wake, ctrl-alt-r=Reattach, ctrl-alt-x=reset→adopt, ctrl-alt-d=reset→timeout"
    );
}

fn register_host_action_bindings(bindings: DemoHostBindings, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-alt-s", DemoSleep, None),
        KeyBinding::new("ctrl-alt-w", DemoWake, None),
        KeyBinding::new("ctrl-alt-r", DemoReattach, None),
        KeyBinding::new("ctrl-alt-x", DemoResetAdopt, None),
        KeyBinding::new("ctrl-alt-d", DemoResetTimeout, None),
    ]);

    let tab = bindings.tab.clone();
    cx.on_action::<DemoSleep>(move |_, cx| {
        tab.update(cx, |tab, cx| {
            tab.on_host_event(TerminalHostEvent::Sleep, cx)
        });
    });

    let tab = bindings.tab.clone();
    cx.on_action::<DemoWake>(move |_, cx| {
        tab.update(cx, |tab, cx| tab.on_host_event(TerminalHostEvent::Wake, cx));
    });

    let tab = bindings.tab.clone();
    cx.on_action::<DemoReattach>(move |_, cx| {
        tab.update(cx, |tab, cx| {
            tab.on_host_event(TerminalHostEvent::Reattach, cx)
        });
    });

    let tab = bindings.tab.clone();
    let client = bindings.client.clone();
    let session_id = bindings.session_id.clone();
    let target = bindings.target.clone();
    cx.on_action::<DemoResetAdopt>(move |_, cx| {
        let tab = tab.clone();
        let client = client.clone();
        let session_id = session_id.clone();
        let target = target.clone();
        cx.spawn(async move |cx| {
            let Some((terminal_id, terminal_name, session_key)) =
                resolve_reset_identity(&client, &session_id, &target)
            else {
                eprintln!("demo: reset-adopt: could not resolve terminal identity");
                return;
            };
            let _ = tab.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::ResourceDeleted {
                        terminal_id: terminal_id.clone(),
                    },
                    cx,
                );
            });
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = tab.update(cx, |tab, cx| {
                tab.on_host_event(
                    TerminalHostEvent::ResourceCreated {
                        session_id: session_id.clone(),
                        terminal_id,
                        terminal_name,
                        session_key,
                    },
                    cx,
                );
            });
        })
        .detach();
    });

    let tab = bindings.tab.clone();
    let client = bindings.client.clone();
    let session_id = bindings.session_id.clone();
    let target = bindings.target.clone();
    cx.on_action::<DemoResetTimeout>(move |_, cx| {
        let Some((terminal_id, _, _)) = resolve_reset_identity(&client, &session_id, &target)
        else {
            eprintln!("demo: reset-timeout: could not resolve terminal identity");
            return;
        };
        tab.update(cx, |tab, cx| {
            tab.on_host_event(TerminalHostEvent::ResourceDeleted { terminal_id }, cx);
        });
    });
}

fn resolve_reset_identity(
    client: &Client,
    session_id: &SessionId,
    target: &TerminalTarget,
) -> Option<(TerminalId, String, String)> {
    match target {
        TerminalTarget::Existing { terminal_id, .. } => {
            let (terminal_name, session_key) = lookup_terminal_key(client, session_id, terminal_id)
                .unwrap_or_else(|| ("terminal".into(), "main".into()));
            Some((terminal_id.clone(), terminal_name, session_key))
        }
        TerminalTarget::OpenOrCreate { key, .. } => client
            .terminals(session_id.clone())
            .list()
            .ok()
            .and_then(|terminals| {
                terminals.into_iter().find(|t| {
                    t.metadata.terminal_name.as_deref() == Some(key.terminal_name.as_str())
                        && t.metadata.session_key.as_deref() == Some(key.session_key.as_str())
                })
            })
            .map(|t| (t.id, key.terminal_name.clone(), key.session_key.clone())),
    }
}

fn lookup_terminal_key(
    client: &Client,
    session_id: &SessionId,
    terminal_id: &TerminalId,
) -> Option<(String, String)> {
    client
        .terminals(session_id.clone())
        .get(terminal_id)
        .ok()
        .and_then(|t| {
            let name = t.metadata.terminal_name?;
            let key = t.metadata.session_key?;
            Some((name, key))
        })
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
