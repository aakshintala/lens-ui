//! The SSE reader thread: holds the blocking reqwest body, feeds the pure
//! parser, deserializes typed events, and pushes them down a bounded crossbeam
//! channel.
//! One thread per active session (typed-client.md §4); the gpui poller drains
//! via `try_recv` off `cx.background_spawn`. Never blocks the foreground thread.

use std::io::Read;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::error::ClientError;
use crate::reconnect::{BACKOFF_MS, Reopen, items_to_replay};
use crate::sessions::SessionStatus;

use super::event::{DisconnectReason, ServerStreamEvent, parse_event};
use super::normalize::Normalizer;
use super::sse::SseParser;

/// Bound on the reader→poller channel. A full channel blocks the reader thread
/// (off the foreground), propagating backpressure to TCP (impl-spec §6). Sized
/// for a generous burst without unbounded growth under a stalled UI poller.
pub(crate) const EVENT_CHANNEL_BOUND: usize = 1024;

pub struct EventStream {
    rx: Receiver<ServerStreamEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _handle: JoinHandle<()>,
}

impl EventStream {
    /// Spawn the reader thread over an open blocking response body.
    pub(crate) fn spawn<Re: Reopen + 'static>(
        resp: reqwest::blocking::Response,
        reopener: Re,
    ) -> crate::error::Result<Self> {
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("lens-sse-reader".into())
            .spawn(move || {
                run(
                    Box::new(resp) as Box<dyn Read + Send>,
                    tx,
                    reopener,
                    std::thread::sleep,
                    &thread_stop,
                )
            })
            .map_err(|e| crate::error::ClientError::ThreadSpawn(e.to_string()))?;
        Ok(EventStream {
            rx,
            stop,
            _handle: handle,
        })
    }

    /// Cooperatively stop the reader. Takes effect on the next read/heartbeat or
    /// backoff tick (omnigent sends `session.heartbeat`, so this is bounded in
    /// practice). A fully silent socket is interrupted only when the next byte
    /// arrives — a read-idle backstop is deferred (it would race the reconnect path).
    /// A reader parked in a blocking channel send (full bounded channel) is
    /// unblocked by dropping the `EventStream` (which drops the receiver, so the
    /// next send errors and the reader exits); `stop()` itself covers the read-park
    /// case (heartbeat-bounded).
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The raw event receiver, for a consumer that multiplexes it with other
    /// channels (the lens-core actor `Select`s over this + its command channel).
    ///
    /// Single-consumer invariant: this shares the one internal queue with
    /// `recv()`/`try_recv()`. Exactly one consumer must drain it — the actor's
    /// `Select` is the sole consumer once it attaches. Do not also poll via
    /// `recv()`/`try_recv()` or clone the returned receiver concurrently;
    /// crossbeam receivers split a stream across consumers rather than
    /// broadcasting it.
    pub fn receiver(&self) -> &Receiver<ServerStreamEvent> {
        &self.rx
    }

    /// Block until the next event, or `None` when the stream closes.
    pub fn recv(&self) -> Option<ServerStreamEvent> {
        self.rx.recv().ok()
    }

    /// Non-blocking drain for the UI poller. `None` when no event is queued
    /// (including after the stream has closed).
    pub fn try_recv(&self) -> Option<ServerStreamEvent> {
        self.rx.try_recv().ok()
    }
}

/// §7 stop-immediately table. check_status/decode_json encode 401|403 as Auth{status}
/// and everything else (incl. 404) as Server{status,..}, so 404 is matched on Server.
fn stop_reason(e: &ClientError) -> Option<DisconnectReason> {
    match e {
        ClientError::Auth { status: 401 } => Some(DisconnectReason::Unauthorized),
        ClientError::Auth { status: 403 } => Some(DisconnectReason::Forbidden),
        ClientError::Server { status: 404, .. } => Some(DisconnectReason::NotFound),
        _ => None, // network / 5xx / parse — retryable
    }
}

