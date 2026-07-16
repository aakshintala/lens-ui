//! Throwaway omnigent terminal-attach capture harness (Spike B1).
//!
//! Dumps every WebSocket frame to JSONL for live contract verification.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const CAPTURE_SUBDIR: &str = "docs/spikes/captures/2026-07-15-pty-attach";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Default,
    Pty,
    Control,
}

impl Transport {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "default" => Ok(Self::Default),
            "pty" => Ok(Self::Pty),
            "control" => Ok(Self::Control),
            other => bail!("unknown transport {other:?}; expected pty|control|default"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Attach,
    Input,
    Resize,
    Reconnect,
    All,
}

impl Scenario {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "attach" => Ok(Self::Attach),
            "input" => Ok(Self::Input),
            "resize" => Ok(Self::Resize),
            "reconnect" => Ok(Self::Reconnect),
            "all" => Ok(Self::All),
            other => bail!("unknown scenario {other:?}; expected attach|input|resize|reconnect|all"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Input => "input",
            Self::Resize => "resize",
            Self::Reconnect => "reconnect",
            Self::All => "all",
        }
    }
}

#[derive(Debug)]
struct Config {
    base_url: String,
    token: Option<String>,
    session_id: Option<String>,
    terminal_id: Option<String>,
    terminal_name: String,
    terminal_session_key: String,
    transport: Transport,
    scenario: Scenario,
    capture_dir: PathBuf,
}

impl Config {
    fn from_env_and_args() -> Result<Self> {
        let mut transport = Transport::Control;
        let mut scenario = Scenario::All;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--transport" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--transport requires a value"))?;
                    transport = Transport::parse(&value)?;
                }
                "--scenario" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--scenario requires a value"))?;
                    scenario = Scenario::parse(&value)?;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other:?}; try --help"),
            }
        }

        let base_url = std::env::var("OMNIGENT_BASE_URL")
            .context("OMNIGENT_BASE_URL is required (e.g. http://127.0.0.1:8000)")?;

        Ok(Self {
            base_url,
            token: std::env::var("OMNIGENT_TOKEN").ok().filter(|s| !s.is_empty()),
            session_id: std::env::var("OMNIGENT_SESSION_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            terminal_id: std::env::var("OMNIGENT_TERMINAL_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            terminal_name: std::env::var("OMNIGENT_TERMINAL_NAME")
                .unwrap_or_else(|_| "shell".to_string()),
            terminal_session_key: std::env::var("OMNIGENT_TERMINAL_SESSION_KEY")
                .unwrap_or_else(|_| "main".to_string()),
            transport,
            scenario,
            capture_dir: capture_dir()?,
        })
    }
}

fn print_help() {
    println!(
        "\
terminal-attach — Spike B1 omnigent PTY-attach frame capture harness

USAGE:
    terminal-attach [--transport <pty|control|default>] [--scenario <name>]

FLAGS:
    --transport <pty|control|default>   WS attach transport query (default: control)
    --scenario <attach|input|resize|reconnect|all>   Scenario to run (default: all)

ENV:
    OMNIGENT_BASE_URL          Required REST base URL (e.g. http://127.0.0.1:8000)
    OMNIGENT_TOKEN             Optional Bearer token
    OMNIGENT_SESSION_ID        Optional existing session id
    OMNIGENT_TERMINAL_ID       Optional existing terminal id
    OMNIGENT_TERMINAL_NAME     Terminal name for POST create (default: shell)
    OMNIGENT_TERMINAL_SESSION_KEY  session_key for POST create (default: main)

See spikes/terminal-attach/README.md for scenario details.
"
    );
}

fn capture_dir() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("../../").join(CAPTURE_SUBDIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create capture dir {}", dir.display()))?;
    Ok(dir.canonicalize().unwrap_or(dir))
}

fn http_to_ws_base(http_base: &str) -> Result<String> {
    let trimmed = http_base.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        return Ok(format!("wss://{rest}"));
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        return Ok(format!("ws://{rest}"));
    }
    bail!("OMNIGENT_BASE_URL must start with http:// or https://; got {http_base}");
}

#[derive(Debug, Clone)]
struct TerminalIds {
    session_id: String,
    terminal_id: String,
}

async fn rest_client(config: &Config) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    if let Some(token) = &config.token {
        let value = format!("Bearer {token}");
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&value)
                .context("invalid OMNIGENT_TOKEN for Authorization header")?,
        );
    }
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .build()?)
}

