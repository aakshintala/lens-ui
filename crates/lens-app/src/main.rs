mod fleet_verify;

use gpui::{App, AppContext, Application, WindowOptions};
use gpui_component::Root;
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{GetOpts, SessionSnapshot, SessionStatus};
use lens_client::{Auth, Client, Connection};
use lens_core::actor::ActorStores;
use lens_core::domain::ids::AgentId;
use lens_core::domain::scalars::SessionStatusValue;
#[cfg(feature = "demo")]
use lens_core::domain::scalars::{ErrorInfo, SessionLifecycle};
use lens_core::domain::session::SessionState;
#[cfg(feature = "demo")]
use lens_core::domain::usage::Cost;
use lens_core::persist::{
    BoardStore, ConnectionRecord, ControlStore, SqliteBoardStore, SqliteControlStore,
    SqliteTranscriptStore,
};
use lens_ui::board::{BoardReplica, BoardView};
#[cfg(feature = "demo")]
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

    #[cfg(feature = "demo")]
    if config.demo {
        run_demo();
        return;
    }
    #[cfg(not(feature = "demo"))]
    let _ = &config.demo;

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

    let board_db = config.data_dir.join("lens.db");
    let mut board_store: Option<Box<dyn BoardStore + Send>> = SqliteBoardStore::open(&board_db)
        .ok()
        .map(|s| Box::new(s) as _);
    let conn_id = ConnectionId::new("lens-app");

    Application::new()
        .with_assets(lens_ui::assets::LensAssets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            lens_ui::theme::install_at_startup(cx);

            let clock = Arc::new(WallUiClock) as Arc<dyn UiClock>;
            let fleet = FleetStore::new_live(clock, cx);
            lens_ui::shortcuts::register(&fleet, cx);

            let window_options = WindowOptions {
                // Transparent native titlebar + an in-app themed TitleBar (rendered by
                // BoardView) so the dark theme's title bar replaces the white system strip.
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                ..Default::default()
            };
            cx.open_window(window_options, move |window, cx| {
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
                let replica = cx.new(|cx| {
                    BoardReplica::new(
                        board_store.take(),
                        board_db.clone(),
                        conn_id.clone(),
                        fleet.clone(),
                        cx,
                    )
                });
                let board = cx.new(|cx| {
                    BoardView::mount(
                        fleet.clone(),
                        replica.clone(),
                        placeholder_tab(cx),
                        None,
                        cx,
                    )
                });
                let any: gpui::AnyView = board.into();
                cx.new(|cx| Root::new(any, window, cx))
            })
            .ok();
            cx.activate(true);
        });
}

/// The preset demo session-id stems (one per wave state). MUST stay in sync with
/// `demo_preset_cards`' ids — the group seed and the fleet cards key on the same values.
#[cfg(feature = "demo")]
const DEMO_BASE_IDS: [&str; 8] = [
    "demo-needs-input",
    "demo-ready",
    "demo-working",
    "demo-failed",
    "demo-slept",
    "demo-neutral",
    "demo-scheduled",
    "demo-awaiting-review",
];

/// Every demo session id that `demo_cards` will create (7 stems × `LENS_DEMO_N` replicas,
/// same scheme: rep 0 = stem, rep>0 = `{stem}-r{rep}`).
#[cfg(feature = "demo")]
fn demo_session_ids() -> Vec<String> {
    let n = std::env::var("LENS_DEMO_N")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1);
    let mut ids = Vec::with_capacity(DEMO_BASE_IDS.len() * n);
    for rep in 0..n {
        for stem in DEMO_BASE_IDS {
            ids.push(if rep == 0 {
                stem.to_string()
            } else {
                format!("{stem}-r{rep}")
            });
        }
    }
    ids
}

