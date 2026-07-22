//! Engine worker run-loop — owns the non-`Send` [`VtEngine`] on a pinned OS thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};

use super::command::{
    CopyResponder, CopyResult, GestureDisposition, GestureOwner, InputAck, KeyInput, MouseAck,
    MouseButtonKind, MouseEventKind, MouseGesture, MouseReportEv, MouseReportPolicy, MouseTracking,
    PasteInput, ScrollDelta, WheelInput,
};
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
/// Upper bound on wheel notches reported for a single `Wheel` command. A physical
/// notch is 1-few lines; this caps a pathological delta so the report loop can never
/// spin unboundedly (codex whole-slice F3).
const MAX_WHEEL_NOTCHES: u32 = 32;

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
    #[allow(dead_code, reason = "foreground lowering lands in Task 5")]
    MouseGesture(MouseGesture),
    #[allow(dead_code, reason = "foreground lowering lands in Task 5")]
    Wheel(WheelInput),
    SelectAll,
    Copy(CopyResponder),
    Focus {
        focused: bool,
        report: bool,
        access_epoch: u64,
    },
    #[allow(
        dead_code,
        reason = "superseded by Wheel arm in Task 5 foreground lowering"
    )]
    LocalScroll(super::command::ScrollDelta),
    Resize(u16, u16),
    SetVisible(bool),
    /// Test/deterministic helper — bypass the time throttle for one publish pass.
    BuildNow,
    SetEgress(Option<Sender<EgressFrame>>),
    /// Ordered access gate — foreground sends on open and every access change (Task 5).
    SetAccess(bool),
    Stop,
}

#[derive(Debug)]
struct Latch {
    owner: GestureOwner,
    button: MouseButtonKind,
    epoch: u64,
    suppressed: bool,
    dragged: bool,
    /// Click token from the Down; echoed on `LocalClick` for frame correlation (codex F2).
    click_seq: u64,
}

#[derive(Debug)]
struct MouseState {
    latch: Option<Latch>,
    any_button_pressed: bool,
    /// Engine-authoritative write gate at this command's stream position.
    write_allowed: bool,
}

impl Default for MouseState {
    fn default() -> Self {
        Self {
            latch: None,
            any_button_pressed: false,
            write_allowed: true,
        }
    }
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
        let presentation_tx_for_local_click = presentation_tx.clone();
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
        let mut mouse_state = MouseState::default();

