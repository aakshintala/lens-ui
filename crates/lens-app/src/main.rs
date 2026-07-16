mod fleet_verify;

use gpui::{App, AppContext, Application, KeyBinding, WindowOptions};
use gpui_component::Root;
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{GetOpts, SessionSnapshot, SessionStatus};
use lens_client::{Auth, Client, Connection};
use lens_core::actor::ActorStores;
use lens_core::domain::ids::AgentId;
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::session::SessionState;
use lens_core::persist::{
    ConnectionRecord, ControlStore, SqliteControlStore, SqliteTranscriptStore,
};
use lens_ui::actions::BackToBoard;
use lens_ui::board::BoardView;
use lens_ui::clock::{UiClock, WallUiClock};
use lens_ui::fleet::store::FleetStore;
use lens_ui::slot::placeholder_tab;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

struct Config {
    base_url: url::Url,
    session_id: Option<SessionId>,
    data_dir: PathBuf,
    fleet_verify: bool,
    fleet_count: usize,
}

struct LivePrep {
    conn: Connection,
    client: Client,
    session_id: SessionId,
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

    let live_prep = match config.session_id.as_ref() {
        Some(sid) => match prepare_live(&config, sid) {
            Ok(p) => Some(p),
            Err(msg) => {
                eprintln!("lens-app: {msg}");
                process::exit(1);
            }
        },
        None => None,
    };

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        register_keybindings(cx);

        let clock = Arc::new(WallUiClock) as Arc<dyn UiClock>;
        let fleet = FleetStore::new_live(clock, cx);

        cx.open_window(WindowOptions::default(), move |window, cx| {
            if let Some(prep) = live_prep {
                fleet.update(cx, |fleet, cx| {
                    if let Err(e) = fleet.spawn_live_session(
                        &prep.conn,
                        &prep.client,
                        prep.session_id,
                        &prep.data_dir,
                        cx,
                    ) {
                        eprintln!("lens-app: spawn_live_session: {e}");
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

fn register_keybindings(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-.", BackToBoard, None)]);
}

fn prepare_live(config: &Config, session_id: &SessionId) -> Result<LivePrep, String> {
    std::fs::create_dir_all(&config.data_dir)
        .map_err(|e| format!("failed to create {}: {e}", config.data_dir.display()))?;

    let auth = auth_from_env();
    let conn_id = ConnectionId::new("lens-app");
    let conn = Connection::new(conn_id, config.base_url.clone(), auth);
    let client = Client::new(conn.clone()).map_err(|e| format!("connect failed: {e}"))?;

    let snap = client
        .sessions()
        .get(session_id, GetOpts::default())
        .map_err(|e| format!("failed to resolve session {session_id}: {e}"))?;

    seed_disk(&conn, session_id, &config.data_dir, &snap)?;

    Ok(LivePrep {
        conn,
        client,
        session_id: session_id.clone(),
        data_dir: config.data_dir.clone(),
    })
}

fn parse_config() -> Result<Config, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut base_url: Option<String> = None;
    let mut session_id: Option<SessionId> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut fleet_verify = false;
    let mut fleet_count: usize = 10;
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
                session_id = Some(SessionId::new(id));
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
            let sid = session_id.as_ref().map(|s| s.as_str()).unwrap_or("board");
            std::env::temp_dir().join(format!("lens-app-{sid}"))
        }
    });

    Ok(Config {
        base_url,
        session_id,
        data_dir,
        fleet_verify,
        fleet_count,
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
