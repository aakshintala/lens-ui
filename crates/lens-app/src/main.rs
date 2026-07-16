mod fleet_verify;

use gpui::{App, AppContext, Application, WindowOptions};
use gpui_component::Root;
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{GetOpts, SessionSnapshot, SessionStatus};
use lens_client::{Auth, Client, Connection};
use lens_core::actor::ActorStores;
use lens_core::domain::ids::AgentId;
use lens_core::domain::scalars::{ErrorInfo, SessionLifecycle, SessionStatusValue};
use lens_core::domain::session::SessionState;
use lens_core::domain::usage::Cost;
use lens_core::persist::{
    ConnectionRecord, ControlStore, SqliteControlStore, SqliteTranscriptStore,
};
use lens_ui::board::BoardView;
use lens_ui::card::model::SessionCard;
use lens_ui::clock::{UiClock, WallUiClock};
use lens_ui::fleet::store::FleetStore;
use lens_ui::slot::placeholder_tab;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

struct Config {
    base_url: url::Url,
    session_ids: Vec<SessionId>,
    data_dir: PathBuf,
    fleet_verify: bool,
    fleet_count: usize,
    demo: bool,
}

struct LivePrep {
    conn: Connection,
    client: Client,
    session_ids: Vec<SessionId>,
    data_dir: PathBuf,
}

fn main() {
    let config = match parse_config() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("lens-app: {msg}");
            process::exit(2);
        }
    };

    if config.fleet_verify {
        let exit = fleet_verify::run(fleet_verify::FleetVerifyOptions {
            base_url: config.base_url,
            count: config.fleet_count,
            data_dir: config.data_dir,
        });
        process::exit(exit);
    }

    if config.demo {
        run_demo();
        return;
    }

    let live_prep = if config.session_ids.is_empty() {
        None
    } else {
        match prepare_live(&config) {
            Ok(p) => Some(p),
            Err(msg) => {
                eprintln!("lens-app: {msg}");
                process::exit(1);
            }
        }
    };

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);

        let clock = Arc::new(WallUiClock) as Arc<dyn UiClock>;
        let fleet = FleetStore::new_live(clock, cx);
        lens_ui::shortcuts::register(&fleet, cx);

        cx.open_window(WindowOptions::default(), move |window, cx| {
            if let Some(prep) = live_prep {
                fleet.update(cx, |fleet, cx| {
                    for sid in prep.session_ids {
                        if let Err(e) = fleet.spawn_live_session(
                            &prep.conn,
                            &prep.client,
                            sid.clone(),
                            &prep.data_dir,
                            cx,
                        ) {
                            eprintln!("lens-app: spawn_live_session {sid}: {e}");
                        }
                    }
                });
            }
            let board = cx.new(|cx| BoardView::mount(fleet.clone(), placeholder_tab(cx), None, cx));
            let any: gpui::AnyView = board.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .ok();
        cx.activate(true);
    });
}

/// `--demo`: paint six cards in the six wave states (no live server needed) so the
/// status language is visible at a glance. Cards carry no poller/commands — clicking
/// one still toggles focus (and suppresses that card's glow while focused).
fn run_demo() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);

        let clock = Arc::new(WallUiClock) as Arc<dyn UiClock>;
        let now = clock.now_millis();
        let fleet = FleetStore::new_live(clock, cx);
        lens_ui::shortcuts::register(&fleet, cx);

        cx.open_window(WindowOptions::default(), move |window, cx| {
            fleet.update(cx, |f, cx| {
                for card in demo_cards(now) {
                    let id = card.session_id.clone();
                    let entity = cx.new(|_| card);
                    f.cards.insert(id, entity);
                }
                cx.notify();
            });
            let board = cx.new(|cx| BoardView::mount(fleet.clone(), placeholder_tab(cx), None, cx));
            let any: gpui::AnyView = board.into();
            cx.new(|cx| Root::new(any, window, cx))
        })
        .ok();
        cx.activate(true);
    });
}