/// Open a temp board store and seed TWO adjacent, distinctly-colored 2×2 groups over the
/// first 8 demo stems (conn `lens-app`, matching the replica) so the loaded board renders
/// B-3 group chrome AND exercises two group tiles meeting in the inter-tile gap:
/// - "Demo group A" (blue, 2×2): the loud four — needs-input, ready, working, failed.
/// - "Demo group B" (orange, 2×1): a quiet pair — slept, neutral.
/// - loose (no group): scheduled + awaiting-review — so the board exercises the grouped +
///   loose mix (loose tiles hole-backfill beside/below the groups).
///
/// Adjacency is what makes the on-device check meaningful: now that the ring-gutter (12) >
/// half the inter-tile gap (8), two group tint boxes overlap ~8px in the seam. At
/// `LENS_DEMO_N>1` the extra replicas reconcile loose, still exercising group/loose at scale.
/// Best-effort: any store failure just yields `None` → the demo board still renders loose.
#[cfg(feature = "demo")]
fn seed_demo_groups(
    db: &std::path::Path,
    conn: &ConnectionId,
) -> Option<Box<dyn BoardStore + Send>> {
    use lens_core::domain::board::{DEFAULT_BOARD_ID, PlacementTarget};
    use lens_core::domain::ids::{BoardId, SessionId};

    let store = SqliteBoardStore::open(db).ok()?;
    let board = BoardId::new(DEFAULT_BOARD_ID);
    let ids = demo_session_ids();

    let groups: [(&str, &str, &[String]); 2] = [
        ("Demo group A", "blue", &ids[..ids.len().min(4)]),
        // Group B trimmed to 2 members (slept, neutral) so the board also exercises the
        // grouped + loose mix: scheduled and awaiting-review are placed loose below.
        (
            "Demo group B",
            "orange",
            if ids.len() > 4 {
                &ids[4..ids.len().min(6)]
            } else {
                &[]
            },
        ),
    ];
    for (ordinal, (name, color, slice)) in groups.iter().enumerate() {
        if slice.is_empty() {
            continue;
        }
        let Ok(group) = store.create_group(&board, None, ordinal as i32, name) else {
            continue;
        };
        let _ = store.set_color(&group, color);
        for (i, sid) in slice.iter().enumerate() {
            let _ = store.place_session(
                conn,
                &SessionId::new(sid.clone()),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(group.clone()),
                    ordinal: Some(i as i32),
                },
            );
        }
    }
    // Loose cards: the remaining stems (scheduled, awaiting-review) placed at board
    // top-level (no parent group), ordinals after the two groups — a grouped + loose mix.
    for (i, sid) in ids.iter().enumerate().skip(6).take(2) {
        let _ = store.place_session(
            conn,
            &SessionId::new(sid.clone()),
            &PlacementTarget {
                board_id: Some(board.clone()),
                parent_item_id: None,
                ordinal: Some((groups.len() + (i - 6)) as i32),
            },
        );
    }
    Some(Box::new(store) as _)
}

/// `--demo`: paint six cards in the six wave states (no live server needed) so the
/// status language is visible at a glance. Cards carry no poller/commands — clicking
/// one still toggles focus (and suppresses that card's glow while focused).
#[cfg(feature = "demo")]
fn run_demo() {
    // A held TempDir (auto-cleaned on app exit; no PID-reuse stale data). Seed the groups
    // BEFORE Application::run (compliant — no cx.new SQLite) so B-3 group chrome renders
    // live over the demo cards; extra replicas reconcile loose.
    let demo_dir = tempfile::tempdir().expect("demo tempdir");
    let demo_db = demo_dir.path().join("board.db");
    let demo_conn = ConnectionId::new("lens-app");
    let mut demo_store = seed_demo_groups(&demo_db, &demo_conn);

    Application::new()
        .with_assets(lens_ui::assets::LensAssets)
        .run(move |cx: &mut App| {
            let _demo_dir = &demo_dir; // hold the TempDir for the app's lifetime
            gpui_component::init(cx);
            // Demo defaults to the dark palette; `LENS_THEME=light` still overrides.
            lens_ui::theme::install_at_startup_with_default(gpui_component::ThemeMode::Dark, cx);

            let clock = Arc::new(WallUiClock) as Arc<dyn UiClock>;
            let now = clock.now_millis();
            let fleet = FleetStore::new_live(clock, cx);
            lens_ui::shortcuts::register(&fleet, cx);

            // Size the demo window so the 8 cards land as a centered 4×2 grid
            // (4×280 card + 3×28 gap + 56 padding + 48 nav rail ≈ 1300px wide).
            let mut bounds =
                gpui::Bounds::centered(None, gpui::size(gpui::px(1340.0), gpui::px(860.0)), cx);
            // Stagger + title so two demo windows can be told apart in an A/B (LENS_DEMO_LABEL).
            if let Some(dx) = std::env::var("LENS_DEMO_DX")
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
            {
                bounds.origin.x += gpui::px(dx);
                bounds.origin.y += gpui::px(dx);
            }
            let window_options = WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                // Transparent native titlebar; BoardView renders the in-app themed TitleBar
                // so the dark strip replaces the white system titlebar (label dropped — the
                // transparent bar shows no OS title anyway).
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                ..Default::default()
            };
            cx.open_window(window_options, move |window, cx| {
                fleet.update(cx, |f, cx| {
                    for card in demo_cards(now) {
                        let id = card.session_id.clone();
                        let entity = cx.new(|_| card);
                        f.cards.insert(id, entity);
                    }
                    cx.notify();
                });
                let replica = cx.new(|cx| {
                    BoardReplica::new(
                        demo_store.take(),
                        demo_db.clone(),
                        demo_conn.clone(),
                        fleet.clone(),
                        cx,
                    )
                });
                let board = cx.new(|cx| {
                    BoardView::mount(
                        fleet.clone(),
                        replica.clone(),
                        placeholder_tab(cx),
                        None,
                        cx,
                    )
                });
                let card_views = board.read(cx).card_views_for_test().clone();
                lens_ui::card::spawn_demo_paint_instrumentation(&card_views, cx);
                let any: gpui::AnyView = board.into();
                cx.new(|cx| Root::new(any, window, cx))
            })
            .ok();
            cx.activate(true);
        });
}

