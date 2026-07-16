//! Engine worker run-loop — owns the non-`Send` [`VtEngine`] on a pinned OS thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Receiver, Sender, TrySendError};

use super::frame::Frame;
use super::vt::{EngineConfig, VtEngine};

pub(crate) type WakerSlot = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;

const CMD_CHANNEL_CAP: usize = 256;
const DA_DSR_CHANNEL_CAP: usize = 64;
/// Default min interval between frame builds (~60 Hz).
const DEFAULT_BUILD_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) enum EngineCommand {
    Feed(Vec<u8>),
    Resize(u16, u16),
    SetVisible(bool),
    /// Test/deterministic helper — bypass the time throttle for one publish pass.
    BuildNow,
    Stop,
}

pub(crate) struct WorkerChannels {
    pub cmd_tx: Sender<EngineCommand>,
    pub cmd_rx: Receiver<EngineCommand>,
    pub da_dsr_tx: Sender<Vec<u8>>,
    pub da_dsr_rx: Receiver<Vec<u8>>,
}

pub(crate) fn worker_channels() -> WorkerChannels {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(CMD_CHANNEL_CAP);
    let (da_dsr_tx, da_dsr_rx) = crossbeam_channel::bounded(DA_DSR_CHANNEL_CAP);
    WorkerChannels {
        cmd_tx,
        cmd_rx,
        da_dsr_tx,
        da_dsr_rx,
    }
}

pub(crate) fn spawn_worker(
    cfg: EngineConfig,
    cmd_rx: Receiver<EngineCommand>,
    da_dsr_tx: Sender<Vec<u8>>,
    da_dsr_rx: Receiver<Vec<u8>>,
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut engine = match VtEngine::new(&cfg, |_| {}) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("lens-terminal engine: failed to create VtEngine: {e}");
                return;
            }
        };

        let mut dirty = false;
        let mut visible = true;
        let mut force_build = false;
        let mut last_build = Instant::now()
            .checked_sub(DEFAULT_BUILD_INTERVAL)
            .unwrap_or_else(Instant::now);

        loop {
            match cmd_rx.recv() {
                Ok(EngineCommand::Stop) | Err(_) => break,
                Ok(cmd) => handle_command(
                    cmd,
                    &mut engine,
                    &da_dsr_tx,
                    &da_dsr_rx,
                    &mut dirty,
                    &mut visible,
                    &mut force_build,
                ),
            }

            while let Ok(cmd) = cmd_rx.try_recv() {
                if matches!(cmd, EngineCommand::Stop) {
                    return;
                }
                handle_command(
                    cmd,
                    &mut engine,
                    &da_dsr_tx,
                    &da_dsr_rx,
                    &mut dirty,
                    &mut visible,
                    &mut force_build,
                );
            }

            maybe_publish(
                &mut engine,
                &frame_slot,
                &frame_ready,
                &waker,
                &mut dirty,
                visible,
                &mut force_build,
                &mut last_build,
            );
        }
    })
}

fn handle_command(
    cmd: EngineCommand,
    engine: &mut VtEngine,
    da_dsr_tx: &Sender<Vec<u8>>,
    da_dsr_rx: &Receiver<Vec<u8>>,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
) {
    match cmd {
        EngineCommand::Feed(bytes) => {
            engine.feed(&bytes);
            let replies = engine.take_replies();
            if !replies.is_empty() {
                emit_da_dsr(da_dsr_tx, da_dsr_rx, replies);
            }
            *dirty = true;
        }
        EngineCommand::Resize(cols, rows) => {
            if let Err(e) = engine.resize(cols, rows) {
                eprintln!("lens-terminal engine: resize failed: {e}");
            } else {
                *dirty = true;
            }
        }
        EngineCommand::SetVisible(v) => {
            let was_visible = *visible;
            *visible = v;
            if v && !was_visible {
                *force_build = true;
            }
        }
        EngineCommand::BuildNow => {
            *force_build = true;
        }
        EngineCommand::Stop => {}
    }
}

#[expect(clippy::too_many_arguments, reason = "publish gate threads many engine-owned handles")]
fn maybe_publish(
    engine: &mut VtEngine,
    frame_slot: &Arc<ArcSwapOption<Frame>>,
    frame_ready: &Arc<AtomicBool>,
    waker: &WakerSlot,
    dirty: &mut bool,
    visible: bool,
    force_build: &mut bool,
    last_build: &mut Instant,
) {
    if !*dirty || !visible {
        return;
    }

    let due = *force_build || last_build.elapsed() >= DEFAULT_BUILD_INTERVAL;
    if !due {
        return;
    }

    *force_build = false;

    match engine.build_frame() {
        Ok(frame) => {
            frame_slot.store(Some(Arc::new(frame)));
            frame_ready.store(true, Ordering::Release);
            *dirty = false;
            *last_build = Instant::now();
            if let Ok(guard) = waker.lock()
                && let Some(w) = guard.as_ref()
            {
                w();
            }
        }
        Err(e) => {
            eprintln!("lens-terminal engine: build_frame failed: {e}");
            *dirty = false;
        }
    }
}

/// Non-blocking emit; if the channel is full, drop the oldest reply and retry once.
fn emit_da_dsr(tx: &Sender<Vec<u8>>, rx: &Receiver<Vec<u8>>, replies: Vec<u8>) {
    match tx.try_send(replies) {
        Ok(()) => {}
        Err(TrySendError::Full(replies)) => {
            let _ = rx.try_recv();
            let _ = tx.try_send(replies);
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}
