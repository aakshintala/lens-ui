//! lens-drive — headless JSON-lines actor driver.
//!
//! **Bright line:** lens-drive dumps state as JSON; it never renders. No markdown,
//! no transcript layout, no virtualization. Rendering belongs in `lens-ui`.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Receiver;
use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::{GetOpts, SessionSnapshot, SessionStatus};
use lens_client::stream::EventStream;
use lens_client::{Auth, Client, Connection};
use lens_core::actor::{
    ActorFeed, ActorOutcome, ActorStores, ActorTransport, ClientSessionApi, CommandOutcome,
    FleetScheduler, FleetSchedulerError, ParkReason, SessionCommand, TerminalResourceSignal,
};
use lens_core::clock::Clock;
use lens_core::domain::ids::AgentId;
use lens_core::domain::scalars::SessionStatusValue;
use lens_core::domain::session::SessionState;
use lens_core::persist::{
    ConnectionRecord, ControlStore, SqliteControlStore, SqliteTranscriptStore,
};
use lens_core::reduce::StreamUpdate;
use serde_json::{Value, json};

struct Config {
    base_url: url::Url,
    session_id: SessionId,
    script: Option<PathBuf>,
    data_dir: PathBuf,
}

struct SnapshotGate {
    waiting: AtomicBool,
    done: crossbeam_channel::Sender<()>,
}

impl SnapshotGate {
    fn new() -> (Self, crossbeam_channel::Receiver<()>) {
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        (
            Self {
                waiting: AtomicBool::new(false),
                done: done_tx,
            },
            done_rx,
        )
    }

    fn arm(&self) {
        self.waiting.store(true, Ordering::SeqCst);
    }

    fn signal_if_waiting(&self) {
        if self.waiting.swap(false, Ordering::SeqCst) {
            let _ = self.done.try_send(());
        }
    }
}

struct StreamBridge {
    stream: Arc<EventStream>,
    forwarder: Option<JoinHandle<()>>,
}

impl StreamBridge {
    fn shutdown(&mut self) {
        self.stream.stop();
        if let Some(h) = self.forwarder.take() {
            let _ = h.join();
        }
    }
}

impl Drop for StreamBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Live wall clock for the driver — reads `SystemTime` on each call.
struct WallClock;

impl Clock for WallClock {
    fn now_millis(&self) -> i64 {
        wall_clock_millis()
    }
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("lens-drive: {msg}");
        process::exit(1);
    }
}

fn die_usage(msg: impl std::fmt::Display) -> ! {
    eprintln!("lens-drive: {msg}");
    process::exit(2);
}