async fn ensure_terminal(config: &Config, client: &reqwest::Client) -> Result<TerminalIds> {
    if let (Some(session_id), Some(terminal_id)) = (&config.session_id, &config.terminal_id) {
        println!("reusing session_id={session_id} terminal_id={terminal_id}");
        return Ok(TerminalIds {
            session_id: session_id.clone(),
            terminal_id: terminal_id.clone(),
        });
    }

    let session_id = config
        .session_id
        .clone()
        .ok_or_else(|| {
            anyhow!(
                "OMNIGENT_SESSION_ID is required when OMNIGENT_TERMINAL_ID is not also set"
            )
        })?;

    let url = format!(
        "{}/v1/sessions/{session_id}/resources/terminals",
        config.base_url.trim_end_matches('/')
    );

    // OpenAPI 0.5.1 documents the semantics but omits a requestBody schema; this
    // is the shape implied by the route description + lens-client call sites.
    let body = serde_json::json!({
        "terminal": config.terminal_name,
        "session_key": config.terminal_session_key,
    });
    println!(
        "POST {url} body={}",
        serde_json::to_string(&body).unwrap_or_default()
    );

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        bail!("terminal create failed ({status}): {text}");
    }

    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parse terminal create: {text}"))?;
    let terminal_id = value
        .get("id")
        .and_then(|id| id.as_str())
        .ok_or_else(|| anyhow!("terminal create response missing string id: {value}"))?
        .to_owned();
    println!("created session_id={session_id} terminal_id={terminal_id}");
    Ok(TerminalIds {
        session_id,
        terminal_id,
    })
}

fn attach_url(config: &Config, ids: &TerminalIds) -> Result<String> {
    let ws_base = http_to_ws_base(&config.base_url)?;
    let mut url = format!(
        "{ws_base}/v1/sessions/{}/resources/terminals/{}/attach",
        ids.session_id, ids.terminal_id
    );
    let mut query = Vec::new();
    match config.transport {
        Transport::Pty => query.push("transport=pty".to_string()),
        Transport::Control => query.push("transport=control".to_string()),
        Transport::Default => {}
    }
    query.push("read_only=false".to_string());
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }
    Ok(url)
}

#[derive(Serialize)]
struct FrameRecord {
    ts_offset_ms: u64,
    direction: String,
    kind: String,
    len: usize,
    hex: String,
    utf8_lossy: String,
}

#[derive(Default, Clone, Debug)]
struct FrameStats {
    frames: usize,
    bytes_in: usize,
    bytes_out: usize,
}

struct FrameLogger {
    file: File,
    start: Instant,
    stats: FrameStats,
}

impl FrameLogger {
    fn create(path: &Path) -> Result<Self> {
        let file = File::create(path)
            .with_context(|| format!("open capture file {}", path.display()))?;
        Ok(Self::from_file(file))
    }

    fn from_file(file: File) -> Self {
        Self {
            file,
            start: Instant::now(),
            stats: FrameStats::default(),
        }
    }

    fn reset_leg(&mut self) {
        self.start = Instant::now();
        self.stats = FrameStats::default();
    }

    fn log_in(&mut self, kind: &'static str, payload: &[u8]) -> Result<()> {
        self.stats.frames += 1;
        self.stats.bytes_in += payload.len();
        self.write_record("in", kind, payload)
    }

    fn log_out(&mut self, kind: &'static str, payload: &[u8]) -> Result<()> {
        self.stats.frames += 1;
        self.stats.bytes_out += payload.len();
        self.write_record("out", kind, payload)
    }