/// First-open prelude: emit the same post-open sequence as reconnect, minus the
/// Reconnecting/Reconnected markers (no gap on first connect), so the consumer's
/// reducer is the single writer (app-arch §4.1). Returns `false` to abort `run`
/// (consumer gone, or a fatal fetch error for which Disconnected was sent).
/// Retryable fetch failure degrades to live-tail-only (no regression vs pre-Plan-4).
fn bootstrap<Re: Reopen>(
    reopener: &Re,
    tx: &Sender<ServerStreamEvent>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    match reopener
        .snapshot()
        .and_then(|snap| reopener.items().map(|items| (snap, items)))
    {
        Ok((snap, items)) => {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return false;
            }
            if tx
                .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
                .is_err()
            {
                return false;
            }
            for ev in items_to_replay(items) {
                if tx.send(ev).is_err() {
                    return false;
                }
            }
            true
        }
        Err(e) => match stop_reason(&e) {
            Some(r) => {
                let _ = tx.send(ServerStreamEvent::Disconnected { reason: r });
                false
            }
            None => true, // retryable: skip prelude, proceed to live tail
        },
    }
}

fn run<Re: Reopen>(
    body: Box<dyn Read + Send>,
    tx: Sender<ServerStreamEvent>,
    reopener: Re,
    sleep: impl Fn(Duration),
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if !bootstrap(&reopener, &tx, stop) {
        return;
    }
    read_loop(body, tx, reopener, sleep, stop);
}

fn read_loop<Re: Reopen>(
    mut body: Box<dyn Read + Send>,
    tx: Sender<ServerStreamEvent>,
    reopener: Re,
    sleep: impl Fn(Duration),
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut parser = SseParser::default();
    let mut normalizer = Normalizer::default();
    let mut buf = [0u8; 8192];
    let mut last_seen_seq: Option<u64> = None;
    let mut resume_floor: Option<u64> = None; // Some(_) => inside post-reopen dedup window
    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match body.read(&mut buf) {
            Ok(0) => {
                // CLEAN EOF: flush reasoning bracket BEFORE reconnecting (§7a). Send errors => consumer gone => return.
                for frame in parser.finish() {
                    for ev in normalizer.push(parse_event(&frame)) {
                        if tx.send(ev).is_err() {
                            return;
                        }
                    }
                }
                for ev in normalizer.flush() {
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
                match reconnect(&reopener, &sleep, &tx, &mut normalizer, stop) {
                    Some(nb) => {
                        body = nb;
                        parser = SseParser::default();
                        resume_floor = last_seen_seq;
                    }
                    None => return, // Disconnected already sent; channel closes
                }
            }
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    let seq = frame.sequence_number();
                    if let Some(s) = seq {
                        last_seen_seq = Some(last_seen_seq.map_or(s, |p| p.max(s)));
                    }
                    // overlap dedup window: drop frames already delivered before the drop
                    if let Some(floor) = resume_floor {
                        match seq {
                            Some(s) if s <= floor => continue, // duplicate overlap — drop
                            Some(_) => {
                                resume_floor = None;
                            } // first fresh seq — exit window, process it
                            None => {} // no seq — process, window stays open
                        }
                    }
                    for ev in normalizer.push(parse_event(&frame)) {
                        if tx.send(ev).is_err() {
                            return;
                        }
                    }
                }
            }
            // TRANSPORT DROP: do NOT flush (the bracket did not end, the connection did — §7a).
            Err(_) => match reconnect(&reopener, &sleep, &tx, &mut normalizer, stop) {
                Some(nb) => {
                    body = nb;
                    parser = SseParser::default();
                    resume_floor = last_seen_seq;
                }
                None => return,
            },
        }
    }
}