fn run() -> Result<(), String> {
    let config = parse_config()?;
    let auth = auth_from_env();
    let conn_id = ConnectionId::new("lens-drive");
    let conn = Connection::new(conn_id.clone(), config.base_url.clone(), auth);
    let client = Client::new(conn.clone()).map_err(|e| format!("connect failed: {e}"))?;

    fs::create_dir_all(&config.data_dir)
        .map_err(|e| format!("failed to create {}: {e}", config.data_dir.display()))?;

    let snap = client
        .sessions()
        .get(&config.session_id, GetOpts::default())
        .map_err(|e| format!("failed to resolve session {}: {e}", config.session_id))?;

    seed_disk(&conn, &config.session_id, &config.data_dir, &snap)?;

    let (feed_tx, feed_rx) = async_channel::bounded(64);
    let (snap_gate, snap_done) = SnapshotGate::new();
    let snap_gate = Arc::new(snap_gate);
    let stop = Arc::new(AtomicBool::new(false));

    let mut scheduler = FleetScheduler::new();
    let mut stream_bridge = attach_actor(
        &conn,
        &client,
        &config.session_id,
        &config.data_dir,
        &mut scheduler,
        feed_tx.clone(),
    )?;

    let session_id = config.session_id.clone();
    let scheduler = Arc::new(std::sync::Mutex::new(scheduler));

    let outcome_scheduler = Arc::clone(&scheduler);
    let outcome_session = session_id.clone();
    let outcome_stop = Arc::clone(&stop);
    let outcome_handle = thread::spawn(move || {
        drain_outcomes(&outcome_scheduler, &outcome_session, &outcome_stop);
    });

    let update_gate = Arc::clone(&snap_gate);
    let update_stop = Arc::clone(&stop);
    let update_handle = thread::spawn(move || drain_updates(feed_rx, &update_gate, &update_stop));

    let input: Box<dyn BufRead> = match &config.script {
        Some(path) => {
            Box::new(BufReader::new(File::open(path).map_err(|e| {
                format!("failed to open {}: {e}", path.display())
            })?))
        }
        None => Box::new(BufReader::new(io::stdin())),
    };

    for line in input.lines() {
        let line = line.map_err(|e| format!("read error: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_command(line) {
            Command::Send { text } => send_command(&scheduler, &session_id, text)?,
            Command::Sleep => sleep_command(&scheduler, &session_id)?,
            Command::Reconnect => {
                stream_bridge = reconnect_command(
                    &conn,
                    &client,
                    &session_id,
                    &config.data_dir,
                    &scheduler,
                    feed_tx.clone(),
                    stream_bridge,
                )?;
            }
            Command::Snapshot => snapshot_command(&scheduler, &session_id, &snap_gate, &snap_done)?,
            Command::Stop => {
                stop_command(&scheduler, &session_id, stream_bridge)?;
                stop.store(true, Ordering::Relaxed);
                outcome_handle.join().ok();
                update_handle.join().ok();
                return Ok(());
            }
        }
    }

    stop_command(&scheduler, &session_id, stream_bridge)?;
    stop.store(true, Ordering::Relaxed);
    outcome_handle.join().ok();
    update_handle.join().ok();
    Ok(())
}

enum Command {
    Send { text: String },
    Sleep,
    Reconnect,
    Stop,
    Snapshot,
}

fn parse_command(line: &str) -> Command {
    if let Some(text) = line.strip_prefix("send ") {
        return Command::Send {
            text: text.to_string(),
        };
    }
    match line {
        "sleep" => Command::Sleep,
        "reconnect" => Command::Reconnect,
        "stop" => Command::Stop,
        "snapshot" => Command::Snapshot,
        other => die_usage(format!("unknown command: {other}")),
    }
}

fn start_stream_bridge(
    stream: EventStream,
) -> (
    StreamBridge,
    Receiver<lens_client::stream::ServerStreamEvent>,
) {
    const EVENT_CHANNEL_BOUND: usize = 1024;
    let (events_tx, events_rx) = crossbeam_channel::bounded(EVENT_CHANNEL_BOUND);
    let stream = Arc::new(stream);
    let reader = Arc::clone(&stream);
    let forwarder = thread::spawn(move || {
        while let Some(ev) = reader.recv() {
            if events_tx.send(ev).is_err() {
                break;
            }
        }
    });
    (
        StreamBridge {
            stream,
            forwarder: Some(forwarder),
        },
        events_rx,
    )
}

fn attach_actor(
    conn: &Connection,
    client: &Client,
    session_id: &SessionId,
    data_dir: &Path,
    scheduler: &mut FleetScheduler,
    feed_tx: async_channel::Sender<ActorFeed>,
) -> Result<StreamBridge, String> {
    let stream = client
        .sessions()
        .stream(session_id)
        .map_err(|e| format!("stream subscribe failed: {e}"))?;
    let (bridge, events) = start_stream_bridge(stream);
    let stores = open_stores(data_dir, &conn.id, session_id)?;
    let api = actor_api(conn)?;
    let clock = make_clock();
    scheduler
        .reconnect(
            &conn.id,
            session_id,
            events,
            feed_tx,
            lens_core::actor::OutputMode::Detailed,
            stores,
            clock,
            api,
        )
        .map_err(scheduler_err)?;
    Ok(bridge)
}

fn reconnect_command(
    conn: &Connection,
    client: &Client,
    session_id: &SessionId,
    data_dir: &Path,
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    feed_tx: async_channel::Sender<ActorFeed>,
    mut old_bridge: StreamBridge,
) -> Result<StreamBridge, String> {
    old_bridge.shutdown();
    let stream = client
        .sessions()
        .stream(session_id)
        .map_err(|e| format!("stream subscribe failed: {e}"))?;
    let (bridge, events) = start_stream_bridge(stream);
    let stores = open_stores(data_dir, &conn.id, session_id)?;
    let api = actor_api(conn)?;
    let clock = make_clock();
    let live = {
        let mut sched = scheduler
            .lock()
            .map_err(|e| format!("scheduler lock: {e}"))?;
        sched
            .reconnect(
                &conn.id,
                session_id,
                events,
                feed_tx,
                lens_core::actor::OutputMode::Detailed,
                stores,
                clock,
                api,
            )
            .map_err(scheduler_err)?
    };
    print_reconnect_line(live);
    Ok(bridge)
}

fn send_command(
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
    text: String,
) -> Result<(), String> {
    let sched = scheduler
        .lock()
        .map_err(|e| format!("scheduler lock: {e}"))?;
    let handle = sched
        .handle(session_id)
        .ok_or_else(|| format!("session {session_id} is not running (parked?)"))?;
    handle
        .commands
        .send(SessionCommand::Send {
            text,
            model_override: None,
        })
        .map_err(|_| "failed to send command to actor".to_string())
}

fn sleep_command(
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
) -> Result<(), String> {
    let mut sched = scheduler
        .lock()
        .map_err(|e| format!("scheduler lock: {e}"))?;
    sched.sleep(session_id).map_err(scheduler_err)
}

fn snapshot_command(
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
    gate: &SnapshotGate,
    done: &crossbeam_channel::Receiver<()>,
) -> Result<(), String> {
    gate.arm();
    {
        let sched = scheduler
            .lock()
            .map_err(|e| format!("scheduler lock: {e}"))?;
        let handle = sched
            .handle(session_id)
            .ok_or_else(|| format!("session {session_id} is not running (parked?)"))?;
        handle
            .commands
            .send(SessionCommand::Promote)
            .map_err(|_| "failed to send Promote".to_string())?;
    }
    done.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "timed out waiting for snapshot Rebased".to_string())?;
    Ok(())
}

