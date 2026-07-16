//! Headless N-session fleet live gate — `lens-app --fleet-verify`.
//!
//! Exit codes: 0 = pass, 1 = gate failed, 2 = server unreachable.

use crate::{auth_from_env, seed_disk};
use gpui::{App, Application, Entity};
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{CreateSessionRequest, SessionSnapshot};
use lens_client::{Client, Connection};
use lens_ui::card::model::{ConnectionOverlay, SessionCard};
use lens_ui::clock::{UiClock, WallUiClock};
use lens_ui::fleet::store::FleetStore;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const WAIT_SUMMARY_MS: u64 = 15_000;
const WAIT_MODE_MS: u64 = 5_000;

pub struct FleetVerifyOptions {
    pub base_url: url::Url,
    pub count: usize,
    pub data_dir: PathBuf,
}

#[derive(Clone, Debug)]
struct CardProbe {
    seeded: bool,
    notify_count: u64,
    title: Option<String>,
    harness: Option<String>,
    overlay: ConnectionOverlay,
}

struct SessionPrep {
    conn: Connection,
    client: Client,
    sessions: Vec<(SessionId, SessionSnapshot, PathBuf)>,
}

pub fn run(opts: FleetVerifyOptions) -> i32 {
    let prep = match prepare(&opts) {
        Ok(p) => p,
        Err(code) => return code,
    };

    let exit = Rc::new(Cell::new(1i32));
    let exit_for_run = Rc::clone(&exit);
    let count = opts.count;

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        let clock = std::sync::Arc::new(WallUiClock) as std::sync::Arc<dyn UiClock>;
        let fleet = FleetStore::new_live(clock, cx);

        let mut attach_failures: Vec<String> = Vec::new();
        for (session_id, snap, data_dir) in &prep.sessions {
            if let Err(e) = std::fs::create_dir_all(data_dir) {
                attach_failures.push(format!(
                    "{}: create data dir {}: {e}",
                    session_id.as_str(),
                    data_dir.display()
                ));
                continue;
            }
            if let Err(e) = seed_disk(&prep.conn, session_id, data_dir, snap) {
                attach_failures.push(format!("{}: seed disk: {e}", session_id.as_str()));
                continue;
            }
            let spawn_result = fleet.update(cx, |fleet, cx| {
                fleet.spawn_live_session(&prep.conn, &prep.client, session_id.clone(), data_dir, cx)
            });
            match spawn_result {
                Ok(_) => {}
                Err(e) => attach_failures.push(format!("{}: spawn: {e}", session_id.as_str())),
            }
        }

        if !attach_failures.is_empty() {
            eprintln!("FLEET-VERIFY FAILED: attach errors:");
            for msg in &attach_failures {
                eprintln!("  {msg}");
            }
            exit_for_run.set(1);
            cx.quit();
            return;
        }

        let fleet = fleet.clone();
        let sessions = prep.sessions;
        cx.spawn(async move |cx| {
            let code = drive_gate(fleet, &sessions, count, cx).await;
            exit_for_run.set(code);
            let _ = cx.update(|app| app.quit());
        })
        .detach();
    });

    exit.get()
}

