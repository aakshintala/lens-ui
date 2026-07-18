//! Engine worker run-loop — owns the non-`Send` [`VtEngine`] on a pinned OS thread.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};

use super::command::{KeyInput, ScrollDelta};
use super::frame::Frame;
use super::inspect::InspectShared;
use super::vt::{EngineConfig, VtEngine};

pub(crate) type WakerSlot = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

const CMD_CHANNEL_CAP: usize = 256;
const DA_DSR_CHANNEL_CAP: usize = 64;
/// Default min interval between frame builds (~60 Hz).
const DEFAULT_BUILD_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) enum EngineCommand {
    Feed(Vec<u8>),
    #[expect(dead_code, reason = "Slice 2a Task 1 — Key arm wired in Task 2")]
    Key(KeyInput),
    #[expect(dead_code, reason = "Slice 2a Task 1 — Focus arm wired in Task 5")]
    Focus {
        focused: bool,
        report: bool,
        access_epoch: u64,
    },
    #[expect(
        dead_code,
        reason = "Slice 2a Task 1 — LocalScroll arm wired in Task 2"
    )]
    LocalScroll(ScrollDelta),
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

#[expect(
    clippy::too_many_arguments,
    reason = "worker thread wires many channels and shared state"
)]
pub(crate) fn spawn_worker(
    cfg: EngineConfig,
    cmd_rx: Receiver<EngineCommand>,
    da_dsr_tx: Sender<Vec<u8>>,
    da_dsr_rx: Receiver<Vec<u8>>,
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
    inspect: Arc<InspectShared>,
    // Per-worker test hook: the next N `build_frame` attempts fail synthetically.
    // Per-handle (not a process-global) so parallel tests can't consume each
    // other's injected failures. Zero-cost in production (never read).
    test_build_failures: Arc<AtomicUsize>,
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
        let mut stopping = false;
        let mut last_build = Instant::now()
            .checked_sub(DEFAULT_BUILD_INTERVAL)
            .unwrap_or_else(Instant::now);

        loop {
            let throttle_remaining = DEFAULT_BUILD_INTERVAL.saturating_sub(last_build.elapsed());
            let wait_for_throttle =
                dirty && visible && !force_build && throttle_remaining > Duration::ZERO;

            let cmd = if wait_for_throttle {
                match cmd_rx.recv_timeout(throttle_remaining) {
                    Ok(cmd) => Some(cmd),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match cmd_rx.recv() {
                    Ok(cmd) => Some(cmd),
                    Err(_) => break,
                }
            };

            if let Some(cmd) = cmd {
                if matches!(cmd, EngineCommand::Stop) {
                    stopping = true;
                } else {
                    handle_command(
                        cmd,
                        &mut engine,
                        &da_dsr_tx,
                        &da_dsr_rx,
                        &inspect,
                        &mut dirty,
                        &mut visible,
                        &mut force_build,
                    );
                }
            }

            while let Ok(cmd) = cmd_rx.try_recv() {
                if matches!(cmd, EngineCommand::Stop) {
                    stopping = true;
                    break;
                }
                handle_command(
                    cmd,
                    &mut engine,
                    &da_dsr_tx,
                    &da_dsr_rx,
                    &inspect,
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
                &inspect,
                &mut dirty,
                visible,
                &mut force_build,
                &mut last_build,
                &test_build_failures,
            );

            if stopping {
                if dirty && visible {
                    force_build = true;
                    maybe_publish(
                        &mut engine,
                        &frame_slot,
                        &frame_ready,
                        &waker,
                        &inspect,
                        &mut dirty,
                        visible,
                        &mut force_build,
                        &mut last_build,
                        &test_build_failures,
                    );
                }
                break;
            }
        }
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "command dispatch threads engine + I/O handles"
)]
fn handle_command(
    cmd: EngineCommand,
    engine: &mut VtEngine,
    da_dsr_tx: &Sender<Vec<u8>>,
    da_dsr_rx: &Receiver<Vec<u8>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
) {
    match cmd {
        EngineCommand::Feed(bytes) => {
            let n = bytes.len() as u64;
            engine.feed(&bytes);
            inspect.record_bytes_fed(n);
            let replies = engine.take_replies();
            if !replies.is_empty() {
                inspect.record_da_dsr(replies.len());
                emit_da_dsr(da_dsr_tx, da_dsr_rx, replies);
            }
            *dirty = true;
        }
        EngineCommand::Resize(cols, rows) => {
            if let Err(e) = engine.resize(cols, rows) {
                eprintln!("lens-terminal engine: resize failed: {e}");
            } else {
                inspect.record_resize(cols, rows);
                *dirty = true;
            }
        }
        EngineCommand::SetVisible(v) => {
            let was_visible = *visible;
            *visible = v;
            inspect.set_visible(v);
            if v && !was_visible {
                *dirty = true;
                *force_build = true;
            }
        }
        EngineCommand::BuildNow => {
            *force_build = true;
        }
        EngineCommand::Key(_) | EngineCommand::Focus { .. } | EngineCommand::LocalScroll(_) => {}
        EngineCommand::Stop => {}
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "publish gate threads many engine-owned handles"
)]
// `test_build_failures` is consumed only under `#[cfg(test)]`.
#[cfg_attr(not(test), allow(unused_variables))]
fn maybe_publish(
    engine: &mut VtEngine,
    frame_slot: &Arc<ArcSwapOption<Frame>>,
    frame_ready: &Arc<AtomicBool>,
    waker: &WakerSlot,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: bool,
    force_build: &mut bool,
    last_build: &mut Instant,
    test_build_failures: &Arc<AtomicUsize>,
) {
    if !*dirty || !visible {
        return;
    }

    let due = *force_build || last_build.elapsed() >= DEFAULT_BUILD_INTERVAL;
    if !due {
        return;
    }

    let was_forced = *force_build;

    #[cfg(test)]
    {
        let remaining = test_build_failures.load(Ordering::SeqCst);
        if remaining > 0 {
            test_build_failures.store(remaining - 1, Ordering::SeqCst);
            eprintln!("lens-terminal engine: build_frame failed (test injection)");
            *dirty = true;
            if was_forced {
                *force_build = true;
            }
            return;
        }
    }

    let started = Instant::now();
    match engine.build_frame() {
        Ok(frame) => {
            let micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            inspect.record_frame_built(micros);
            frame_slot.store(Some(Arc::new(frame)));
            frame_ready.store(true, Ordering::Release);
            *dirty = false;
            *force_build = false;
            *last_build = Instant::now();
            let waker_fn = waker
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(Arc::clone));
            if let Some(w) = waker_fn {
                w();
            }
        }
        Err(e) => {
            eprintln!("lens-terminal engine: build_frame failed: {e}");
            *dirty = true;
            if was_forced {
                *force_build = true;
            }
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
