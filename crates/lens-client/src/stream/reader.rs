//! The SSE reader thread: holds the blocking reqwest body, feeds the pure
//! parser, deserializes typed events, and pushes them down an mpsc channel.
//! One thread per active session (typed-client.md §4); the gpui poller drains
//! via `try_recv` off `cx.background_spawn`. Never blocks the foreground thread.

use std::io::Read;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::ClientError;
use crate::reconnect::{BACKOFF_MS, Reopen, items_to_replay};
use crate::sessions::SessionStatus;

use super::event::{DisconnectReason, ServerStreamEvent, parse_event};
use super::normalize::Normalizer;
use super::sse::SseParser;

pub struct EventStream {
    rx: mpsc::Receiver<ServerStreamEvent>,
    _handle: JoinHandle<()>,
}

impl EventStream {
    /// Spawn the reader thread over an open blocking response body.
    pub(crate) fn spawn<Re: Reopen + 'static>(
        resp: reqwest::blocking::Response,
        reopener: Re,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("lens-sse-reader".into())
            .spawn(move || {
                run(Box::new(resp) as Box<dyn Read + Send>, tx, reopener, |d| {
                    std::thread::sleep(d)
                })
            })
            .expect("spawn SSE reader thread");
        EventStream {
            rx,
            _handle: handle,
        }
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

fn run<Re: Reopen>(
    mut body: Box<dyn Read + Send>,
    tx: mpsc::Sender<ServerStreamEvent>,
    reopener: Re,
    sleep: impl Fn(Duration),
) {
    let mut parser = SseParser::default();
    let mut normalizer = Normalizer::default();
    let mut buf = [0u8; 8192];
    let mut last_seen_seq: Option<u64> = None;
    let mut resume_floor: Option<u64> = None; // Some(_) => inside post-reopen dedup window
    loop {
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
                match reconnect(&reopener, &sleep, &tx, &mut normalizer, last_seen_seq) {
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
            Err(_) => match reconnect(&reopener, &sleep, &tx, &mut normalizer, last_seen_seq) {
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
    tx: &mpsc::Sender<ServerStreamEvent>,
    normalizer: &mut Normalizer,
    _last_seen_seq: Option<u64>,
) -> Option<Box<dyn Read + Send>> {
    for (i, &delay) in BACKOFF_MS.iter().enumerate() {
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
            let _ = tx.send(ServerStreamEvent::Reconnected { gap: None });
            normalizer.reset_seen_items();
            let _ = tx.send(ServerStreamEvent::SnapshotRestored(Box::new(snap)));
            let _ = tx.send(ServerStreamEvent::Disconnected {
                reason: DisconnectReason::SessionFailed,
            });
            return None;
        }
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
        if tx
            .send(ServerStreamEvent::Reconnected { gap: None })
            .is_err()
        {
            return None;
        }
        normalizer.reset_seen_items();
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
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(reader) as Box<dyn Read + Send>,
            tx,
            ExhaustReopener {
                snapshot: happy_idle_snapshot(),
            },
            |_| {},
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
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(reader) as Box<dyn Read + Send>,
            tx,
            ExhaustReopener {
                snapshot: happy_idle_snapshot(),
            },
            |_| {},
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
            bodies: Mutex::new(vec![body2]),
            open_stream_always_503: false,
        };
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(Cursor::new(body1)) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
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
            bodies: Mutex::new(vec![]),
            open_stream_always_503: false,
        };
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
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
            bodies: Mutex::new(vec![]),
            open_stream_always_503: false,
        };
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
        );
        let events: Vec<_> = rx.iter().collect();

        let rec = idx_reconnected(&events).expect("Reconnected");
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

        assert!(rec < snap);
        assert!(snap < disc);
        assert!(!events.iter().any(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
            )
        }));
    }

    #[test]
    fn exhausted_backoff_emits_retries_exhausted() {
        let mock = MockReopen {
            snapshot: happy_idle_snapshot(),
            snapshot_auth_401: false,
            items: Mutex::new(None),
            bodies: Mutex::new(vec![]),
            open_stream_always_503: true,
        };
        let (tx, rx) = mpsc::channel();
        run(
            Box::new(Cursor::new(Vec::<u8>::new())) as Box<dyn Read + Send>,
            tx,
            mock,
            |_| {},
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
}
