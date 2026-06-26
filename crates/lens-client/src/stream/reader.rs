//! The SSE reader thread: holds the blocking reqwest body, feeds the pure
//! parser, deserializes typed events, and pushes them down an mpsc channel.
//! One thread per active session (typed-client.md §4); the gpui poller drains
//! via `try_recv` off `cx.background_spawn`. Never blocks the foreground thread.

use std::io::Read;
use std::sync::mpsc;
use std::thread::JoinHandle;

use super::event::{ServerStreamEvent, parse_event};
use super::normalize::Normalizer;
use super::sse::SseParser;

pub struct EventStream {
    rx: mpsc::Receiver<ServerStreamEvent>,
    _handle: JoinHandle<()>,
}

impl EventStream {
    /// Spawn the reader thread over an open blocking response body.
    pub(crate) fn spawn(resp: reqwest::blocking::Response) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("lens-sse-reader".into())
            .spawn(move || run(resp, tx))
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

fn run<R: Read>(mut reader: R, tx: mpsc::Sender<ServerStreamEvent>) {
    let mut parser = SseParser::default();
    let mut normalizer = Normalizer::default();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // clean EOF — fall through to finish + flush
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    for ev in normalizer.push(parse_event(&frame)) {
                        if tx.send(ev).is_err() {
                            return; // consumer dropped EventStream — stop reading
                        }
                    }
                }
            }
            // Transport error: the stream was interrupted, not closed. Do NOT
            // flush a synthetic ReasoningClosed — the reasoning bracket did not
            // end, the connection did. Plan 3b-2 reconnect attaches here.
            Err(_) => return,
        }
    }
    // Clean EOF only: flush any trailing frame + close a dangling reasoning bracket (§7a).
    for frame in parser.finish() {
        for ev in normalizer.push(parse_event(&frame)) {
            let _ = tx.send(ev);
        }
    }
    for ev in normalizer.flush() {
        let _ = tx.send(ev);
    }
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
        run(reader, tx);
        let events: Vec<_> = rx.iter().collect();
        assert!(!events.iter().any(|e| {
            matches!(
                e,
                ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })
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
        run(reader, tx);
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
    }
}