/// Six preset cards, one per wave state.
fn demo_cards(now: i64) -> Vec<SessionCard> {
    let base = |id: &str, title: &str| {
        let mut c = SessionCard::new(SessionId::new(id));
        c.harness = Some("claude-sdk".into());
        c.llm_model = Some("opus".into());
        c.workspace = Some("lens".into());
        c.git_branch = Some("main".into());
        c.context_window = Some(200_000);
        c.last_total_tokens = Some(48_000);
        c.cumulative_cost = Cost {
            total_cost_usd: Some(0.42),
            ..Cost::default()
        };
        c.title = Some(title.into());
        c
    };

    let mut needs_input = base("demo-needs-input", "Approve: run `rm -rf build/`?");
    needs_input.status = SessionStatusValue::Waiting;
    needs_input.needs_attention = true;
    needs_input.activity_summary = "awaiting your approval".into();

    let mut ready = base("demo-ready", "Finished — reply with a greeting");
    ready.status = SessionStatusValue::Idle;
    ready.seeded = true;
    ready.seen_turn = 1;
    ready.last_completed_at = Some(now);

    let mut working = base("demo-working", "Refactor the session poller");
    working.status = SessionStatusValue::Running;
    working.activity_summary = "running the test suite…".into();

    let mut failed = base("demo-failed", "cargo build");
    failed.status = SessionStatusValue::Failed;
    failed.last_task_error = Some(ErrorInfo {
        code: "build_error".into(),
        message: "E0432: unresolved import".into(),
    });

    let mut slept = base("demo-slept", "Archived brainstorm");
    slept.status = SessionStatusValue::Idle;
    slept.lifecycle = SessionLifecycle::Slept;

    let neutral = base("demo-neutral", "Fresh session — no activity yet");

    vec![needs_input, ready, working, failed, slept, neutral]
}

fn prepare_live(config: &Config) -> Result<LivePrep, String> {
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| format!("failed to create {}: {e}", config.data_dir.display()))?;

    let auth = auth_from_env();
    let conn_id = ConnectionId::new("lens-app");
    let conn = Connection::new(conn_id, config.base_url.clone(), auth);
    let client = Client::new(conn.clone()).map_err(|e| format!("connect failed: {e}"))?;

    // One connection, all sessions share the control store; each seeds its own transcript.
    for session_id in &config.session_ids {
        let snap = client
            .sessions()
            .get(session_id, GetOpts::default())
            .map_err(|e| format!("failed to resolve session {session_id}: {e}"))?;
        seed_disk(&conn, session_id, &config.data_dir, &snap)?;
    }

    Ok(LivePrep {
        conn,
        client,
        session_ids: config.session_ids.clone(),
        data_dir: config.data_dir.clone(),
    })
}

fn parse_config() -> Result<Config, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut base_url: Option<String> = None;
    let mut session_ids: Vec<SessionId> = Vec::new();
    let mut data_dir: Option<PathBuf> = None;
    let mut fleet_verify = false;
    let mut fleet_count: usize = 10;
    let mut demo = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            "--fleet-verify" => {
                fleet_verify = true;
                i += 1;
            }
            "--demo" => {
                demo = true;
                i += 1;
            }
            "--count" => {
                i += 1;
                let raw = next_flag_value(&args, i, "--count")?;
                fleet_count = raw
                    .parse::<usize>()
                    .map_err(|_| format!("--count must be a positive integer, got `{raw}`"))?;
                if fleet_count == 0 {
                    return Err("--count must be at least 1".into());
                }
                i += 1;
            }
            "--base-url" => {
                i += 1;
                base_url = Some(next_flag_value(&args, i, "--base-url")?);
                i += 1;
            }
            "--session" => {
                i += 1;
                let id = next_flag_value(&args, i, "--session")?;
                // Repeatable: pass --session multiple times to show several cards.
                session_ids.push(SessionId::new(id));
                i += 1;
            }
            "--sessions" => {
                i += 1;
                let raw = next_flag_value(&args, i, "--sessions")?;
                session_ids.extend(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(SessionId::new),
                );
                i += 1;
            }
            "--data-dir" => {
                i += 1;
                data_dir = Some(PathBuf::from(next_flag_value(&args, i, "--data-dir")?));
                i += 1;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unknown flag: {arg}"));
            }
            _ => return Err(format!("unexpected argument: {}", args[i])),
        }
    }

    let base_url = base_url
        .or_else(|| std::env::var("LENS_OMNIGENT_URL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());

    let base_url = base_url
        .parse::<url::Url>()
        .map_err(|e| format!("invalid base URL `{base_url}`: {e}"))?;

    let data_dir = data_dir.unwrap_or_else(|| {
        if fleet_verify {
            std::env::temp_dir().join("lens-fleet-verify")
        } else {
            let sid = session_ids.first().map(|s| s.as_str()).unwrap_or("board");
            std::env::temp_dir().join(format!("lens-app-{sid}"))
        }
    });

    Ok(Config {
        base_url,
        session_ids,
        data_dir,
        fleet_verify,
        fleet_count,
        demo,
    })
}

fn next_flag_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index)
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "\
lens-app — Lens native macOS client (board shell)