fn stop_command(
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
    mut bridge: StreamBridge,
) -> Result<(), String> {
    bridge.shutdown();
    let mut sched = scheduler
        .lock()
        .map_err(|e| format!("scheduler lock: {e}"))?;
    if let Some(handle) = sched.take_handle(session_id) {
        let _ = handle.commands.send(SessionCommand::Stop);
        while let Ok(outcome) = handle.outcomes.recv_blocking() {
            process_outcome(&outcome, scheduler, session_id);
        }
        handle.join_exited();
    }
    Ok(())
}

fn process_outcome(
    outcome: &ActorOutcome,
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
) {
    if let ActorOutcome::Parked { reason } = outcome
        && let Ok(mut sched) = scheduler.lock()
    {
        sched.mark_parked(session_id, *reason);
    }
    print_outcome_line(outcome);
}

fn drain_outcomes(
    scheduler: &Arc<std::sync::Mutex<FleetScheduler>>,
    session_id: &SessionId,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        let outcome = {
            let sched = match scheduler.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            let Some(handle) = sched.handle(session_id) else {
                drop(sched);
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            match handle.outcomes.try_recv() {
                Ok(o) => o,
                Err(async_channel::TryRecvError::Empty)
                | Err(async_channel::TryRecvError::Closed) => {
                    drop(sched);
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
            }
        };
        process_outcome(&outcome, scheduler, session_id);
    }
}

fn drain_updates(rx: async_channel::Receiver<ActorFeed>, gate: &SnapshotGate, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match rx.try_recv() {
            Ok(ActorFeed::Detailed(update)) => match update {
                StreamUpdate::Rebased(state) => {
                    print_state_line(&state);
                    gate.signal_if_waiting();
                }
                StreamUpdate::TranscriptAdvanced { committed_ordinal } => {
                    print_transcript_advanced_line(committed_ordinal)
                }
                _ => {}
            },
            Ok(ActorFeed::Summary(_)) => {
                // lens-drive is Detailed-only; ignore Summary frames if any appear.
            }
            Err(async_channel::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(async_channel::TryRecvError::Closed) => return,
        }
    }
}

fn parse_config() -> Result<Config, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        die_usage("missing arguments (try --help)");
    }

    let mut base_url: Option<String> = None;
    let mut session_id: Option<SessionId> = None;
    let mut script: Option<PathBuf> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            "--base-url" => {
                i += 1;
                base_url = Some(next_flag_value(&args, i, "--base-url"));
                i += 1;
            }
            "--session" => {
                i += 1;
                let id = next_flag_value(&args, i, "--session");
                session_id = Some(SessionId::new(id));
                i += 1;
            }
            "--script" => {
                i += 1;
                script = Some(PathBuf::from(next_flag_value(&args, i, "--script")));
                i += 1;
            }
            "--data-dir" => {
                i += 1;
                data_dir = Some(PathBuf::from(next_flag_value(&args, i, "--data-dir")));
                i += 1;
            }
            arg if arg.starts_with('-') => die_usage(format!("unknown flag: {arg}")),
            _ => die_usage(format!("unexpected argument: {}", args[i])),
        }
    }

    let session_id = session_id.unwrap_or_else(|| die_usage("--session CONV_ID is required"));

    let base_url = base_url
        .or_else(|| std::env::var("LENS_OMNIGENT_URL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            die_usage("omnigent base URL required: pass --base-url URL or set LENS_OMNIGENT_URL");
        });

    let base_url = base_url
        .parse::<url::Url>()
        .unwrap_or_else(|e| die_usage(format!("invalid base URL `{base_url}`: {e}")));

    let data_dir = data_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("lens-drive-{}", session_id.as_str()))
    });

    Ok(Config {
        base_url,
        session_id,
        script,
        data_dir,
    })
}