fn reconnect<Re: Reopen>(
    reopener: &Re,
    sleep: &impl Fn(Duration),
    tx: &Sender<ServerStreamEvent>,
    normalizer: &mut Normalizer,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<Box<dyn Read + Send>> {
    for (i, &delay) in BACKOFF_MS.iter().enumerate() {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        if tx
            .send(ServerStreamEvent::Reconnecting {
                attempt: i as u32 + 1,
            })
            .is_err()
        {
            return None;
        }
        sleep(Duration::from_millis(delay));
        let snap = match reopener.snapshot() {
            Ok(s) => s,
            Err(e) => {
                if let Some(r) = stop_reason(&e) {
                    let _ = tx.send(ServerStreamEvent::Disconnected { reason: r });
                    return None;
                }
                continue;
            }
        };
        if snap.status() == SessionStatus::Failed {
            // Terminal: no live tail resumes, so we do NOT emit `Reconnected`. Deliver
            // the terminal snapshot (it carries the failure state) then Disconnect.
            let _ = tx.send(ServerStreamEvent::SnapshotRestored(Box::new(snap)));
            let _ = tx.send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::SessionFailed,
            });
            return None;
        }
        // Fetch durable history BEFORE opening the live body: a retryable `/items`
        // failure must never discard an already-opened no-replay stream.
        let list = match reopener.items() {
            Ok(l) => l,
            Err(e) => {
                if let Some(r) = stop_reason(&e) {
                    let _ = tx.send(ServerStreamEvent::Disconnected { reason: r });
                    return None;
                }
                continue;
            }
        };
        // open_stream is the LAST fallible call: if it fails retryably, no body was
        // opened, so `continue` drops nothing.
        let new_body = match reopener.open_stream() {
            Ok(b) => b,
            Err(e) => {
                if let Some(r) = stop_reason(&e) {
                    let _ = tx.send(ServerStreamEvent::Disconnected { reason: r });
                    return None;
                }
                continue;
            }
        };
        if tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .is_err()
        {
            return None;
        }
        normalizer.reset_transient();
        if tx
            .send(ServerStreamEvent::SnapshotRestored(Box::new(snap)))
            .is_err()
        {
            return None;
        }
        for ev in items_to_replay(list) {
            if tx.send(ev).is_err() {
                return None;
            }
        }
        return Some(new_body);
    }
    let _ = tx.send(ServerStreamEvent::Disconnected {
        reason: DisconnectReason::RetriesExhausted,
    });
    None
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::super::event::{ResponseEvent, ServerStreamEvent};
    use super::*;

    /// Yields each step once, then `Ok(0)` or `Err` per the final step.
    struct StepRead {
        steps: Vec<io::Result<&'static [u8]>>,
        next: usize,
    }

    impl Read for StepRead {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.get(self.next) {
                Some(Ok(chunk)) => {
                    self.next += 1;
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    Ok(n)
                }
                Some(Err(e)) => {
                    self.next += 1;
                    Err(io::Error::new(e.kind(), e.to_string()))
                }
                None => Ok(0),
            }
        }
    }

    fn reasoning_started_frame() -> &'static [u8] {
        b"event: response.reasoning.started\ndata: {\"sequence_number\": 3, \"type\": \"response.reasoning.started\"}\n\n"
    }

    fn happy_idle_snapshot() -> crate::sessions::SessionSnapshot {
        let raw = include_str!(
            "../../../../docs/spikes/captures/2026-06-26-sse/happy_path.snapshot.json"
        );
        serde_json::from_str(raw).expect("parse happy snapshot")
    }

    /// Mock whose `open_stream` always returns a retryable 503 (exhausts backoff).
    struct ExhaustReopener {
        snapshot: crate::sessions::SessionSnapshot,
    }

    impl crate::reconnect::Reopen for ExhaustReopener {
        fn open_stream(&self) -> crate::error::Result<Box<dyn Read + Send>> {
            Err(crate::error::ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            })
        }

        fn snapshot(&self) -> crate::error::Result<crate::sessions::SessionSnapshot> {
            Ok(self.snapshot.clone())
        }

        fn items(&self) -> crate::error::Result<crate::sessions::ItemList> {
            Err(crate::error::ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            })
        }
    }

    #[test]
    fn channel_is_bounded() {
        assert_eq!(EVENT_CHANNEL_BOUND, 1024);
    }

    /// `EventStream::receiver()` exposes the same `rx` that `recv()` drains; we
    /// cannot construct `EventStream` without a live `reqwest` body, so this
    /// drives `run` over a crossbeam bounded channel and reads via `rx.recv`.
    #[test]
    fn crossbeam_receiver_delivers_events_from_run() {
        use std::io::Cursor;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        use crate::reconnect::Reopen;
        use crate::sessions::{ItemList, SessionSnapshot};
        use crate::stream::event::ResponseEvent;

        struct MockReopen {
            snapshot: SessionSnapshot,
            items: std::sync::Mutex<Option<ItemList>>,
        }

        impl Reopen for MockReopen {
            fn open_stream(&self) -> crate::error::Result<Box<dyn Read + Send>> {
                Err(crate::error::ClientError::Server {
                    status: 503,
                    body: serde_json::json!({}),
                })
            }

            fn snapshot(&self) -> crate::error::Result<SessionSnapshot> {
                Ok(self.snapshot.clone())
            }

            fn items(&self) -> crate::error::Result<ItemList> {
                match self.items.lock().unwrap().take() {
                    Some(list) => Ok(list),
                    None => Err(crate::error::ClientError::Server {
                        status: 503,
                        body: serde_json::json!({}),
                    }),
                }
            }
        }

        let reopener = MockReopen {
            snapshot: happy_idle_snapshot(),
            items: std::sync::Mutex::new(Some({
                let raw = include_str!(
                    "../../../../docs/spikes/captures/2026-06-26-sse/happy_path.items.json"
                );
                serde_json::from_str(raw).expect("parse happy items")
            })),
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let body: Box<dyn Read + Send> = Box::new(Cursor::new(Vec::<u8>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        run(body, tx, reopener, |_d| {}, &stop);

        let first = rx.recv().expect("snapshot via crossbeam receiver");
        assert!(matches!(first, ServerStreamEvent::SnapshotRestored(_)));
        let second = rx.recv().expect("replayed item via crossbeam receiver");
        assert!(matches!(
            second,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
        ));
    }

    /// Crossbeam bounded `Sender::send` unblocks when all receivers drop — the
    /// primitive `stop()`'s doc relies on for a reader parked on a full channel.
    #[test]
    fn dropping_receiver_unblocks_a_parked_sender() {
        use std::time::Instant;

        const CAP: usize = 1;
        let (tx, rx) = bounded::<ServerStreamEvent>(CAP);
        let filler = || ServerStreamEvent::Reconnecting { attempt: 1 };
        // Fill to capacity so the producer's next send MUST block.
        tx.send(filler()).expect("prime the full channel");

        // ready-signal proves the producer thread was scheduled and reached the
        // blocking send before we drop rx (closes the "never scheduled" hole).
        let (ready_tx, ready_rx) = bounded::<()>(1);
        let producer = std::thread::spawn(move || {
            ready_tx.send(()).expect("signal about-to-block");
            // Channel is full → this send parks until rx drops, then errors.
            tx.send(filler()).is_err()
        });

        ready_rx.recv().expect("producer reached the blocking send");
        std::thread::sleep(Duration::from_millis(20)); // let it enter the parked send
        drop(rx); // wake the parked sender → Err

        // Watchdog: a regression must fail, not hang CI.
        let (done_tx, done_rx) = bounded::<bool>(1);
        let watch = std::thread::spawn(move || {
            let errored = producer.join().expect("producer join");
            let _ = done_tx.send(errored);
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let errored = loop {
            if let Ok(v) = done_rx.try_recv() {
                break v;
            }
            assert!(
                Instant::now() < deadline,
                "parked sender was not unblocked by rx drop"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        watch.join().expect("watchdog join");
        assert!(
            errored,
            "the parked send must return Err after the receiver dropped"
        );
    }

    #[test]
    fn run_returns_immediately_when_stop_is_set_at_entry() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        // A body that would block/emit if read; the entry stop check must short-circuit.
        let body: Box<dyn Read + Send> = Box::new(StepRead {
            steps: vec![Ok(b"event: session.heartbeat\ndata: {}\n\n")],
            next: 0,
        });
        let stop = Arc::new(AtomicBool::new(true)); // already stopped
        let reopener = ExhaustReopener {
            snapshot: happy_idle_snapshot(),
        };
        run(body, tx, reopener, |_d| {}, &stop);
        assert!(rx.try_recv().is_err(), "stopped run must emit nothing");
    }

    #[test]
    fn transport_error_does_not_synthesize_reasoning_closed() {
        let reader = StepRead {
            steps: vec![
                Ok(reasoning_started_frame()),
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "mock transport drop",
                )),
            ],
            next: 0,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        run(
            Box::new(reader) as Box<dyn Read + Send>,
            tx,
            ExhaustReopener {
                snapshot: happy_idle_snapshot(),
            },
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();
        assert!(!events.iter().any(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
            )
        }));
        assert!(events.iter().any(|e| {
            matches!(
                e,
                ServerStreamEvent::Disconnected {
                    reason: DisconnectReason::RetriesExhausted
                }
            )
        }));
    }

    #[test]
    fn clean_eof_flushes_dangling_reasoning_closed() {
        let reader = StepRead {
            steps: vec![Ok(reasoning_started_frame())],
            next: 0,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        run(
            Box::new(reader) as Box<dyn Read + Send>,
            tx,
            ExhaustReopener {
                snapshot: happy_idle_snapshot(),
            },
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();
        let closes = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
                )
            })
            .count();
        assert_eq!(closes, 1);
        let close_idx = events.iter().position(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
            )
        });
        let disconnect_idx = events.iter().position(|e| {
            matches!(
                e,
                ServerStreamEvent::Disconnected {
                    reason: DisconnectReason::RetriesExhausted
                }
            )
        });
        assert!(close_idx < disconnect_idx);
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    use crate::error::ClientError;
    use crate::reconnect::Reopen;
    use crate::sessions::{ItemList, SessionSnapshot};
    use crate::stream::event::ResponseEvent;
    use std::io::{Cursor, Read};
    use std::sync::Mutex;

    fn happy_idle_snapshot() -> SessionSnapshot {
        let raw = include_str!(
            "../../../../docs/spikes/captures/2026-06-26-sse/happy_path.snapshot.json"
        );
        serde_json::from_str(raw).expect("parse happy snapshot")
    }

    fn failed_snapshot() -> SessionSnapshot {
        serde_json::from_str(
            r#"{"id":"conv_fail","agent_id":"ag_x","status":"failed","created_at":0}"#,
        )
        .expect("parse minimal failed snapshot")
    }

    fn happy_items() -> ItemList {
        let raw =
            include_str!("../../../../docs/spikes/captures/2026-06-26-sse/happy_path.items.json");
        serde_json::from_str(raw).expect("parse happy items")
    }

    fn output_text_delta_frame(seq: u64, delta: &str) -> Vec<u8> {
        format!(
            "event: response.output_text.delta\ndata: {{\"sequence_number\": {seq}, \"type\": \"response.output_text.delta\", \"delta\": {delta:?}}}\n\n"
        )
        .into_bytes()
    }

    struct MockReopen {
        snapshot: SessionSnapshot,
        snapshot_auth_401: bool,
        items: Mutex<Option<ItemList>>,
        items_retry_503_first: bool,
        items_call_count: Mutex<u32>,
        bodies: Mutex<Vec<Vec<u8>>>,
        open_stream_always_503: bool,
    }

    impl Reopen for MockReopen {
        fn open_stream(&self) -> crate::error::Result<Box<dyn Read + Send>> {
            let mut bodies = self.bodies.lock().unwrap();
            if !bodies.is_empty() {
                let body = bodies.remove(0);
                return Ok(Box::new(Cursor::new(body)));
            }
            if self.open_stream_always_503 {
                return Err(ClientError::Server {
                    status: 503,
                    body: serde_json::json!({}),
                });
            }
            Err(ClientError::Server {
                status: 503,
                body: serde_json::json!({}),
            })
        }

        fn snapshot(&self) -> crate::error::Result<SessionSnapshot> {
            if self.snapshot_auth_401 {
                return Err(ClientError::Auth { status: 401 });
            }
            Ok(self.snapshot.clone())
        }

        fn items(&self) -> crate::error::Result<ItemList> {
            if self.items_retry_503_first {
                let mut count = self.items_call_count.lock().unwrap();
                *count += 1;
                if *count == 1 {
                    return Err(ClientError::Server {
                        status: 503,
                        body: serde_json::json!({}),
                    });
                }
            }
            match self.items.lock().unwrap().take() {
                Some(list) => Ok(list),
                None => Err(ClientError::Server {
                    status: 503,
                    body: serde_json::json!({}),
                }),
            }
        }
    }

    fn idx_reconnecting(events: &[ServerStreamEvent], attempt: u32) -> Option<usize> {
        events.iter().position(|e| {
            matches!(
                e,
                ServerStreamEvent::Reconnecting { attempt: a } if *a == attempt
            )
        })
    }

    fn idx_reconnected(events: &[ServerStreamEvent]) -> Option<usize> {
        events
            .iter()
            .position(|e| matches!(e, ServerStreamEvent::Reconnected { gap: None }))
    }

    fn idx_snapshot_restored(events: &[ServerStreamEvent]) -> Option<usize> {
        events
            .iter()
            .position(|e| matches!(e, ServerStreamEvent::SnapshotRestored(_)))
    }

    fn idx_first_replayed_item(events: &[ServerStreamEvent]) -> Option<usize> {
        events.iter().position(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
            )
        })
    }

    fn idx_live_delta(events: &[ServerStreamEvent], delta: &str) -> Option<usize> {
        events.iter().position(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { delta: d, .. })
                    if d == delta
            )
        })
    }

    #[test]
    fn drop_then_reconnect_emits_lifecycle_in_order() {
        let body1 = output_text_delta_frame(42, "before drop");
        let body2 = output_text_delta_frame(99, "after reopen");
        let mock = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(Some(happy_items())),
            items_retry_503_first: false,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![body2]),
            open_stream_always_503: false,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        read_loop(
            Box::new(Cursor::new(body1)) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();

        let r1 = idx_reconnecting(&events, 1).expect("Reconnecting{1}");
        let rec = idx_reconnected(&events).expect("Reconnected");
        let snap = idx_snapshot_restored(&events).expect("SnapshotRestored");
        let replay = idx_first_replayed_item(&events).expect("replayed OutputItemDone");
        let live = idx_live_delta(&events, "after reopen").expect("live frame after reopen");

        assert!(r1 < rec);
        assert!(rec < snap);
        assert!(snap < replay);
        assert!(replay < live);
    }

    #[test]
    fn unauthorized_snapshot_emits_disconnected_unauthorized_and_stops() {
        let mock = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: true,
            items: Mutex::new(None),
            items_retry_503_first: false,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![]),
            open_stream_always_503: false,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        read_loop(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();

        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ServerStreamEvent::Reconnected { .. }))
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ServerStreamEvent::SnapshotRestored(_)))
        );
        assert!(matches!(
            events.last(),
            Some(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized
            })
        ));
    }

    #[test]
    fn failed_status_snapshot_emits_snapshot_then_disconnected() {
        let mock = MockReopen {
            snapshot: failed_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(None),
            items_retry_503_first: false,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![]),
            open_stream_always_503: false,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        read_loop(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();

        let snap = idx_snapshot_restored(&events).expect("SnapshotRestored");
        let disc = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Disconnected {
                        reason: DisconnectReason::SessionFailed
                    }
                )
            })
            .expect("Disconnected SessionFailed");

        assert!(snap < disc);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ServerStreamEvent::Reconnected { .. }))
        );
        assert!(!events.iter().any(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
            )
        }));
    }

    #[test]
    fn retryable_items_failure_does_not_drop_the_reopened_body() {
        let reopen_body = output_text_delta_frame(99, "live after items retry");
        let mock = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(Some(happy_items())),
            items_retry_503_first: true,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![reopen_body]),
            open_stream_always_503: false,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        read_loop(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();

        assert!(idx_reconnecting(&events, 1).is_some());
        assert!(idx_reconnecting(&events, 2).is_some());
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ServerStreamEvent::Reconnected { gap: None }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ServerStreamEvent::SnapshotRestored(_)))
                .count(),
            1
        );
        assert!(
            idx_live_delta(&events, "live after items retry").is_some(),
            "live delta from open_stream body must be delivered"
        );
    }

    #[test]
    fn exhausted_backoff_emits_retries_exhausted() {
        let mock = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(None),
            items_retry_503_first: false,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![]),
            open_stream_always_503: true,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        read_loop(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
            &stop,
        );
        let events: Vec<_> = rx.iter().collect();

        let reconnects: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                ServerStreamEvent::Reconnecting { attempt } => Some(*attempt),
                _ => None,
            })
            .collect();
        assert_eq!(reconnects.len(), BACKOFF_MS.len());
        for (i, attempt) in reconnects.iter().enumerate() {
            assert_eq!(*attempt, i as u32 + 1);
        }
        assert!(matches!(
            events.last(),
            Some(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::RetriesExhausted
            })
        ));
    }

    #[test]
    fn first_open_emits_snapshot_then_items_before_live_tail() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let reopener = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(Some(happy_items())),
            items_retry_503_first: false,
            items_call_count: Mutex::new(0),
            bodies: Mutex::new(vec![]),
            open_stream_always_503: true,
        };
        let (tx, rx) = bounded(EVENT_CHANNEL_BOUND);
        let body: Box<dyn Read + Send> = Box::new(Cursor::new(Vec::<u8>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        run(body, tx, reopener, |_d| {}, &stop);
        let evs: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(evs[0], ServerStreamEvent::SnapshotRestored(_)),
            "first: {:?}",
            evs[0]
        );
        assert!(
            matches!(
                evs[1],
                ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
            ),
            "second: {:?}",
            evs[1]
        );
    }
}