async fn drive_gate(
    fleet: Entity<FleetStore>,
    sessions: &[(SessionId, SessionSnapshot, PathBuf)],
    count: usize,
    cx: &mut gpui::AsyncApp,
) -> i32 {
    let ids: Vec<SessionId> = sessions.iter().map(|(id, _, _)| id.clone()).collect();

    if !wait_all_summary(&fleet, &ids, cx).await {
        eprintln!(
            "FLEET-VERIFY FAILED: not all {count} cards received Summary seed within {WAIT_SUMMARY_MS}ms"
        );
        print_session_report(&fleet, sessions, cx);
        return 1;
    }

    for (id, snap, _) in sessions {
        println!(
            "  ok   {} summary seed (title={:?}, harness={:?})",
            id.as_str(),
            snap.title(),
            snap.harness()
        );
    }

    // Promote/demote cycles: session 0 → blur → session 1 → blur → session 0.
    let cycles: [usize; 3] = [0, 1, 0];
    for &idx in &cycles {
        if idx >= ids.len() {
            continue;
        }
        let id = &ids[idx];
        let snap = &sessions[idx].1;

        let before = match probe_card(&fleet, id, cx) {
            Some(p) => p,
            None => {
                eprintln!("FLEET-VERIFY FAILED: card missing for {}", id.as_str());
                return 1;
            }
        };

        let before_notify = before.notify_count;

        let _ = fleet.update(cx, |fleet, cx| fleet.focus_session(id.clone(), cx));

        if !wait_promote(&fleet, id, snap, before_notify, cx).await {
            eprintln!(
                "FLEET-VERIFY FAILED: promote on {} did not produce Detailed/Rebased projection",
                id.as_str()
            );
            print_session_report(&fleet, sessions, cx);
            return 1;
        }
        println!("  ok   {} promote → Detailed", id.as_str());

        let blur_before_notify = probe_card(&fleet, id, cx)
            .map(|p| p.notify_count)
            .unwrap_or(0);
        let _ = fleet.update(cx, |fleet, cx| fleet.blur_to_board(cx));

        if !wait_demote(&fleet, id, blur_before_notify, cx).await {
            eprintln!(
                "FLEET-VERIFY FAILED: demote on {} did not restore Summary projection",
                id.as_str()
            );
            print_session_report(&fleet, sessions, cx);
            return 1;
        }
        println!("  ok   {} demote → Summary", id.as_str());
    }

    for id in &ids {
        if let Some(p) = probe_card(&fleet, id, cx)
            && p.overlay == ConnectionOverlay::Disconnected
        {
            eprintln!(
                "FLEET-VERIFY FAILED: {} stuck Disconnected (parked/wiring bug)",
                id.as_str()
            );
            return 1;
        }
    }

    println!(
        "FLEET-VERIFY PASSED: N={count} sessions attached, Summary seeds + promote/demote cycles OK"
    );
    print_session_report(&fleet, sessions, cx);
    0
}

fn card_looks_detailed(probe: &CardProbe, snap: &SessionSnapshot) -> bool {
    probe.seeded
        && probe.overlay != ConnectionOverlay::Disconnected
        && (probe.harness.is_some() || probe.title.is_some() || snap.title().is_some())
}

fn probe_card_on_app(fleet: &Entity<FleetStore>, id: &SessionId, app: &App) -> Option<CardProbe> {
    fleet.read(app).card(id).map(|entity| {
        let card = entity.read(app);
        card_probe(card)
    })
}

fn probe_card(
    fleet: &Entity<FleetStore>,
    id: &SessionId,
    cx: &mut gpui::AsyncApp,
) -> Option<CardProbe> {
    cx.update(|app| probe_card_on_app(fleet, id, app))
        .ok()
        .flatten()
}

fn card_probe(card: &SessionCard) -> CardProbe {
    CardProbe {
        seeded: card.seeded,
        notify_count: card.notify_count,
        title: card.title.clone(),
        harness: card.harness.clone(),
        overlay: card.connection_overlay,
    }
}

