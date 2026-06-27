//! CLI harness wrapper: spawn an interactive omnigent session, subscribe to its
//! SSE stream immediately, and write a golden-corpus capture on exit.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::SessionFilter;
use lens_client::{Auth, Client, Connection};

const POLL_INTERVAL: Duration = Duration::from_millis(150);
const POLL_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REST_TIMEOUT: Duration = Duration::from_secs(30);

struct Config {
    base_url: url::Url,
    out_prefix: PathBuf,
    harness: Vec<String>,
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("lens_capture: {msg}");
        process::exit(1);
    }
}

fn die_usage(msg: impl std::fmt::Display) -> ! {
    eprintln!("lens_capture: {msg}");
    process::exit(2);
}

fn run() -> Result<(), String> {
    let config = parse_config();
    let auth = auth_from_env();
    let conn = Connection::new(ConnectionId::new("lens-capture"), config.base_url, auth);

    let client = Client::new(conn.clone()).map_err(|e| format!("connect failed: {e}"))?;

    let known = snapshot_session_ids(&client)?;
    let harness_cmd = config.harness.join(" ");
    eprintln!("spawning harness: {harness_cmd}");

    let mut child = Command::new(&config.harness[0])
        .args(&config.harness[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to spawn `{harness_cmd}`: {e}"))?;

    let session_id = wait_for_new_session(&client, &known, &mut child)?;

    let stream_path = path_with_suffix(&config.out_prefix, ".stream.sse");
    let snapshot_path = path_with_suffix(&config.out_prefix, ".snapshot.json");
    let items_path = path_with_suffix(&config.out_prefix, ".items.json");

    ensure_parent_dirs(&config.out_prefix)?;

    eprintln!(
        "capturing session {} → {}",
        session_id,
        stream_path.display()
    );

    let stop = Arc::new(AtomicBool::new(false));
    let capture_conn = conn.clone();
    let capture_sid = session_id.clone();
    let capture_path = stream_path.clone();
    let stop_for_thread = Arc::clone(&stop);
    // Surface a stream-subscribe failure (404/auth/etc.) — otherwise it would
    // silently leave a 0-byte capture and only show up as "0 bytes" in the summary.
    let _capture_handle = thread::spawn(move || {
        if let Err(e) = capture_stream(capture_conn, &capture_sid, &capture_path, stop_for_thread) {
            eprintln!("lens_capture: capture thread: {e}");
        }
    });

    let status = child
        .wait()
        .map_err(|e| format!("failed to wait on harness: {e}"))?;
    stop.store(true, Ordering::Relaxed);

    // A non-zero harness exit (incl. Ctrl-C quit) is NOT a capture failure — the
    // stream was captured regardless. Warn, but still write artifacts and exit 0.
    if !status.success() {
        eprintln!(
            "harness exited with {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        );
    }

    let sid_path = format!("/v1/sessions/{session_id}");
    let snapshot_bytes = fetch_bytes(
        &conn,
        &sid_path,
        &[("include_items", "true"), ("include_liveness", "true")],
    )?;
    write_file(&snapshot_path, &snapshot_bytes)?;

    let items_bytes = fetch_bytes(&conn, &format!("{sid_path}/items"), &[])?;
    write_file(&items_path, &items_bytes)?;

    let stream_bytes = fs::read(&stream_path)
        .map_err(|e| format!("failed to read {} for summary: {e}", stream_path.display()))?;

    let frame_count = count_sse_frames(&stream_bytes);
    print_summary(
        &stream_path,
        stream_bytes.len(),
        frame_count,
        &snapshot_path,
        snapshot_bytes.len(),
        &items_path,
        items_bytes.len(),
    );

    Ok(())
}

fn parse_config() -> Config {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        // Bare invocation is a usage error (exit 2); `--help` below exits 0.
        die_usage("missing harness command (try --help)");
    }

    let mut url: Option<String> = None;
    let mut out_prefix = PathBuf::from("./capture");
    let mut harness_start = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            "--url" => {
                i += 1;
                url = Some(next_flag_value(&args, i, "--url"));
                i += 1;
            }
            "--out" => {
                i += 1;
                out_prefix = PathBuf::from(next_flag_value(&args, i, "--out"));
                i += 1;
            }
            "--" => {
                harness_start = Some(i + 1);
                break;
            }
            arg if arg.starts_with('-') => {
                die_usage(format!("unknown flag: {arg}"));
            }
            _ => {
                harness_start = Some(i);
                break;
            }
        }
    }

    let harness_start = harness_start.unwrap_or_else(|| die_usage("missing harness command"));
    let harness: Vec<String> = args[harness_start..].to_vec();
    if harness.is_empty() {
        die_usage("missing harness command");
    }

    let url = url
        .or_else(|| std::env::var("LENS_OMNIGENT_URL").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            die_usage("omnigent base URL required: pass --url URL or set LENS_OMNIGENT_URL");
        });

    let base_url = url
        .parse::<url::Url>()
        .unwrap_or_else(|e| die_usage(format!("invalid base URL `{url}`: {e}")));

    Config {
        base_url,
        out_prefix,
        harness,
    }
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
lens_capture — capture an omnigent session's raw SSE stream while driving a harness interactively

Usage:
  lens_capture [--url URL] [--out PREFIX] [--] <harness cmd...>