    fn write_record(&mut self, direction: &str, kind: &str, payload: &[u8]) -> Result<()> {
        let record = FrameRecord {
            ts_offset_ms: self.start.elapsed().as_millis() as u64,
            direction: direction.to_owned(),
            kind: kind.to_owned(),
            len: payload.len(),
            hex: hex_encode(payload),
            utf8_lossy: String::from_utf8_lossy(payload).into_owned(),
        };
        serde_json::to_writer(&mut self.file, &record)?;
        self.file.write_all(b"\n")?;
        Ok(())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[derive(Debug, Clone)]
struct CloseInfo {
    code: u16,
    reason: String,
}

fn describe_close_code(code: u16) -> &'static str {
    match code {
        4404 => "TERMINAL_NOT_FOUND (stop reconnecting)",
        4405 => "TERMINAL_DETACHED (reconnect ok)",
        4500 => "INTERNAL_ERROR (retry with backoff)",
        _ => "other",
    }
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

struct AttachConn {
    write: futures_util::stream::SplitSink<WsStream, Message>,
    reader: tokio::task::JoinHandle<Result<(FrameStats, Option<CloseInfo>)>>,
}

impl AttachConn {
    async fn connect(
        config: &Config,
        ids: &TerminalIds,
        logger: Arc<Mutex<FrameLogger>>,
    ) -> Result<Self> {
        let url = attach_url(config, ids)?;
        println!("WS connect {url}");

        let mut request = url
            .clone()
            .into_client_request()
            .with_context(|| format!("build WS request for {url}"))?;
        if let Some(token) = &config.token {
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {token}")
                    .parse()
                    .context("invalid bearer token for WS handshake")?,
            );
        }

        let (stream, response) = connect_async(request)
            .await
            .with_context(|| format!("WS handshake to {url}"))?;
        println!(
            "WS handshake status={} headers={:?}",
            response.status(),
            response.headers()
        );

        let (write, mut read) = stream.split();
        let logger_reader = Arc::clone(&logger);

        let reader = tokio::spawn(async move {
            let mut close_info = None;
            while let Some(msg) = read.next().await {
                match msg.with_context(|| "WS read")? {
                    Message::Text(text) => {
                        let payload = text.as_str().as_bytes();
                        logger_reader
                            .lock()
                            .await
                            .log_in("text", payload)
                            .context("log inbound text")?;
                    }
                    Message::Binary(bin) => {
                        logger_reader
                            .lock()
                            .await
                            .log_in("binary", &bin)
                            .context("log inbound binary")?;
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                    Message::Close(frame) => {
                        if let Some(frame) = frame {
                            let code = frame.code.into();
                            let reason = frame.reason.to_string();
                            println!(
                                "WS close code={code} ({}) reason={reason:?}",
                                describe_close_code(code)
                            );
                            close_info = Some(CloseInfo { code, reason });
                        } else {
                            println!("WS close (no frame)");
                        }
                        break;
                    }
                }
            }

            let logger = logger_reader.lock().await;
            Ok((logger.stats.clone(), close_info))
        });

        Ok(Self { write, reader })
    }

    async fn send_binary(
        &mut self,
        logger: &Arc<Mutex<FrameLogger>>,
        payload: &[u8],
    ) -> Result<()> {
        logger
            .lock()
            .await
            .log_out("binary", payload)
            .context("log outbound binary")?;
        self.write
            .send(Message::Binary(payload.to_vec().into()))
            .await
            .context("send binary frame")?;
        Ok(())
    }

    async fn send_text(&mut self, logger: &Arc<Mutex<FrameLogger>>, payload: &str) -> Result<()> {
        logger
            .lock()
            .await
            .log_out("text", payload.as_bytes())
            .context("log outbound text")?;
        self.write
            .send(Message::Text(payload.into()))
            .await
            .context("send text frame")?;
        Ok(())
    }

    async fn finish(mut self) -> Result<(FrameStats, Option<CloseInfo>)> {
        let _ = self.write.send(Message::Close(None)).await;
        let _ = self.write.close().await;
        match self.reader.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(err),
            Err(join_err) => Err(join_err).context("reader task join"),
        }
    }

    async fn hard_drop(self) -> Result<(FrameStats, Option<CloseInfo>)> {
        drop(self.write);
        match self.reader.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(err),
            Err(join_err) => Err(join_err).context("reader task join after hard drop"),
        }
    }
}

#[derive(Debug, Clone)]
struct ScenarioResult {
    name: String,
    stats: FrameStats,
    close: Option<CloseInfo>,
    capture_path: PathBuf,
}

