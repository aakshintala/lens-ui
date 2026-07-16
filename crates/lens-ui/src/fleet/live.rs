//! Real-scheduler wiring — mirrors `lens-drive` `attach_actor` / `start_stream_bridge`.

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::stream::EventStream;
use lens_core::actor::ActorStores;
use lens_core::clock::Clock;
use lens_core::persist::{SqliteControlStore, SqliteTranscriptStore};
use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub(crate) const EVENT_CHANNEL_BOUND: usize = 1024;

/// Live wall clock for the scheduler — reads `SystemTime` on each call.
pub(crate) struct WallClock;

impl Clock for WallClock {
    fn now_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

pub(crate) struct StreamBridge {
    stream: Arc<EventStream>,
    forwarder: Option<JoinHandle<()>>,
}

impl StreamBridge {
    pub(crate) fn shutdown(&mut self) {
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

pub(crate) fn start_stream_bridge(
    stream: EventStream,
) -> (
    StreamBridge,
    crossbeam_channel::Receiver<lens_client::stream::ServerStreamEvent>,
) {
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

pub(crate) fn open_stores(
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