/// Eight preset cards (one per wave state), replicated `LENS_DEMO_N` times (default 1).
#[cfg(feature = "demo")]
fn demo_cards(now: i64) -> Vec<SessionCard> {
    let replicas = std::env::var("LENS_DEMO_N")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1);
    let presets = demo_preset_cards(now);
    let mut out = Vec::with_capacity(presets.len() * replicas);
    for rep in 0..replicas {
        for mut card in presets.iter().cloned() {
            if rep > 0 {
                card.session_id = SessionId::new(format!("{}-r{rep}", card.session_id.as_str()));
            }
            out.push(card);
        }
    }
    out
}

/// One replica of the eight wave-state demo cards.
#[cfg(feature = "demo")]
fn demo_preset_cards(now: i64) -> [SessionCard; 8] {
    let base = |id: &str, title: &str| {
        let mut c = SessionCard::new(SessionId::new(id));
        c.harness = Some("claude-sdk".into());
        c.llm_model = Some("opus".into());
        c.workspace = Some("lens".into());
        c.git_branch = Some("main".into());
        c.repos = vec![lens_ui::card::model::RepoRef {
            name: "lens".into(),
            branch: Some("feat/multi-session".into()),
        }];
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
    // No activity line: real activity_summary is empty when waiting on the user (only live
    // tools/todos populate it) — the STATUS eyebrow carries the meaning.

    let mut ready = base("demo-ready", "Finished — reply with a greeting");
    ready.status = SessionStatusValue::Idle;
    ready.seeded = true;
    ready.seen_turn = 1;
    ready.last_completed_at = Some(now);

    let mut working = base("demo-working", "Refactor the session poller");
    working.status = SessionStatusValue::Running;
    working.activity_summary = "running the test suite…".into();
    working.last_total_tokens = Some(130_000); // ~65% → amber ctx bar

    let mut failed = base("demo-failed", "cargo build");
    failed.status = SessionStatusValue::Failed;
    failed.last_task_error = Some(ErrorInfo {
        code: "build_error".into(),
        message: "E0432: unresolved import".into(),
    });
    failed.last_total_tokens = Some(180_000); // ~90% → red ctx bar

    let mut slept = base("demo-slept", "Archived brainstorm");
    slept.status = SessionStatusValue::Idle;
    slept.lifecycle = SessionLifecycle::Slept;

    let neutral = base("demo-neutral", "Fresh session — no activity yet");

    let mut awaiting_review = base(
        "demo-awaiting-review",
        "Review the draft spec on the Canvas",
    );
    awaiting_review.status = SessionStatusValue::Idle;
    awaiting_review.awaiting_review = true;
    // No activity line (empty in real data — see needs_input above).
    // Multi-repo card: inline row collapses to `·+2`, hover reveals the full list.
    awaiting_review.repos = vec![
        lens_ui::card::model::RepoRef {
            name: "lens".into(),
            branch: Some("feat/multi-session".into()),
        },
        lens_ui::card::model::RepoRef {
            name: "omnigent".into(),
            branch: Some("main".into()),
        },
        lens_ui::card::model::RepoRef {
            name: "docs".into(),
            branch: None,
        },
    ];

    let mut scheduled = base("demo-scheduled", "Follow-up check scheduled");
    scheduled.status = SessionStatusValue::Idle;
    // Wake far enough out that the demo card stays Scheduled through a visual pass
    // (short spans age past wake against wall-clock and decay to Idle mid-viewing).
    // The Scheduled activity line is the live countdown override — no static summary.
    // Default 45m so the card stays Scheduled through a full visual pass; override
    // with LENS_DEMO_WAKE_SECS (e.g. =60) to watch the countdown ring visibly deplete.
    let wake_secs = std::env::var("LENS_DEMO_WAKE_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(45 * 60);
    scheduled.scheduled_wake_at = Some(now + wake_secs * 1000);
    scheduled.scheduled_started_at = Some(now);

    [
        needs_input,
        ready,
        working,
        failed,
        slept,
        neutral,
        awaiting_review,
        scheduled,
    ]
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
    #[cfg(feature = "demo")]
    let mut demo = false;
    #[cfg(not(feature = "demo"))]
    let demo = false;
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
            #[cfg(feature = "demo")]
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