Usage:
  lens-app [--base-url URL] [--session CONV_ID] [--data-dir PATH]
  lens-app --fleet-verify [--count N] [--base-url URL] [--data-dir PATH]

With --session, attaches a live omnigent session on the real FleetScheduler.
Without --session, opens an empty board.

Environment:
  LENS_OMNIGENT_URL     default base URL (fallback http://127.0.0.1:8080)
  LENS_OMNIGENT_TOKEN   optional bearer token
{}",
        fleet_verify::help_section()
    );
}

fn auth_from_env() -> Auth {
    match std::env::var("LENS_OMNIGENT_TOKEN") {
        Ok(token) if !token.is_empty() => Auth::Bearer { token },
        _ => Auth::None,
    }
}

fn wall_clock_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn open_stores(
    data_dir: &Path,
    conn_id: &ConnectionId,
    session_id: &SessionId,
) -> Result<ActorStores, String> {
    let control = SqliteControlStore::open(&data_dir.join("lens.db"))
        .map_err(|e| format!("control store: {e}"))?;
    let transcript = SqliteTranscriptStore::open(
        &data_dir.join(format!("{session_id}.db")),
        conn_id,
        session_id,
    )
    .map_err(|e| format!("transcript store: {e}"))?;
    Ok(ActorStores {
        control: Box::new(control),
        transcript: Box::new(transcript),
    })
}

fn seed_disk(
    conn: &Connection,
    session_id: &SessionId,
    data_dir: &Path,
    snap: &SessionSnapshot,
) -> Result<(), String> {
    let stores = open_stores(data_dir, &conn.id, session_id)?;
    let now = wall_clock_millis();
    seed_connection(stores.control.as_ref(), conn, now)
        .map_err(|e| format!("seed connection: {e}"))?;
    seed_session(stores.control.as_ref(), &conn.id, snap, now)
        .map_err(|e| format!("seed session: {e}"))?;
    Ok(())
}

fn seed_connection(
    control: &dyn ControlStore,
    conn: &Connection,
    now: i64,
) -> lens_core::persist::Result<()> {
    let auth_kind = match &conn.auth {
        Auth::None => "none",
        Auth::Bearer { .. } => "bearer",
        Auth::Cookie { .. } => "cookie",
        Auth::ForwardedEmail { .. } => "forwarded_email",
    };
    control.upsert_connection(&ConnectionRecord {
        id: conn.id.clone(),
        base_url: conn.base_url.to_string(),
        auth_kind: auth_kind.into(),
        label: Some("lens-app".into()),
        server_info: None,
        created_at: now,
    })
}

fn seed_session(
    control: &dyn ControlStore,
    conn_id: &ConnectionId,
    snap: &SessionSnapshot,
    now: i64,
) -> lens_core::persist::Result<()> {
    let mut state = SessionState::new(
        conn_id.clone(),
        snap.id().clone(),
        AgentId::new(snap.agent_id()),
    );
    state.status = match snap.status() {
        SessionStatus::Idle => SessionStatusValue::Idle,
        SessionStatus::Running => SessionStatusValue::Running,
        SessionStatus::Failed => SessionStatusValue::Failed,
    };
    state.agent_name = snap.agent_name().map(str::to_string);
    state.title = snap.title().map(str::to_string);
    state.created_at = snap.created_at();
    state.archived = snap.archived();
    control.upsert_session(&state, now)
}
