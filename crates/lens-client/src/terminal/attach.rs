use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use futures_util::{SinkExt, StreamExt};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::client::Client;
use crate::connection::Auth;
use crate::error::{ClientError, Result};
use crate::ids::{SessionId, TerminalId};

use super::close::CloseCause;
use super::wire::{WsInbound, WsOutbound, classify_inbound, encode_outbound};

/// Bound on attach in/out queues. Full inbound blocks the I/O thread briefly,
/// then disconnects so Slice 1d can enter visible reconnect.
pub(crate) const ATTACH_CHANNEL_BOUND: usize = 256;

/// Brief block before treating inbound saturation as sustained (Slice 1d policy).
const INBOUND_SEND_TIMEOUT: Duration = Duration::from_millis(50);

const BACKOFF_CAP: Duration = Duration::from_secs(30);
const BACKOFF_BASE_MS: u64 = 100;
const INSPECT_RING_CAP: usize = 32;
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub struct AttachOptions {
    pub read_only: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AttachInspect {
    pub connected: bool,
    pub inbound_len: usize,
    pub inbound_cap: usize,
    pub outbound_len: usize,
    pub outbound_cap: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub last_close: Option<CloseCause>,
    pub recent: Vec<InspectEvent>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct InspectEvent {
    pub kind: &'static str,
}

pub struct AttachHandle {
    pub inbound: Receiver<WsInbound>,
    pub outbound: Sender<WsOutbound>,
    shutdown: Arc<AtomicBool>,
    inspect: Arc<AttachInspectState>,
    _handle: Option<JoinHandle<()>>,
}

struct AttachInspectState {
    enabled: AtomicBool,
    connected: AtomicBool,
    inbound_cap: usize,
    outbound_cap: usize,
    bytes_in: std::sync::atomic::AtomicU64,
    bytes_out: std::sync::atomic::AtomicU64,
    last_close: std::sync::Mutex<Option<CloseCause>>,
    recent: std::sync::Mutex<std::collections::VecDeque<InspectEvent>>,
}

impl AttachInspectState {
    fn new(inbound_cap: usize, outbound_cap: usize) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            inbound_cap,
            outbound_cap,
            bytes_in: std::sync::atomic::AtomicU64::new(0),
            bytes_out: std::sync::atomic::AtomicU64::new(0),
            last_close: std::sync::Mutex::new(None),
            recent: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    fn snapshot(&self, inbound_len: usize, outbound_len: usize) -> AttachInspect {
        let enabled = self.enabled.load(Ordering::Relaxed);
        let recent = if enabled {
            self.recent
                .lock()
                .map(|r| r.iter().cloned().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let last_close = if enabled {
            self.last_close.lock().ok().and_then(|g| *g)
        } else {
            None
        };
        AttachInspect {
            connected: self.connected.load(Ordering::Relaxed),
            inbound_len,
            inbound_cap: self.inbound_cap,
            outbound_len,
            outbound_cap: self.outbound_cap,
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
            last_close,
            recent,
        }
    }

    fn record_event(&self, kind: &'static str) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut ring) = self.recent.lock() {
            if ring.len() >= INSPECT_RING_CAP {
                ring.pop_front();
            }
            ring.push_back(InspectEvent { kind });
        }
    }

    fn on_connected(&self) {
        self.connected.store(true, Ordering::Relaxed);
        self.record_event("connect");
    }

    fn on_closed(&self, cause: CloseCause) {
        self.connected.store(false, Ordering::Relaxed);
        if self.enabled.load(Ordering::Relaxed) {
            if let Ok(mut last) = self.last_close.lock() {
                *last = Some(cause);
            }
            self.record_event("close");
        }
    }

    fn on_saturation(&self) {
        self.record_event("saturation");
    }
}

impl AttachHandle {
    pub fn close(mut self) {
        self.signal_shutdown();
        if let Some(h) = self._handle.take() {
            let _ = h.join();
        }
    }

    pub fn inspect(&self) -> AttachInspect {
        self.inspect
            .snapshot(self.inbound.len(), self.outbound.len())
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_inspect_enabled(&self, enabled: bool) {
        self.inspect.enabled.store(enabled, Ordering::Relaxed);
    }

    fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for AttachHandle {
    fn drop(&mut self) {
        self.signal_shutdown();
        if let Some(h) = self._handle.take() {
            let _ = h.join();
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub fn next_delay(&mut self) -> Duration {
        let shift = self.attempt.min(20);
        let ms = BACKOFF_BASE_MS.saturating_mul(1u64 << shift);
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_millis(ms).min(BACKOFF_CAP)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Spawn a dedicated I/O thread with a contained current-thread tokio runtime.
/// tokio/tungstenite types never cross the thread boundary — only `WsInbound` /
/// `WsOutbound` do via crossbeam.
pub fn attach(
    client: &Client,
    session: &SessionId,
    tid: &TerminalId,
    opts: AttachOptions,
) -> Result<AttachHandle> {
    let path = format!(
        "/v1/sessions/{session}/resources/terminals/{tid}/attach?read_only={}",
        opts.read_only
    );
    let ws_url = http_to_ws_url(client.conn(), &path)?;
    let conn = client.conn().clone();
    let (inbound_tx, inbound_rx) = bounded(ATTACH_CHANNEL_BOUND);
    let (outbound_tx, outbound_rx) = bounded(ATTACH_CHANNEL_BOUND);
    let shutdown = Arc::new(AtomicBool::new(false));
    let inspect = Arc::new(AttachInspectState::new(
        ATTACH_CHANNEL_BOUND,
        ATTACH_CHANNEL_BOUND,
    ));
    let thread_shutdown = Arc::clone(&shutdown);
    let thread_inspect = Arc::clone(&inspect);

    let handle = std::thread::Builder::new()
        .name("lens-terminal-attach".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => {
                    drop(inbound_tx);
                    return;
                }
            };
            rt.block_on(io_loop(
                ws_url,
                conn,
                inbound_tx,
                outbound_rx,
                thread_shutdown,
                thread_inspect,
            ));
        })
        .map_err(|e| ClientError::ThreadSpawn(e.to_string()))?;

    Ok(AttachHandle {
        inbound: inbound_rx,
        outbound: outbound_tx,
        shutdown,
        inspect,
        _handle: Some(handle),
    })
}

fn http_to_ws_url(conn: &crate::connection::Connection, path: &str) -> Result<url::Url> {
    let http_url = conn.url(path)?;
    let scheme = match http_url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => {
            return Err(ClientError::NotFound {
                what: format!("unsupported URL scheme for WS attach: {other}"),
            });
        }
    };
    let mut ws = http_url;
    ws.set_scheme(scheme).map_err(|_| ClientError::NotFound {
        what: format!("could not set WS scheme for {path}"),
    })?;
    Ok(ws)
}

fn apply_auth(
    mut request: tokio_tungstenite::tungstenite::http::Request<()>,
    auth: &Auth,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    match auth {
        Auth::None => {}
        Auth::Bearer { token } => {
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {token}")
                    .parse()
                    .map_err(|_| ClientError::NotFound {
                        what: "invalid bearer token for WS handshake".into(),
                    })?,
            );
        }
        Auth::Cookie { value } => {
            request.headers_mut().insert(
                "Cookie",
                value.parse().map_err(|_| ClientError::NotFound {
                    what: "invalid cookie for WS handshake".into(),
                })?,
            );
        }
        Auth::ForwardedEmail { email } => {
            request.headers_mut().insert(
                "X-Forwarded-Email",
                email.parse().map_err(|_| ClientError::NotFound {
                    what: "invalid forwarded-email for WS handshake".into(),
                })?,
            );
        }
    }
    Ok(request)
}

fn deliver_inbound(tx: &mut Option<Sender<WsInbound>>, msg: WsInbound) -> bool {
    let Some(tx) = tx.as_ref() else {
        return false;
    };
    match tx.try_send(msg) {
        Ok(()) => true,
        Err(TrySendError::Full(msg)) => tx.send_timeout(msg, INBOUND_SEND_TIMEOUT).is_ok(),
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// Drop the inbound sender so `recv()` returns `Disconnected` — a signal that
/// cannot be blocked behind a saturated VT queue.
fn signal_inbound_gone(tx: &mut Option<Sender<WsInbound>>) {
    tx.take();
}

async fn wait_shutdown(shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Relaxed) {
        sleep(SHUTDOWN_POLL).await;
    }
}

fn forward_outbound(
    async_out_tx: &tokio::sync::mpsc::Sender<WsOutbound>,
    mut msg: WsOutbound,
    shutdown: &AtomicBool,
) -> bool {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        match async_out_tx.try_send(msg) {
            Ok(()) => return true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => {
                msg = returned;
                std::thread::sleep(SHUTDOWN_POLL);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return false,
        }
    }
}

async fn io_loop(
    ws_url: url::Url,
    conn: crate::connection::Connection,
    inbound_tx: Sender<WsInbound>,
    outbound_rx: Receiver<WsOutbound>,
    shutdown: Arc<AtomicBool>,
    inspect: Arc<AttachInspectState>,
) {
    let mut inbound_tx = Some(inbound_tx);
    let url_str = ws_url.to_string();
    let request = match url_str.into_client_request() {
        Ok(req) => req,
        Err(_) => {
            let _ = deliver_inbound(
                &mut inbound_tx,
                WsInbound::Closed(CloseCause::Network),
            );
            return;
        }
    };
    let request = match apply_auth(request, &conn.auth) {
        Ok(req) => req,
        Err(_) => {
            let _ = deliver_inbound(
                &mut inbound_tx,
                WsInbound::Closed(CloseCause::Network),
            );
            return;
        }
    };

    let connect = tokio::select! {
        result = connect_async(request) => result,
        () = wait_shutdown(&shutdown) => {
            signal_inbound_gone(&mut inbound_tx);
            return;
        }
    };
    let (mut sink, mut stream) = match connect {
        Ok((ws, _resp)) => ws.split(),
        Err(_) => {
            let _ = deliver_inbound(
                &mut inbound_tx,
                WsInbound::Closed(CloseCause::Network),
            );
            return;
        }
    };

    inspect.on_connected();

    let (async_out_tx, async_out_rx) =
        tokio::sync::mpsc::channel::<WsOutbound>(ATTACH_CHANNEL_BOUND);
    let forward_shutdown = Arc::clone(&shutdown);
    let forwarder = std::thread::spawn(move || {
        while !forward_shutdown.load(Ordering::Relaxed) {
            match outbound_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => {
                    if !forward_outbound(&async_out_tx, msg, &forward_shutdown) {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let mut async_out_rx = async_out_rx;

    let mut disconnect = false;
    let mut drop_inbound = false;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            drop_inbound = true;
            break;
        }
        tokio::select! {
            inbound = stream.next() => {
                match inbound {
                    Some(Ok(msg)) => {
                        if let Some(classified) = classify_inbound(msg) {
                            if let WsInbound::Vt(ref b) = classified {
                                inspect
                                    .bytes_in
                                    .fetch_add(b.len() as u64, Ordering::Relaxed);
                            }
                            let close_cause = match classified {
                                WsInbound::Closed(c) => Some(c),
                                other => {
                                    if !deliver_inbound(&mut inbound_tx, other) {
                                        disconnect = true;
                                        let cause = CloseCause::Network;
                                        inspect.on_saturation();
                                        inspect.on_closed(cause);
                                        signal_inbound_gone(&mut inbound_tx);
                                    }
                                    None
                                }
                            };
                            if let Some(cause) = close_cause {
                                let _ = deliver_inbound(
                                    &mut inbound_tx,
                                    WsInbound::Closed(cause),
                                );
                                inspect.on_closed(cause);
                                break;
                            }
                            if disconnect {
                                break;
                            }
                        }
                    }
                    Some(Err(_)) => {
                        let cause = CloseCause::Network;
                        inspect.on_closed(cause);
                        let _ = deliver_inbound(
                            &mut inbound_tx,
                            WsInbound::Closed(cause),
                        );
                        break;
                    }
                    None => {
                        let cause = CloseCause::Network;
                        inspect.on_closed(cause);
                        let _ = deliver_inbound(
                            &mut inbound_tx,
                            WsInbound::Closed(cause),
                        );
                        break;
                    }
                }
            }
            outbound = async_out_rx.recv() => {
                match outbound {
                    Some(msg) => {
                        let out_len = match &msg {
                            WsOutbound::Input(b) => b.len() as u64,
                            WsOutbound::Resize { .. } => 0,
                        };
                        inspect.bytes_out.fetch_add(out_len, Ordering::Relaxed);
                        let frame = encode_outbound(&msg);
                        if sink.send(frame).await.is_err() {
                            let cause = CloseCause::Network;
                            inspect.on_closed(cause);
                            let _ = deliver_inbound(
                                &mut inbound_tx,
                                WsInbound::Closed(cause),
                            );
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    shutdown.store(true, Ordering::Relaxed);
    drop(async_out_rx);
    let _ = forwarder.join();

    tokio::select! {
        _ = sink.close() => {}
        () = sleep(GRACEFUL_CLOSE_TIMEOUT) => {}
    }
    drop(sink);
    drop(stream);
    if drop_inbound {
        signal_inbound_gone(&mut inbound_tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn resize_from_outbound_channel_encodes_on_wire() {
        let (tx, rx) = bounded(1);
        tx.send(WsOutbound::Resize {
            cols: 120,
            rows: 40,
        })
        .unwrap();
        let msg = rx.recv().unwrap();
        match encode_outbound(&msg) {
            Message::Text(t) => {
                assert_eq!(t.as_str(), r#"{"type":"resize","cols":120,"rows":40}"#);
            }
            _ => panic!("expected text frame"),
        }
    }

    #[test]
    fn backoff_grows_then_caps_at_30s() {
        let mut b = Backoff::default();
        let d0 = b.next_delay();
        let d1 = b.next_delay();
        assert!(d1 >= d0);
        for _ in 0..20 {
            let d = b.next_delay();
            assert!(d <= Duration::from_secs(30));
        }
        b.reset();
        assert!(b.next_delay() <= d1);
    }

    #[test]
    fn inspect_ring_records_connect_and_close_when_enabled() {
        let state = AttachInspectState::new(8, 8);
        state.enabled.store(true, Ordering::Relaxed);
        state.on_connected();
        state.on_closed(CloseCause::Network);
        let snap = state.snapshot(0, 0);
        assert_eq!(snap.recent.len(), 2);
        assert_eq!(snap.recent[0].kind, "connect");
        assert_eq!(snap.recent[1].kind, "close");
        assert_eq!(snap.last_close, Some(CloseCause::Network));
    }

    #[test]
    fn inspect_ring_stays_empty_when_disabled() {
        let state = AttachInspectState::new(8, 8);
        state.on_connected();
        state.on_closed(CloseCause::Network);
        let snap = state.snapshot(0, 0);
        assert!(snap.recent.is_empty());
        assert!(snap.last_close.is_none());
    }

    #[test]
    fn inspect_ring_evicts_oldest_at_capacity() {
        let state = AttachInspectState::new(8, 8);
        state.enabled.store(true, Ordering::Relaxed);
        for _ in 0..INSPECT_RING_CAP + 5 {
            state.record_event("pulse");
        }
        let snap = state.snapshot(0, 0);
        assert_eq!(snap.recent.len(), INSPECT_RING_CAP);
        assert_eq!(snap.recent[0].kind, "pulse");
    }
}