Examples:
  lens_capture omnigent claude
  lens_capture --out docs/spikes/captures/2026-06-27-sessionstore/work omnigent codex

Environment:
  LENS_OMNIGENT_URL    omnigent base URL (required if --url omitted)
  LENS_OMNIGENT_TOKEN  optional bearer token

Output files (PREFIX defaults to ./capture):
  <PREFIX>.stream.sse     raw SSE bytes (write-through while the harness runs)
  <PREFIX>.snapshot.json  final GET /v1/sessions/{{id}}?include_items=true&include_liveness=true
  <PREFIX>.items.json     final GET /v1/sessions/{{id}}/items

The harness is spawned with inherited stdio. A background thread subscribes to the
session SSE stream as soon as a new session id appears — before you send the first
prompt — because omnigent creates the session during harness startup."
    );
}

fn auth_from_env() -> Auth {
    match std::env::var("LENS_OMNIGENT_TOKEN") {
        Ok(token) if !token.is_empty() => Auth::Bearer { token },
        _ => Auth::None,
    }
}

fn snapshot_session_ids(client: &Client) -> Result<HashSet<String>, String> {
    let list = client
        .sessions()
        .list(&SessionFilter::new())
        .map_err(|e| format!("failed to list sessions: {e}"))?;
    Ok(list.data.iter().map(|s| s.id().to_string()).collect())
}

fn wait_for_new_session(
    client: &Client,
    known: &HashSet<String>,
    child: &mut std::process::Child,
) -> Result<SessionId, String> {
    let deadline = Instant::now() + POLL_TIMEOUT;

    loop {
        if let Some(id) = find_first_new_session(client, known)? {
            return Ok(id);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into());
                return Err(format!(
                    "harness exited (status {code}) before a new session appeared; \
                     ensure the harness creates an omnigent session on startup"
                ));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("failed to poll harness: {e}")),
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out after {}s waiting for a new session id",
                POLL_TIMEOUT.as_secs()
            ));
        }

        thread::sleep(POLL_INTERVAL);
    }
}

/// First session id absent from the pre-spawn snapshot. Sub-agent sessions may
/// also appear; we take the earliest new row as the harness session heuristic.
fn find_first_new_session(
    client: &Client,
    known: &HashSet<String>,
) -> Result<Option<SessionId>, String> {
    let list = client
        .sessions()
        .list(&SessionFilter::new())
        .map_err(|e| format!("failed to list sessions: {e}"))?;

    Ok(list
        .data
        .iter()
        .find(|s| !known.contains(&s.id().to_string()))
        .map(|s| s.id().clone()))
}

fn path_with_suffix(prefix: &Path, suffix: &str) -> PathBuf {
    let mut path = prefix.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn ensure_parent_dirs(prefix: &Path) -> Result<(), String> {
    if let Some(parent) = prefix.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    Ok(())
}

/// Raw SSE GET on a dedicated thread. Chunks are written straight to disk so a
/// hard exit still leaves a usable partial corpus.
fn capture_stream(
    conn: Connection,
    session_id: &SessionId,
    stream_path: &Path,
    stop: Arc<AtomicBool>,
) -> Result<u64, String> {
    let mut file = File::create(stream_path).map_err(|e| {
        format!(
            "failed to create stream file {}: {e}",
            stream_path.display()
        )
    })?;

    let http = reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let path = format!("/v1/sessions/{session_id}/stream");
    let url = conn
        .url(&path)
        .map_err(|e| format!("bad stream url: {e}"))?;
    let resp = conn
        .auth
        .apply(http.get(url))
        .send()
        .map_err(|e| format!("stream GET failed: {e}"))?;
    let mut resp = resp
        .error_for_status()
        .map_err(|e| format!("stream GET error status: {e}"))?;

    let mut buf = [0u8; 8192];
    let mut total = 0u64;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match resp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                file.write_all(&buf[..n])
                    .map_err(|e| format!("stream write failed: {e}"))?;
                total += u64::try_from(n).unwrap_or(0);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("stream read failed: {e}")),
        }
    }

    if let Err(e) = file.flush() {
        eprintln!("warning: stream flush failed: {e}");
    }

    Ok(total)
}

fn fetch_bytes(conn: &Connection, path: &str, query: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let http = reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REST_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let url = conn
        .url(path)
        .map_err(|e| format!("bad url for {path}: {e}"))?;
    let mut rb = http.get(url);
    if !query.is_empty() {
        rb = rb.query(query);
    }

    let resp = conn
        .auth
        .apply(rb)
        .send()
        .map_err(|e| format!("GET {path} failed: {e}"))?;
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("GET {path} error status: {e}"))?;

    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("GET {path} body read failed: {e}"))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

fn count_sse_frames(bytes: &[u8]) -> usize {
    bytes.windows(2).filter(|w| *w == b"\n\n").count()
}

fn print_summary(
    stream_path: &Path,
    stream_bytes: usize,
    frame_count: usize,
    snapshot_path: &Path,
    snapshot_bytes: usize,
    items_path: &Path,
    items_bytes: usize,
) {
    println!("capture complete:");
    println!(
        "  {} — {stream_bytes} bytes, ~{frame_count} SSE frames",
        stream_path.display()
    );
    println!("  {} — {snapshot_bytes} bytes", snapshot_path.display());
    println!("  {} — {items_bytes} bytes", items_path.display());
}