fn next_flag_value(args: &[String], index: usize, flag: &str) -> String {
    args.get(index)
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| die_usage(format!("{flag} requires a value")))
}

fn print_help() {
    println!(
        "\
lens-drive — headless JSON-lines actor driver (dumps state, never renders)

Usage:
  lens-drive --base-url URL --session CONV_ID [--script PATH] [--data-dir PATH]

Commands (stdin or --script):
  send <text>    optimistic user message
  sleep          durable sleep
  reconnect      respawn via FleetScheduler::reconnect (prints live_status)
  snapshot       Promote and dump SessionState JSON
  stop           stop actor and exit

Environment:
  LENS_OMNIGENT_URL     default base URL when --base-url omitted
  LENS_OMNIGENT_TOKEN   optional bearer token"
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

fn make_clock() -> Box<dyn Clock + Send> {
    Box::new(WallClock)
}

fn actor_api(conn: &Connection) -> Result<Box<ClientSessionApi>, String> {
    let client = Client::new(conn.clone()).map_err(|e| format!("client handshake: {e}"))?;
    Ok(Box::new(ClientSessionApi::new(client)))
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
    seed_connection(&*stores.control, conn, now).map_err(|e| format!("seed connection: {e}"))?;
    seed_session(&*stores.control, &conn.id, snap, now)
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
        label: Some("lens-drive".into()),
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

fn scheduler_err(e: FleetSchedulerError) -> String {
    format!("scheduler: {e:?}")
}

fn session_status_json(status: Option<SessionStatus>) -> Value {
    status
        .map(|s| match s {
            SessionStatus::Idle => json!("idle"),
            SessionStatus::Running => json!("running"),
            SessionStatus::Failed => json!("failed"),
        })
        .unwrap_or(Value::Null)
}

fn print_reconnect_line(live_status: Option<SessionStatus>) {
    let line = json!({
        "kind": "reconnect",
        "live_status": session_status_json(live_status),
    });
    emit_line(&line);
}

fn print_outcome_line(outcome: &ActorOutcome) {
    let line = json!({
        "kind": "outcome",
        "outcome": actor_outcome_json(outcome),
    });
    emit_line(&line);
}

fn print_transcript_advanced_line(committed_ordinal: i64) {
    let line = json!({
        "kind": "transcript_advanced",
        "committed_ordinal": committed_ordinal,
    });
    emit_line(&line);
}

fn print_state_line(state: &SessionState) {
    let line = json!({
        "kind": "state",
        "state": state,
    });
    emit_line(&line);
}

fn emit_line(v: &Value) {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, v).expect("serialize JSON line");
    stdout.write_all(b"\n").expect("write newline");
    stdout.flush().ok();
}

fn actor_outcome_json(outcome: &ActorOutcome) -> Value {
    match outcome {
        ActorOutcome::Command(cmd) => command_outcome_json(cmd),
        ActorOutcome::PersistError { where_, message } => json!({
            "variant": "PersistError",
            "where": where_,
            "message": message,
        }),
        ActorOutcome::TransportChanged {
            transport,
            reconcile_in_flight,
        } => json!({
            "variant": "TransportChanged",
            "transport": transport_str(*transport),
            "reconcile_in_flight": reconcile_in_flight,
        }),
        ActorOutcome::Parked { reason } => json!({
            "variant": "Parked",
            "reason": park_reason_str(*reason),
        }),
        ActorOutcome::Slept => json!({"variant": "Slept"}),
        ActorOutcome::SleepDeclined => json!({"variant": "SleepDeclined"}),
        ActorOutcome::FeedConsumerGone => json!({"variant": "FeedConsumerGone"}),
        ActorOutcome::SendLost {
            lens_pending_id,
            content,
        } => json!({
            "variant": "SendLost",
            "lens_pending_id": lens_pending_id,
            "content": content,
        }),
        ActorOutcome::Superseded {
            target_conversation_id,
            reason,
        } => json!({
            "variant": "Superseded",
            "target_conversation_id": target_conversation_id,
            "reason": reason,
        }),
        ActorOutcome::TerminalResource(signal) => match signal {
            TerminalResourceSignal::Created {
                terminal_id,
                terminal_name,
                session_key,
                session_id,
            } => json!({
                "variant": "TerminalResource",
                "signal": "Created",
                "terminal_id": terminal_id,
                "terminal_name": terminal_name,
                "session_key": session_key,
                "session_id": session_id,
            }),
            TerminalResourceSignal::Deleted { terminal_id } => json!({
                "variant": "TerminalResource",
                "signal": "Deleted",
                "terminal_id": terminal_id,
            }),
        },
    }
}

fn command_outcome_json(outcome: &CommandOutcome) -> Value {
    match outcome {
        CommandOutcome::SendAccepted {
            lens_pending_id,
            ack,
        } => json!({
            "variant": "SendAccepted",
            "lens_pending_id": lens_pending_id,
            "ack": {
                "queued": ack.queued,
                "item_id": ack.item_id,
                "pending_id": ack.pending_id,
                "denied": ack.denied,
                "reason": ack.reason,
                "elicitation_id": ack.elicitation_id,
            },
        }),
        CommandOutcome::SendDenied {
            lens_pending_id,
            content,
            reason,
        } => json!({
            "variant": "SendDenied",
            "lens_pending_id": lens_pending_id,
            "content": content,
            "reason": reason,
        }),
        CommandOutcome::SendFailed {
            lens_pending_id,
            content,
            error,
        } => json!({
            "variant": "SendFailed",
            "lens_pending_id": lens_pending_id,
            "content": content,
            "error": error,
        }),
        CommandOutcome::SendPending { lens_pending_id } => json!({
            "variant": "SendPending",
            "lens_pending_id": lens_pending_id,
        }),
    }
}

fn park_reason_str(reason: ParkReason) -> &'static str {
    match reason {
        ParkReason::Unauthorized => "unauthorized",
        ParkReason::SessionFailed => "session_failed",
        ParkReason::RetriesExhausted => "retries_exhausted",
        ParkReason::Forbidden => "forbidden",
        ParkReason::NotFound => "not_found",
    }
}

fn transport_str(transport: ActorTransport) -> String {
    match transport {
        ActorTransport::Connected => "connected".into(),
        ActorTransport::Reconnecting => "reconnecting".into(),
    }
}
