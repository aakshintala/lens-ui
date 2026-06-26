//! The SSE reader thread: holds the blocking reqwest body, feeds the pure
//! parser, deserializes typed events, and pushes them down an mpsc channel.
//! One thread per active session (typed-client.md §4); the gpui poller drains
//! via `try_recv` off `cx.background_spawn`. Never blocks the foreground thread.

use std::io::Read;
use std::sync::mpsc;
use std::thread::JoinHandle;

use super::event::{ServerStreamEvent, parse_event};
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

fn run(mut resp: reqwest::blocking::Response, tx: mpsc::Sender<ServerStreamEvent>) {
    let mut parser = SseParser::default();
    let mut buf = [0u8; 8192];
    loop {
        match resp.read(&mut buf) {
            Ok(0) => break, // server closed the stream
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    if tx.send(parse_event(&frame)).is_err() {
                        return; // consumer dropped EventStream — stop reading
                    }
                }
            }
            Err(_) => break, // network error: close the channel (Plan 3b reconnects)
        }
    }
    for frame in parser.finish() {
        let _ = tx.send(parse_event(&frame));
    }
}