        loop {
            #[cfg(any(test, feature = "test-util"))]
            while worker_stall_gate.load(Ordering::Acquire) {
                // Sleep rather than busy-spin: a held worker must not monopolize a core, or
                // (with many engines under `cargo test` parallelism) it starves other
                // engines' build workers past their deadlines and flakes the suite.
                thread::sleep(Duration::from_millis(1));
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
                        &presentation_tx_for_local_click,
                        &mut mouse_state,
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
                    &presentation_tx_for_local_click,
                    &mut mouse_state,
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
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
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
            presentation_tx,
            mouse_state,
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
            presentation_tx,
            mouse_state,
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
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
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
        presentation_tx,
        mouse_state,
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
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
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
            presentation_tx,
            mouse_state,
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
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
) {
    let current_epoch = access_epoch.load(Ordering::Acquire);

    match cmd {
        EngineCommand::Feed(_) => {
            debug_assert!(false, "Feed must go through handle_feed_chunked");
        }
        EngineCommand::SetEgress(tx) => {
            *egress = tx;
        }
        EngineCommand::MouseGesture(g) => handle_mouse_gesture(
            g,
            engine,
            egress,
            inspect,
            dirty,
            force_build,
            access_epoch,
            current_epoch,
            presentation_tx,
            mouse_state,
        ),
        EngineCommand::Wheel(mut w) => {
            let ack_tx = w.ack.take();
            let write_allowed = mouse_state.write_allowed;
            let report = write_allowed
                && engine.read_live_tracking() != MouseTracking::None
                && w.access_epoch == current_epoch;
            let (encoded, disposition) = if report {
                // Direction MUST match the local-scroll path so the same physical gesture
                // reports the direction it would scroll. `local_scroll` feeds `w.lines` to
                // `ScrollViewport::Delta`, where negative == up (into scrollback). So a
                // wheel-up gesture (negative lines) reports Button::Four. (codex T5 review.)
                let up = w.lines < 0;
                // Bound the notch count so a pathological scroll delta cannot spin the
                // worker for billions of iterations (codex whole-slice F3).
                let notches = w.lines.unsigned_abs().min(MAX_WHEEL_NOTCHES);
                let mut encoded = Vec::new();
                let mut emitted = 0u32;
                for _ in 0..notches {
                    match engine.encode_mouse_report(&MouseReportEv {
                        action: MouseEventKind::Down,
                        button: None,
                        wheel: Some(up),
                        mods: w.mods,
                        px_x: w.px_x,
                        px_y: w.px_y,
                        any_button_pressed: false,
                    }) {
                        Ok(bytes) => {
                            // Re-check the LIVE epoch AFTER encoding, IMMEDIATELY before
                            // egress: a downgrade during encode must not leak this notch
                            // (parity with emit_mouse_report; codex F1 + re-review).
                            if w.access_epoch != access_epoch.load(Ordering::Acquire) {
                                break;
                            }
                            let _ = try_emit_user_input(egress.as_ref(), EgressKind::Input, &bytes);
                            encoded = bytes;
                            emitted += 1;
                        }
                        Err(e) => {
                            eprintln!(
                                "lens-terminal engine: encode_mouse_report (wheel) failed: {e}"
                            );
                            break;
                        }
                    }
                }
                // If every notch was revoked before egress, report honestly as Suppressed
                // rather than a phantom Reported (codex re-review).
                if emitted > 0 {
                    inspect.record_wheel_reported();
                    (encoded, GestureDisposition::Reported)
                } else {
                    inspect.record_mouse_suppressed();
                    (Vec::new(), GestureDisposition::Suppressed)
                }
            } else {
                engine.local_scroll(ScrollDelta::Lines(w.lines));
                *dirty = true;
                *force_build = true;
                (Vec::new(), GestureDisposition::ScrolledLocal)
            };
            send_mouse_ack(ack_tx, encoded, disposition);
        }
        EngineCommand::SelectAll => match engine.select_all() {
            Ok(true) => {
                *dirty = true;
                *force_build = true;
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("lens-terminal engine: select_all failed: {e}");
            }
        },
        EngineCommand::Copy(responder) => {
            inspect.record_copy_started();
            let text = engine.extract_selection_text();
            match &text {
                Some(_) => inspect.record_copy_completed(),
                None => inspect.record_copy_empty(),
            }
            let _ = responder.try_send(CopyResult { text });
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
        EngineCommand::SetAccess(writable) => {
            mouse_state.write_allowed = writable;
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
            inspect.record_retained_rows(engine.total_rows());
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

fn send_mouse_ack(
    ack_tx: Option<Sender<MouseAck>>,
    encoded: Vec<u8>,
    disposition: GestureDisposition,
) {
    if let Some(tx) = ack_tx {
        let _ = tx.try_send(MouseAck {
            encoded,
            disposition,
        });
    }
}

fn should_report_at_down(g: &MouseGesture, tracking: MouseTracking, write_allowed: bool) -> bool {
    write_allowed
        && tracking != MouseTracking::None
        && !g.mods.shift
        && !g.mouse_local
        && g.policy == MouseReportPolicy::Auto
}

fn report_motion_allowed(tracking: MouseTracking, has_latch: bool) -> bool {
    match tracking {
        MouseTracking::X10 | MouseTracking::Normal => false,
        MouseTracking::Button => has_latch,
        MouseTracking::Any => true,
        MouseTracking::None => false,
    }
}

fn mouse_report_ev(g: &MouseGesture, any_button_pressed: bool) -> MouseReportEv {
    mouse_report_ev_with_button(g, g.button, any_button_pressed)
}

/// Build a report event overriding the button (used when a Button-mode motion omits the
/// button but a latch holds it — codex whole-slice F5).
fn mouse_report_ev_with_button(
    g: &MouseGesture,
    button: Option<MouseButtonKind>,
    any_button_pressed: bool,
) -> MouseReportEv {
    MouseReportEv {
        action: g.kind,
        button,
        wheel: None,
        mods: g.mods,
        px_x: g.px_x,
        px_y: g.px_y,
        any_button_pressed,
    }
}

struct MouseReportOutcome {
    encoded: Vec<u8>,
    disposition: GestureDisposition,
    epoch_revoked: bool,
}

fn apply_emit_epoch_revocation(mouse_state: &mut MouseState, outcome: &MouseReportOutcome) {
    if outcome.epoch_revoked
        && let Some(latch) = mouse_state.latch.as_mut()
    {
        latch.suppressed = true;
    }
}

fn emit_mouse_report(
    engine: &mut VtEngine,
    egress: &Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    access_epoch: &AtomicU64,
    cmd_epoch: u64,
    ev: &MouseReportEv,
) -> MouseReportOutcome {
    if cmd_epoch != access_epoch.load(Ordering::Acquire) {
        inspect.record_mouse_suppressed();
        return MouseReportOutcome {
            encoded: Vec::new(),
            disposition: GestureDisposition::Suppressed,
            epoch_revoked: true,
        };
    }
    match engine.encode_mouse_report(ev) {
        Ok(bytes) if bytes.is_empty() => {
            inspect.record_mouse_report_coalesced();
            MouseReportOutcome {
                encoded: bytes,
                disposition: GestureDisposition::Coalesced,
                epoch_revoked: false,
            }
        }
        Ok(bytes) => {
            if cmd_epoch != access_epoch.load(Ordering::Acquire) {
                inspect.record_mouse_suppressed();
                return MouseReportOutcome {
                    encoded: Vec::new(),
                    disposition: GestureDisposition::Suppressed,
                    epoch_revoked: true,
                };
            }
            inspect.record_mouse_encoded();
            let delivered = try_emit_user_input(egress.as_ref(), EgressKind::Input, &bytes);
            if delivered {
                inspect.record_user_egress_accepted();
            } else {
                inspect.record_user_egress_rejected();
            }
            MouseReportOutcome {
                encoded: bytes,
                disposition: GestureDisposition::Reported,
                epoch_revoked: false,
            }
        }
        Err(e) => {
            eprintln!("lens-terminal engine: encode_mouse_report failed: {e}");
            MouseReportOutcome {
                encoded: Vec::new(),
                disposition: GestureDisposition::Suppressed,
                epoch_revoked: false,
            }
        }
    }
}

fn latch_report_suppressed(
    mouse_state: &mut MouseState,
    inspect: &InspectShared,
) -> (Vec<u8>, GestureDisposition) {
    if let Some(latch) = mouse_state.latch.as_mut() {
        latch.suppressed = true;
    }
    inspect.record_mouse_suppressed();
    (Vec::new(), GestureDisposition::Suppressed)
}

#[expect(
    clippy::too_many_arguments,
    reason = "mouse gesture dispatch threads engine + latch + presentation"
)]
fn handle_mouse_gesture(
    mut g: MouseGesture,
    engine: &mut VtEngine,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    force_build: &mut bool,
    access_epoch: &AtomicU64,
    current_epoch: u64,
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
) {
    let ack_tx = g.ack.take();
    let (encoded, disposition) = match g.kind {
        MouseEventKind::Down => handle_mouse_down(
            &g,
            engine,
            egress,
            inspect,
            dirty,
            force_build,
            access_epoch,
            current_epoch,
            mouse_state,
        ),
        MouseEventKind::Move => handle_mouse_move(
            &g,
            engine,
            egress,
            inspect,
            dirty,
            force_build,
            access_epoch,
            current_epoch,
            mouse_state,
        ),
        MouseEventKind::Up => handle_mouse_up(
            &g,
            engine,
            egress,
            inspect,
            dirty,
            force_build,
            access_epoch,
            current_epoch,
            presentation_tx,
            mouse_state,
        ),
    };
    send_mouse_ack(ack_tx, encoded, disposition);
}

#[expect(
    clippy::too_many_arguments,
    reason = "mouse down threads engine + latch + egress + publish flags"
)]
fn handle_mouse_down(
    g: &MouseGesture,
    engine: &mut VtEngine,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    force_build: &mut bool,
    access_epoch: &AtomicU64,
    current_epoch: u64,
    mouse_state: &mut MouseState,
) -> (Vec<u8>, GestureDisposition) {
    let Some(button) = g.button else {
        return (Vec::new(), GestureDisposition::Ignored);
    };

    // Do not clobber an in-flight gesture with a second button-down (chording): keep the
    // original latch and no-op the new press, rather than orphaning the first gesture
    // (codex whole-slice F7). A single-button model is sufficient for 2c.
    if mouse_state.latch.is_some() {
        return (Vec::new(), GestureDisposition::Ignored);
    }

    let write_allowed = mouse_state.write_allowed;
    let tracking = engine.read_live_tracking();
    if should_report_at_down(g, tracking, write_allowed) {
        mouse_state.latch = Some(Latch {
            owner: GestureOwner::Report,
            button,
            epoch: g.access_epoch,
            suppressed: false,
            dragged: false,
            click_seq: g.click_seq,
        });
        mouse_state.any_button_pressed = true;
        // A new report gesture starts a fresh motion-dedup scope: reset so this gesture's
        // first same-cell motion re-emits regardless of a prior gesture's last cell
        // (codex whole-slice F6, ownership invalidation).
        engine.reset_mouse_coalesce();
        if write_allowed && g.access_epoch == current_epoch {
            let ev = mouse_report_ev(g, true);
            let outcome =
                emit_mouse_report(engine, egress, inspect, access_epoch, g.access_epoch, &ev);
            apply_emit_epoch_revocation(mouse_state, &outcome);
            (outcome.encoded, outcome.disposition)
        } else {
            latch_report_suppressed(mouse_state, inspect)
        }
    } else if button == MouseButtonKind::Left {
        mouse_state.latch = Some(Latch {
            owner: GestureOwner::Select,
            button,
            epoch: g.access_epoch,
            suppressed: false,
            dragged: false,
            click_seq: g.click_seq,
        });
        if let Some((col, row)) = g.cell {
            match engine.apply_selection_press(col, row, g.px_x, g.px_y, g.time) {
                Ok(true) => {
                    *dirty = true;
                    *force_build = true;
                    (Vec::new(), GestureDisposition::Selected)
                }
                Ok(false) => (Vec::new(), GestureDisposition::Ignored),
                Err(e) => {
                    eprintln!("lens-terminal engine: apply_selection_press failed: {e}");
                    mouse_state.latch = None;
                    (Vec::new(), GestureDisposition::Ignored)
                }
            }
        } else {
            (Vec::new(), GestureDisposition::Selected)
        }
    } else {
        (Vec::new(), GestureDisposition::Ignored)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "mouse move threads engine + latch + egress + publish flags"
)]
fn handle_mouse_move(
    g: &MouseGesture,
    engine: &mut VtEngine,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    force_build: &mut bool,
    access_epoch: &AtomicU64,
    current_epoch: u64,
    mouse_state: &mut MouseState,
) -> (Vec<u8>, GestureDisposition) {
    let write_allowed = mouse_state.write_allowed;
    let tracking = engine.read_live_tracking();
    // Tracking turned off invalidates the motion-dedup scope: reset so a later re-enable at
    // the same cell re-emits instead of coalescing against a stale cell (codex F6). RESIDUAL
    // (documented): if a program toggles tracking off->on with NO mouse event during the off
    // window and the very next hover lands on the exact same cell, the report coalesces once.
    // Fully closing that needs a vendored mouse-mode generation hook (deferred).
    if tracking == MouseTracking::None {
        engine.reset_mouse_coalesce();
    }
    if let Some(latch) = mouse_state.latch.as_ref() {
        match latch.owner {
            GestureOwner::Report => {
                if latch.suppressed {
                    return (Vec::new(), GestureDisposition::Suppressed);
                }
                if latch.epoch != current_epoch || !write_allowed {
                    return latch_report_suppressed(mouse_state, inspect);
                }
                if !report_motion_allowed(tracking, true) {
                    return (Vec::new(), GestureDisposition::Ignored);
                }
                // The event may omit the button (some platforms send a plain move during a
                // drag); the latch is authoritative for the held button, so fall back to it
                // rather than dropping the motion in Button mode (codex whole-slice F5).
                let ev = mouse_report_ev_with_button(
                    g,
                    g.button.or(Some(latch.button)),
                    mouse_state.any_button_pressed,
                );
                let outcome =
                    emit_mouse_report(engine, egress, inspect, access_epoch, g.access_epoch, &ev);
                apply_emit_epoch_revocation(mouse_state, &outcome);
                (outcome.encoded, outcome.disposition)
            }
            GestureOwner::Select => {
                if let Some((col, row)) = g.cell {
                    // Do NOT force `latch.dragged` here: a sub-threshold jitter move would
                    // otherwise turn a click into a drag and suppress the hyperlink
                    // LocalClick. Feed the gesture machine and let Ghostty's own drag
                    // threshold (`gesture_dragged`, consulted at Up) decide (codex F8).
                    match engine.apply_selection_drag(col, row, g.px_x, g.px_y) {
                        Ok(true) => {
                            *dirty = true;
                            *force_build = true;
                            (Vec::new(), GestureDisposition::Selected)
                        }
                        Ok(false) => (Vec::new(), GestureDisposition::Ignored),
                        Err(e) => {
                            eprintln!("lens-terminal engine: apply_selection_drag failed: {e}");
                            (Vec::new(), GestureDisposition::Ignored)
                        }
                    }
                } else {
                    (Vec::new(), GestureDisposition::Selected)
                }
            }
        }
    } else if g.button.is_none()
        && tracking == MouseTracking::Any
        && write_allowed
        && g.access_epoch == current_epoch
        && !g.mods.shift
        && !g.mouse_local
        && g.policy == MouseReportPolicy::Auto
    {
        // Buttonless (unlatched) Any-mode motion is subject to the SAME local-override
        // arbitration as a Down: Shift, the mouse-local toggle, or ForceLocal suppress it
        // (codex whole-slice F4). Without this, hover motion leaks to the PTY despite the
        // user's local-selection intent.
        let ev = mouse_report_ev(g, false);
        let outcome = emit_mouse_report(engine, egress, inspect, access_epoch, g.access_epoch, &ev);
        (outcome.encoded, outcome.disposition)
    } else {
        (Vec::new(), GestureDisposition::Ignored)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "mouse up threads engine + latch + egress + presentation"
)]
fn handle_mouse_up(
    g: &MouseGesture,
    engine: &mut VtEngine,
    egress: &mut Option<Sender<EgressFrame>>,
    inspect: &InspectShared,
    dirty: &mut bool,
    force_build: &mut bool,
    access_epoch: &AtomicU64,
    current_epoch: u64,
    presentation_tx: &Sender<EnginePresentationEvent>,
    mouse_state: &mut MouseState,
) -> (Vec<u8>, GestureDisposition) {
    let Some(button) = g.button else {
        return (Vec::new(), GestureDisposition::Ignored);
    };

    let Some(latch) = mouse_state.latch.as_ref() else {
        return (Vec::new(), GestureDisposition::Ignored);
    };

    if latch.button != button {
        return (Vec::new(), GestureDisposition::Ignored);
    }

    let write_allowed = mouse_state.write_allowed;
    let Some(latch) = mouse_state.latch.take() else {
        return (Vec::new(), GestureDisposition::Ignored);
    };
    match latch.owner {
        GestureOwner::Report => {
            mouse_state.any_button_pressed = false;
            if latch.suppressed {
                (Vec::new(), GestureDisposition::Suppressed)
            } else if latch.epoch != current_epoch || !write_allowed {
                inspect.record_mouse_suppressed();
                (Vec::new(), GestureDisposition::Suppressed)
            } else {
                let ev = mouse_report_ev(g, false);
                let outcome =
                    emit_mouse_report(engine, egress, inspect, access_epoch, g.access_epoch, &ev);
                (outcome.encoded, outcome.disposition)
            }
        }
        GestureOwner::Select => {
            let dragged = latch.dragged || engine.gesture_dragged();
            match engine.apply_selection_release(g.cell) {
                Ok(()) => {
                    *dirty = true;
                    *force_build = true;
                    if dragged {
                        (Vec::new(), GestureDisposition::Selected)
                    } else if let Some((col, row)) = g.cell {
                        match engine.clear_selection() {
                            Ok(true) => {
                                // A full presentation channel drops the click: report it
                                // honestly (Ignored + inspect) rather than acking a
                                // LocalClick that never reached the foreground (codex F9).
                                match presentation_tx.try_send(
                                    EnginePresentationEvent::LocalClick {
                                        col,
                                        row,
                                        seq: latch.click_seq,
                                    },
                                ) {
                                    Ok(()) => (Vec::new(), GestureDisposition::LocalClick),
                                    Err(_) => {
                                        inspect.record_local_click_dropped();
                                        (Vec::new(), GestureDisposition::Ignored)
                                    }
                                }
                            }
                            _ => (Vec::new(), GestureDisposition::Ignored),
                        }
                    } else {
                        (Vec::new(), GestureDisposition::Ignored)
                    }
                }
                Err(e) => {
                    eprintln!("lens-terminal engine: apply_selection_release failed: {e}");
                    (Vec::new(), GestureDisposition::Ignored)
                }
            }
        }
    }
}