async fn wait_all_summary(
    fleet: &Entity<FleetStore>,
    ids: &[SessionId],
    cx: &mut gpui::AsyncApp,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(WAIT_SUMMARY_MS);
    while Instant::now() < deadline {
        let ready = cx
            .update(|app| {
                ids.iter().all(|id| {
                    probe_card_on_app(fleet, id, app).is_some_and(|p| {
                        p.seeded
                            && p.notify_count > 0
                            && p.overlay != ConnectionOverlay::Disconnected
                    })
                })
            })
            .unwrap_or(false);
        if ready {
            return true;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    false
}

async fn wait_promote(
    fleet: &Entity<FleetStore>,
    id: &SessionId,
    snap: &SessionSnapshot,
    before_notify: u64,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MODE_MS);
    while Instant::now() < deadline {
        let ok = cx
            .update(|app| {
                probe_card_on_app(fleet, id, app).is_some_and(|p| {
                    p.notify_count > before_notify && card_looks_detailed(&p, snap)
                })
            })
            .unwrap_or(false);
        if ok {
            return true;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    false
}

async fn wait_demote(
    fleet: &Entity<FleetStore>,
    id: &SessionId,
    before_notify: u64,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MODE_MS);
    while Instant::now() < deadline {
        let ok = cx
            .update(|app| {
                probe_card_on_app(fleet, id, app).is_some_and(|p| {
                    p.notify_count > before_notify
                        && p.seeded
                        && p.overlay != ConnectionOverlay::Disconnected
                })
            })
            .unwrap_or(false);
        if ok {
            return true;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    false
}

fn print_session_report(
    fleet: &Entity<FleetStore>,
    sessions: &[(SessionId, SessionSnapshot, PathBuf)],
    cx: &mut gpui::AsyncApp,
) {
    println!("--- per-session ---");
    for (id, snap, dir) in sessions {
        let line = probe_card(fleet, id, cx).map_or_else(
            || "missing".to_string(),
            |p| {
                format!(
                    "seeded={} notify={} title={:?} harness={:?} overlay={:?}",
                    p.seeded, p.notify_count, p.title, p.harness, p.overlay
                )
            },
        );
        println!(
            "  {} agent={} dir={} | {line}",
            id.as_str(),
            snap.agent_id(),
            dir.display()
        );
    }
}

fn prepare(opts: &FleetVerifyOptions) -> Result<SessionPrep, i32> {
    if let Err(e) = std::fs::create_dir_all(&opts.data_dir) {
        eprintln!(
            "FLEET-VERIFY FAILED: cannot create data dir {}: {e}",
            opts.data_dir.display()
        );
        return Err(1);
    }

    let auth = auth_from_env();
    let conn_id = ConnectionId::new("lens-fleet-verify");
    let conn = Connection::new(conn_id, opts.base_url.clone(), auth);

    let client = match Client::new(conn.clone()) {
        Ok(c) => c,
        Err(e) => {
            if e.is_transport() {
                eprintln!(
                    "FLEET-VERIFY: server unreachable at {} ({e})",
                    opts.base_url
                );
            } else {
                eprintln!("FLEET-VERIFY: handshake failed at {} ({e})", opts.base_url);
            }
            return Err(2);
        }
    };

    let agent_id = match resolve_agent_id(&client) {
        Ok(id) => id,
        Err(msg) => {
            eprintln!("FLEET-VERIFY: {msg}");
            return Err(if msg.contains("unreachable") { 2 } else { 1 });
        }
    };
    println!("fleet-verify: using agent {agent_id}");

    let mut created: Vec<(SessionId, SessionSnapshot, PathBuf)> = Vec::with_capacity(opts.count);
    for i in 0..opts.count {
        let snap = match client
            .sessions()
            .create(&CreateSessionRequest::new(agent_id.clone()))
        {
            Ok(s) => s,
            Err(e) => {
                if e.is_transport() {
                    eprintln!("FLEET-VERIFY: server unreachable while creating session {i} ({e})");
                    return Err(2);
                }
                eprintln!("FLEET-VERIFY FAILED: create session {i}: {e}");
                break;
            }
        };
        let sid = snap.id().clone();
        let dir = opts.data_dir.join(format!("session-{}", sid.as_str()));
        created.push((sid, snap, dir));
    }

    if created.len() < opts.count {
        eprintln!(
            "FLEET-VERIFY FAILED: created {}/{} sessions",
            created.len(),
            opts.count
        );
        return Err(1);
    }

    Ok(SessionPrep {
        conn,
        client,
        sessions: created,
    })
}

fn resolve_agent_id(client: &Client) -> Result<String, String> {
    let list = client.list_agents().map_err(|e| {
        if e.is_transport() {
            format!("server unreachable listing agents ({e})")
        } else {
            format!("list agents failed: {e}")
        }
    })?;
    list.data
        .first()
        .map(|a| a.id.clone())
        .ok_or_else(|| "no agents on server — cannot create sessions".into())
}

/// Help blurb appended to `lens-app --help`.
pub fn help_section() -> &'static str {
    "\
Fleet verify (merge gate):
  lens-app --fleet-verify [--count N] [--base-url URL] [--data-dir PATH]

  Creates N sessions (default 10), attaches each in Summary mode, exercises
  promote/demote cycles, and exits 0 only when all assertions pass.
  Exit 2 = server unreachable; exit 1 = gate failed."
}