async fn run_attach(
    config: &Config,
    ids: &TerminalIds,
    capture_path: &Path,
) -> Result<ScenarioResult> {
    println!("\n=== scenario: attach ===");
    let logger = Arc::new(Mutex::new(FrameLogger::create(capture_path)?));
    let conn = AttachConn::connect(config, ids, Arc::clone(&logger)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (stats, close) = conn.finish().await?;
    Ok(ScenarioResult {
        name: "attach".to_string(),
        stats,
        close,
        capture_path: capture_path.to_path_buf(),
    })
}

async fn run_input(
    config: &Config,
    ids: &TerminalIds,
    capture_path: &Path,
) -> Result<ScenarioResult> {
    println!("\n=== scenario: input ===");
    let logger = Arc::new(Mutex::new(FrameLogger::create(capture_path)?));
    let mut conn = AttachConn::connect(config, ids, Arc::clone(&logger)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    conn.send_binary(
        &logger,
        b"printf 'MARKER_A\\n'; ls -la\n",
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (stats, close) = conn.finish().await?;
    Ok(ScenarioResult {
        name: "input".to_string(),
        stats,
        close,
        capture_path: capture_path.to_path_buf(),
    })
}

async fn run_resize(
    config: &Config,
    ids: &TerminalIds,
    capture_path: &Path,
) -> Result<ScenarioResult> {
    println!("\n=== scenario: resize ===");
    let logger = Arc::new(Mutex::new(FrameLogger::create(capture_path)?));
    let mut conn = AttachConn::connect(config, ids, Arc::clone(&logger)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    conn.send_text(
        &logger,
        r#"{"type":"resize","cols":120,"rows":40}"#,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (stats, close) = conn.finish().await?;
    Ok(ScenarioResult {
        name: "resize".to_string(),
        stats,
        close,
        capture_path: capture_path.to_path_buf(),
    })
}

async fn run_reconnect(
    config: &Config,
    ids: &TerminalIds,
    capture_path: &Path,
) -> Result<ScenarioResult> {
    println!("\n=== scenario: reconnect ===");
    let logger = Arc::new(Mutex::new(FrameLogger::create(capture_path)?));
    let mut conn = AttachConn::connect(config, ids, Arc::clone(&logger)).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    conn.send_binary(&logger, b"printf 'BEFORE_DROP\\n'\n").await?;
    conn.send_binary(
        &logger,
        b"for i in $(seq 1 30); do echo line $i; sleep 0.1; done\n",
    )
    .await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    println!("hard-closing first attach after ~1s of output");
    let (stats_first, close_first) = conn.hard_drop().await?;
    if let Some(close) = &close_first {
        println!(
            "first attach close code={} ({}) reason={:?}",
            close.code,
            describe_close_code(close.code),
            close.reason
        );
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("re-attaching to terminal_id={}", ids.terminal_id);
    logger.lock().await.reset_leg();
    let mut conn = AttachConn::connect(config, ids, Arc::clone(&logger)).await?;
    conn.send_binary(&logger, b"printf 'AFTER_RECONNECT\\n'\n").await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (stats_second, close_second) = conn.finish().await?;

    let stats = FrameStats {
        frames: stats_first.frames + stats_second.frames,
        bytes_in: stats_first.bytes_in + stats_second.bytes_in,
        bytes_out: stats_first.bytes_out + stats_second.bytes_out,
    };

    Ok(ScenarioResult {
        name: "reconnect".to_string(),
        stats,
        close: close_second.or(close_first),
        capture_path: capture_path.to_path_buf(),
    })
}

async fn run_scenario(
    config: &Config,
    ids: &TerminalIds,
    scenario: Scenario,
) -> Result<ScenarioResult> {
    let capture_path = config
        .capture_dir
        .join(format!("{}.frames.jsonl", scenario.name()));
    match scenario {
        Scenario::Attach => run_attach(config, ids, &capture_path).await,
        Scenario::Input => run_input(config, ids, &capture_path).await,
        Scenario::Resize => run_resize(config, ids, &capture_path).await,
        Scenario::Reconnect => run_reconnect(config, ids, &capture_path).await,
        Scenario::All => unreachable!("handled by caller"),
    }
}

fn print_summary(results: &[ScenarioResult], transport: Transport) {
    println!("\n========== run summary ==========");
    println!("transport={}", transport_label(transport));
    for result in results {
        println!(
            "scenario={} frames={} bytes_in={} bytes_out={} capture={}",
            result.name,
            result.stats.frames,
            result.stats.bytes_in,
            result.stats.bytes_out,
            result.capture_path.display()
        );
        match &result.close {
            Some(close) => println!(
                "  close code={} ({}) reason={:?}",
                close.code,
                describe_close_code(close.code),
                close.reason
            ),
            None => println!("  close: (none observed)"),
        }
    }
}

fn transport_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Default => "default (omit query param)",
        Transport::Pty => "pty",
        Transport::Control => "control",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env_and_args()?;
    println!("capture_dir={}", config.capture_dir.display());
    println!("transport={}", transport_label(config.transport));

    let client = rest_client(&config).await?;
    let ids = ensure_terminal(&config, &client).await?;
    println!(
        "{{\"session_id\":\"{}\",\"terminal_id\":\"{}\"}}",
        ids.session_id, ids.terminal_id
    );

    let scenarios = match config.scenario {
        Scenario::All => vec![
            Scenario::Attach,
            Scenario::Input,
            Scenario::Resize,
            Scenario::Reconnect,
        ],
        other => vec![other],
    };

    let mut results = Vec::new();
    for scenario in scenarios {
        results.push(run_scenario(&config, &ids, scenario).await?);
    }

    print_summary(&results, config.transport);
    Ok(())
}
