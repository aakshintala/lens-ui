//! Engine worker run-loop — owns the non-`Send` [`VtEngine`] on a pinned OS thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};

use super::command::{InputAck, KeyInput, PasteInput};
use super::frame::Frame;
use super::inspect::InspectShared;
use super::presentation::{EnginePresentationEvent, PRESENTATION_CHANNEL_CAP};
use super::vt::{EngineConfig, VtEngine};

pub(crate) type WakerSlot = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

const CMD_CHANNEL_CAP: usize = 256;
pub(crate) const EGRESS_CHANNEL_CAP: usize = 64;
/// Default min interval between frame builds (~60 Hz).
const DEFAULT_BUILD_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) const MAX_FEED_CHUNK: usize = 4096;
const PENDING_PROBE_CAP: usize = 32;

/// Deterministic sync for Stop-preempt-between-chunks test (`cfg(test)` only).
pub(crate) struct TestChunkBarrier {
    #[cfg(test)]
    armed: AtomicBool,
    #[cfg(test)]
    after_chunk0_tx: Mutex<Option<Sender<()>>>,
    #[cfg(test)]
    after_chunk0_rx: Mutex<Option<crossbeam_channel::Receiver<()>>>,
    #[cfg(test)]
    release: AtomicBool,
}

impl TestChunkBarrier {
    pub fn new() -> Self {
        Self {
            #[cfg(test)]
            armed: AtomicBool::new(false),
            #[cfg(test)]
            after_chunk0_tx: Mutex::new(None),
            #[cfg(test)]
            after_chunk0_rx: Mutex::new(None),
            #[cfg(test)]
            release: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub fn arm(&self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        *self.after_chunk0_tx.lock().expect("chunk barrier tx") = Some(tx);
        *self.after_chunk0_rx.lock().expect("chunk barrier rx") = Some(rx);
        self.release.store(false, Ordering::Release);
        self.armed.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub fn wait_after_first_chunk(&self) {
        let rx = self
            .after_chunk0_rx
            .lock()
            .expect("chunk barrier rx")
            .take()
            .expect("chunk barrier not armed");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("timeout waiting for worker after chunk 0");
    }

    #[cfg(test)]
    pub fn release(&self) {
        self.release.store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn signal_and_wait_after_chunk0(&self) {
        if let Some(tx) = self
            .after_chunk0_tx
            .lock()
            .expect("chunk barrier tx")
            .take()
        {
            let _ = tx.send(());
        }
        while !self.release.load(Ordering::Acquire) {
            thread::yield_now();
        }
        self.armed.store(false, Ordering::Release);
    }
}

/// One unit of engine→transport egress, routed to the bridge for the connection that
/// is currently attached. Each connection owns its own channel (C2): residue from a
/// prior connection lives in that connection's channel and is never delivered to a
/// different one.
pub struct EgressFrame {
    pub kind: EgressKind,
    pub bytes: Vec<u8>,
}

/// `Input` = encoded user keystrokes / committed text — a stale drop is user-visible
/// data loss (surfaced). `Other` = focus reports and DA/DSR replies — protocol
/// housekeeping, dropped silently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgressKind {
    Input,
    Other,
}

pub(crate) enum EngineCommand {
    Feed(Vec<u8>),
    Key(KeyInput),
    Paste(PasteInput),
    Focus {
        focused: bool,
        report: bool,
        access_epoch: u64,
    },
    LocalScroll(super::command::ScrollDelta),
    Resize(u16, u16),
    SetVisible(bool),
    /// Test/deterministic helper — bypass the time throttle for one publish pass.
    BuildNow,
    SetEgress(Option<Sender<EgressFrame>>),
    Stop,
}

pub(crate) struct WorkerChannels {
    pub cmd_tx: Sender<EngineCommand>,
    pub cmd_rx: Receiver<EngineCommand>,
    pub presentation_tx: Sender<EnginePresentationEvent>,
    pub presentation_rx: Receiver<EnginePresentationEvent>,
}

pub(crate) fn worker_channels() -> WorkerChannels {
    let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(CMD_CHANNEL_CAP);
    let (presentation_tx, presentation_rx) = crossbeam_channel::bounded(PRESENTATION_CHANNEL_CAP);
    WorkerChannels {
        cmd_tx,
        cmd_rx,
        presentation_tx,
        presentation_rx,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "worker thread wires many channels and shared state"
)]
pub(crate) fn spawn_worker(
    cfg: EngineConfig,
    cmd_rx: Receiver<EngineCommand>,
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
    inspect: Arc<InspectShared>,
    // Per-worker test hook: the next N `build_frame` attempts fail synthetically.
    // Per-handle (not a process-global) so parallel tests can't consume each
    // other's injected failures. Zero-cost in production (never read).
    test_build_failures: Arc<AtomicUsize>,
    #[cfg(any(test, feature = "test-util"))] worker_stall_gate: Arc<AtomicBool>,
    chunk_barrier: Arc<TestChunkBarrier>,
    access_epoch: Arc<AtomicU64>,
    presentation_tx: Sender<EnginePresentationEvent>,
    latest_title_slot: Arc<ArcSwapOption<super::presentation::TitleUpdate>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut engine = match VtEngine::new_shared(
            &cfg,
            |_| {},
            presentation_tx,
            latest_title_slot,
            Some(Arc::clone(&waker)),
            Some(Arc::clone(&inspect)),
        ) {
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
        let mut pending: VecDeque<EngineCommand> = VecDeque::new();
        let mut last_build = Instant::now()
            .checked_sub(DEFAULT_BUILD_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut egress: Option<Sender<EgressFrame>> = None;

        loop {
            #[cfg(any(test, feature = "test-util"))]
            while worker_stall_gate.load(Ordering::Acquire) {
                thread::yield_now();
            }

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
                    dispatch_command(
                        cmd,
                        &mut engine,
                        &cmd_rx,
                        &mut egress,
                        &inspect,
                        &mut dirty,
                        &mut visible,
                        &mut force_build,
                        &mut pending,
                        &mut stopping,
                        &chunk_barrier,
                        &access_epoch,
                    );
                }
            }

            while let Ok(cmd) = cmd_rx.try_recv() {
                if matches!(cmd, EngineCommand::Stop) {
                    stopping = true;
                    break;
                }
                dispatch_command(
                    cmd,
                    &mut engine,
                    &cmd_rx,
                    &mut egress,
                    &inspect,
                    &mut dirty,
                    &mut visible,
                    &mut force_build,
                    &mut pending,
                    &mut stopping,
                    &chunk_barrier,
                    &access_epoch,
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
fn dispatch_command(
    cmd: EngineCommand,
    engine: &mut VtEngine,
    cmd_rx: &Receiver<EngineCommand>,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
    pending: &mut VecDeque<EngineCommand>,
    stopping: &mut bool,
    chunk_barrier: &TestChunkBarrier,
    access_epoch: &AtomicU64,
) {
    match cmd {
        EngineCommand::Feed(bytes) => handle_feed_chunked(
            bytes,
            engine,
            cmd_rx,
            egress,
            inspect,
            dirty,
            visible,
            force_build,
            pending,
            stopping,
            chunk_barrier,
            access_epoch,
        ),
        EngineCommand::Stop => *stopping = true,
        other => handle_command(
            other,
            engine,
            egress,
            inspect,
            dirty,
            visible,
            force_build,
            access_epoch,
        ),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "feed chunk loop threads engine + channel handles"
)]
fn handle_feed_chunked(
    bytes: Vec<u8>,
    engine: &mut VtEngine,
    cmd_rx: &Receiver<EngineCommand>,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
    pending: &mut VecDeque<EngineCommand>,
    stopping: &mut bool,
    chunk_barrier: &TestChunkBarrier,
    access_epoch: &AtomicU64,
) {
    let total = bytes.len();
    let mut offset = 0usize;
    #[cfg(test)]
    let chunk_count = total.div_ceil(MAX_FEED_CHUNK);
    #[cfg(test)]
    let mut chunk_idx = 0usize;

    while offset < total {
        if *stopping {
            inspect.record_stop_preempt();
            return;
        }

        let end = (offset + MAX_FEED_CHUNK).min(total);
        feed_chunk(&bytes[offset..end], engine, egress.as_ref(), inspect, dirty);
        offset = end;

        #[cfg(test)]
        {
            if chunk_barrier.armed.load(Ordering::Acquire) && chunk_count >= 2 && chunk_idx == 0 {
                chunk_barrier.signal_and_wait_after_chunk0();
            }
            chunk_idx += 1;
        }

        probe_pending_during_feed(cmd_rx, pending, stopping);
        if *stopping {
            inspect.record_stop_preempt();
            return;
        }
    }

    drain_pending_after_feed(
        pending,
        engine,
        cmd_rx,
        egress,
        inspect,
        dirty,
        visible,
        force_build,
        stopping,
        chunk_barrier,
        access_epoch,
    );
}

fn feed_chunk(
    chunk: &[u8],
    engine: &mut VtEngine,
    egress: Option<&Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
) {
    inspect.record_feed_chunk();
    let n = chunk.len() as u64;
    engine.feed(chunk);
    inspect.record_bytes_fed(n);
    let replies = engine.take_replies();
    if !replies.is_empty() {
        inspect.record_egress(replies.len());
        emit_reply_egress(egress, replies);
    }
    *dirty = true;
}

fn probe_pending_during_feed(
    cmd_rx: &Receiver<EngineCommand>,
    pending: &mut VecDeque<EngineCommand>,
    stopping: &mut bool,
) {
    for _ in 0..PENDING_PROBE_CAP {
        match cmd_rx.try_recv() {
            Ok(EngineCommand::Stop) => {
                *stopping = true;
                return;
            }
            Ok(cmd) => pending.push_back(cmd),
            Err(_) => break,
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "pending drain re-dispatches through the command path"
)]
fn drain_pending_after_feed(
    pending: &mut VecDeque<EngineCommand>,
    engine: &mut VtEngine,
    cmd_rx: &Receiver<EngineCommand>,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
    stopping: &mut bool,
    chunk_barrier: &TestChunkBarrier,
    access_epoch: &AtomicU64,
) {
    while let Some(cmd) = pending.pop_front() {
        if *stopping {
            pending.push_front(cmd);
            return;
        }
        dispatch_command(
            cmd,
            engine,
            cmd_rx,
            egress,
            inspect,
            dirty,
            visible,
            force_build,
            pending,
            stopping,
            chunk_barrier,
            access_epoch,
        );
        if *stopping {
            return;
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "command dispatch threads engine + I/O handles"
)]
fn handle_command(
    cmd: EngineCommand,
    engine: &mut VtEngine,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    visible: &mut bool,
    force_build: &mut bool,
    access_epoch: &AtomicU64,
) {
    let current_epoch = access_epoch.load(Ordering::Acquire);

    match cmd {
        EngineCommand::Feed(_) => {
            debug_assert!(false, "Feed must go through handle_feed_chunked");
        }
        EngineCommand::SetEgress(tx) => {
            *egress = tx;
        }
        EngineCommand::Key(mut input) => {
            let cmd_epoch = input.access_epoch;
            let ack_tx = input.ack.take();
            let (encoded, accepted) = if cmd_epoch != current_epoch {
                (Vec::new(), false)
            } else {
                match engine.encode_key(&input) {
                    Ok(bytes) if bytes.is_empty() => (bytes, true),
                    Ok(bytes) => {
                        if cmd_epoch != access_epoch.load(Ordering::Acquire) {
                            (Vec::new(), false)
                        } else {
                            inspect.record_keys_encoded();
                            let delivered =
                                try_emit_user_input(egress.as_ref(), EgressKind::Input, &bytes);
                            if delivered {
                                inspect.record_user_egress_accepted();
                            } else {
                                inspect.record_user_egress_rejected();
                            }
                            (bytes, delivered)
                        }
                    }
                    Err(e) => {
                        eprintln!("lens-terminal engine: encode_key failed: {e}");
                        (Vec::new(), false)
                    }
                }
            };
            if let Some(tx) = ack_tx {
                let _ = tx.try_send(InputAck { encoded, accepted });
            }
        }
        EngineCommand::Paste(mut input) => {
            let cmd_epoch = input.access_epoch;
            let ack_tx = input.ack.take();
            let (encoded, accepted) = if cmd_epoch != current_epoch {
                (Vec::new(), false)
            } else {
                match engine.encode_paste(&input.bytes) {
                    Ok(bytes) => {
                        if cmd_epoch != access_epoch.load(Ordering::Acquire) {
                            (Vec::new(), false)
                        } else if bytes.is_empty() {
                            (bytes, true)
                        } else {
                            let delivered =
                                try_emit_user_input(egress.as_ref(), EgressKind::Input, &bytes);
                            if delivered {
                                inspect.record_user_egress_accepted();
                            } else {
                                inspect.record_user_egress_rejected();
                            }
                            (bytes, delivered)
                        }
                    }
                    Err(e) => {
                        eprintln!("lens-terminal engine: encode_paste failed: {e}");
                        (Vec::new(), false)
                    }
                }
            };
            if let Some(tx) = ack_tx {
                let _ = tx.try_send(InputAck { encoded, accepted });
            }
        }
        EngineCommand::Focus {
            focused,
            report,
            access_epoch: cmd_epoch,
        } => {
            if !report || cmd_epoch != current_epoch {
                return;
            }
            match engine.encode_focus_report(focused) {
                Ok(Some(bytes)) => {
                    if cmd_epoch != access_epoch.load(Ordering::Acquire) {
                        return;
                    }
                    inspect.record_keys_encoded();
                    let delivered = try_emit_user_input(egress.as_ref(), EgressKind::Other, &bytes);
                    if delivered {
                        inspect.record_user_egress_accepted();
                    } else {
                        inspect.record_user_egress_rejected();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("lens-terminal engine: encode_focus_report failed: {e}");
                }
            }
        }
        EngineCommand::LocalScroll(delta) => {
            engine.local_scroll(delta);
            *dirty = true;
            *force_build = true;
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

/// Non-blocking emit for DA/DSR replies; best-effort when egress is saturated.
fn emit_reply_egress(tx: Option<&Sender<EgressFrame>>, replies: Vec<u8>) {
    let Some(tx) = tx else { return };
    let _ = tx.try_send(EgressFrame {
        kind: EgressKind::Other,
        bytes: replies,
    });
}

/// Returns true iff `bytes` were handed to a live egress channel.
fn try_emit_user_input(tx: Option<&Sender<EgressFrame>>, kind: EgressKind, bytes: &[u8]) -> bool {
    let Some(tx) = tx else {
        return false;
    };
    match tx.try_send(EgressFrame {
        kind,
        bytes: bytes.to_vec(),
    }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,
        Err(TrySendError::Disconnected(_)) => false,
    }
}
