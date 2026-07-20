//! Send-safe handle to the non-`Send` engine worker.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use thiserror::Error;

use super::forwarder::InputForwarder;
use super::frame::Frame;
use super::inspect::{EngineInspect, InspectShared};
use super::presentation::EnginePresentationEvent;
use super::vt::EngineConfig;
use super::worker::{self, EngineCommand, TestChunkBarrier, WakerSlot};
use crate::engine::command::ScrollDelta;

/// Backpressure / lifecycle error when sending to a stopped or saturated engine.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeedError {
    #[error("engine command channel full")]
    Full,
    #[error("engine worker has stopped")]
    Stopped,
}

/// Send-safe facade over the pinned engine worker thread.
pub struct EngineHandle {
    cmd_tx: Sender<EngineCommand>,
    input_forwarder: Option<InputForwarder>,
    access_epoch: Arc<AtomicU64>,
    #[cfg(any(test, feature = "test-util"))]
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used from #[cfg(test)] handle tests")
    )]
    worker_stall_gate: Arc<AtomicBool>,
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
    inspect: Arc<InspectShared>,
    presentation_rx: Receiver<EnginePresentationEvent>,
    presentation_tx: crossbeam_channel::Sender<EnginePresentationEvent>,
    latest_title_slot: Arc<ArcSwapOption<String>>,
    join: Option<JoinHandle<()>>,
    /// Per-handle build-failure injection counter (see `spawn_worker`). Test-only
    /// — set via `test_inject_build_failures`, shared with this handle's worker.
    #[cfg(test)]
    test_build_failures: Arc<AtomicUsize>,
    #[cfg(test)]
    chunk_barrier: Arc<TestChunkBarrier>,
}

impl std::fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineHandle").finish_non_exhaustive()
    }
}

impl EngineHandle {
    pub fn spawn(cfg: EngineConfig) -> Self {
        let worker::WorkerChannels {
            cmd_tx,
            cmd_rx,
            presentation_tx,
            presentation_rx,
        } = worker::worker_channels();
        Self::spawn_from_parts(cfg, cmd_tx, cmd_rx, presentation_tx, presentation_rx)
    }

    #[cfg(test)]
    fn spawn_with_cmd_cap(cfg: EngineConfig, cmd_cap: usize) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(cmd_cap);
        let (presentation_tx, presentation_rx) =
            crossbeam_channel::bounded(super::presentation::PRESENTATION_CHANNEL_CAP);
        Self::spawn_from_parts(cfg, cmd_tx, cmd_rx, presentation_tx, presentation_rx)
    }

    fn spawn_from_parts(
        cfg: EngineConfig,
        cmd_tx: Sender<EngineCommand>,
        cmd_rx: Receiver<EngineCommand>,
        presentation_tx: crossbeam_channel::Sender<EnginePresentationEvent>,
        presentation_rx: Receiver<EnginePresentationEvent>,
    ) -> Self {
        let frame_slot = Arc::new(ArcSwapOption::from(None));
        let frame_ready = Arc::new(AtomicBool::new(false));
        let waker: WakerSlot = Arc::new(Mutex::new(None));
        let inspect = Arc::new(InspectShared::new(cfg.cols, cfg.rows, cfg.max_scrollback));
        let test_build_failures = Arc::new(AtomicUsize::new(0));
        let chunk_barrier = Arc::new(TestChunkBarrier::new());
        let access_epoch = Arc::new(AtomicU64::new(0));
        let latest_title_slot = Arc::new(ArcSwapOption::from(None));
        #[cfg(any(test, feature = "test-util"))]
        let worker_stall_gate = Arc::new(AtomicBool::new(false));
        let input_forwarder = InputForwarder::spawn(cmd_tx.clone(), Arc::clone(&access_epoch));

        let join = worker::spawn_worker(
            cfg,
            cmd_rx,
            Arc::clone(&frame_slot),
            Arc::clone(&frame_ready),
            Arc::clone(&waker),
            Arc::clone(&inspect),
            Arc::clone(&test_build_failures),
            #[cfg(any(test, feature = "test-util"))]
            Arc::clone(&worker_stall_gate),
            Arc::clone(&chunk_barrier),
            Arc::clone(&access_epoch),
            presentation_tx.clone(),
            Arc::clone(&latest_title_slot),
        );

        Self {
            cmd_tx,
            input_forwarder: Some(input_forwarder),
            access_epoch,
            #[cfg(any(test, feature = "test-util"))]
            worker_stall_gate,
            frame_slot,
            frame_ready,
            waker,
            inspect,
            presentation_rx,
            presentation_tx,
            latest_title_slot,
            join: Some(join),
            #[cfg(test)]
            test_build_failures,
            #[cfg(test)]
            chunk_barrier,
        }
    }

    /// Enqueue a user-input command via the off-fg forwarder (never blocks the caller).
    ///
    /// Stamps the current [`access_epoch`](Self::bump_access_epoch) onto `Key` / `Focus` at
    /// enqueue time.
    pub(crate) fn enqueue_input(&self, mut cmd: EngineCommand) -> Result<(), FeedError> {
        let epoch = self.access_epoch.load(Ordering::Acquire);
        match &mut cmd {
            EngineCommand::Key(input) => input.access_epoch = epoch,
            EngineCommand::Focus {
                access_epoch: cmd_epoch,
                ..
            } => *cmd_epoch = epoch,
            _ => {}
        }
        let forwarder = self.input_forwarder.as_ref().ok_or(FeedError::Stopped)?;
        forwarder.try_enqueue(cmd).map_err(|()| FeedError::Stopped)
    }

    /// Bump the access epoch (read-only downgrade). Stale `Key`/`Focus` commands still
    /// queued in the forwarder are dropped at forward time via epoch comparison; the
    /// worker's final-egress epoch recheck (Slice 2a Task 6) is the second layer for
    /// commands already mid-retry on `cmd_tx`.
    pub(crate) fn bump_access_epoch(&self) -> u64 {
        self.access_epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    #[cfg(any(test, feature = "test-util"))]
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used from #[cfg(test)] lib tests")
    )]
    pub(crate) fn access_epoch(&self) -> u64 {
        self.access_epoch.load(Ordering::Acquire)
    }

    /// Enqueue viewport-only scroll (allowed in read-only; bypasses the input forwarder).
    pub(crate) fn enqueue_local_scroll(&self, delta: ScrollDelta) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::LocalScroll(delta))
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    pub fn feed(&self, bytes: Vec<u8>) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::Feed(bytes))
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::Resize(cols, rows))
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::SetVisible(visible))
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    pub fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.frame_ready.load(Ordering::Acquire);
        self.frame_slot.load_full()
    }

    pub fn set_waker(&self, waker: Box<dyn Fn() + Send + Sync>) {
        if let Ok(mut guard) = self.waker.lock() {
            *guard = Some(Arc::from(waker));
        }
    }

    /// Point the worker's egress at `tx` via an in-order `SetEgress` command, so a query
    /// fed on the prior connection has already emitted its reply to the prior channel
    /// before this takes effect. Returns `Err` if the command could not be enqueued —
    /// the caller MUST NOT then spawn a bridge on the paired receiver (see below).
    pub(crate) fn attach_egress(
        &self,
        tx: crossbeam_channel::Sender<super::worker::EgressFrame>,
    ) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::SetEgress(Some(tx)))
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn attach_test_egress(&self) -> crossbeam_channel::Receiver<super::worker::EgressFrame> {
        let (tx, rx) = crossbeam_channel::bounded(worker::EGRESS_CHANNEL_CAP);
        self.attach_egress(tx)
            .expect("attach_test_egress: cmd_tx full");
        rx
    }

    pub fn set_inspect_enabled(&self, enabled: bool) {
        self.inspect.set_enabled(enabled);
    }

    pub fn inspect(&self) -> EngineInspect {
        self.inspect.snapshot()
    }

    pub fn presentation_rx(&self) -> &Receiver<EnginePresentationEvent> {
        &self.presentation_rx
    }

    pub fn enqueue_presentation(&self, ev: EnginePresentationEvent) -> Result<(), FeedError> {
        self.presentation_tx.try_send(ev).map_err(|e| match e {
            TrySendError::Full(_) => FeedError::Full,
            TrySendError::Disconnected(_) => FeedError::Stopped,
        })
    }

    /// Take and clear the latest OSC title (authoritative when the channel is full).
    pub fn take_latest_title(&self) -> Option<String> {
        self.latest_title_slot
            .swap(None)
            .map(|title| (*title).clone())
    }

    /// Test hook: the next `count` `build_frame` attempts on **this handle's**
    /// worker fail synthetically. Per-handle (not a process-global) so parallel
    /// tests cannot consume each other's injected failures.
    #[cfg(test)]
    pub(crate) fn test_inject_build_failures(&self, count: usize) {
        self.test_build_failures.store(count, Ordering::SeqCst);
    }

    /// Hold the worker before it drains `cmd_rx` so bounded-channel fullness tests are deterministic.
    #[cfg(any(test, feature = "test-util"))]
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used from #[cfg(test)] handle tests")
    )]
    pub(crate) fn test_stall_worker(&self) {
        self.worker_stall_gate.store(true, Ordering::Release);
    }

    /// Release a worker held by [`Self::test_stall_worker`].
    #[cfg(any(test, feature = "test-util"))]
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used from #[cfg(test)] handle tests")
    )]
    pub(crate) fn test_release_worker(&self) {
        self.worker_stall_gate.store(false, Ordering::Release);
    }

    /// Arm a barrier the worker waits on after chunk 0 of a multi-chunk Feed.
    #[cfg(test)]
    pub(crate) fn test_arm_chunk_barrier(&self) {
        self.chunk_barrier.arm();
    }

    /// Block until the worker finishes feeding chunk 0 (requires [`Self::test_arm_chunk_barrier`]).
    #[cfg(test)]
    pub(crate) fn test_wait_after_first_chunk(&self) {
        self.chunk_barrier.wait_after_first_chunk();
    }

    /// Release the worker from the post-chunk-0 barrier.
    #[cfg(test)]
    pub(crate) fn test_release_chunk_barrier(&self) {
        self.chunk_barrier.release();
    }

    /// Bypass the publish throttle — intended for deterministic tests.
    pub fn build_now(&self) -> Result<(), FeedError> {
        self.cmd_tx
            .try_send(EngineCommand::BuildNow)
            .map_err(|e| match e {
                TrySendError::Full(_) => FeedError::Full,
                TrySendError::Disconnected(_) => FeedError::Stopped,
            })
    }

    #[cfg(test)]
    fn cmd_sender(&self) -> Sender<EngineCommand> {
        self.cmd_tx.clone()
    }

    #[cfg(test)]
    fn spawn_with_cmd_cap_for_test(cfg: EngineConfig, cmd_cap: usize) -> Self {
        Self::spawn_with_cmd_cap(cfg, cmd_cap)
    }

    #[cfg(test)]
    fn frame_slot(&self) -> Arc<ArcSwapOption<Frame>> {
        Arc::clone(&self.frame_slot)
    }

    /// Stop the worker and **block until the thread exits**.
    ///
    /// Slice 1d must call this from a background task — never from the gpui
    /// foreground. This is the only path that joins the pinned worker and
    /// reclaims the non-`Send` `VtEngine` + scrollback.
    pub fn stop(mut self) {
        self.shutdown_worker();
    }

    fn shutdown_worker(&mut self) {
        if let Some(mut forwarder) = self.input_forwarder.take() {
            forwarder.sever_and_join();
        }
        let _ = self.cmd_tx.send(EngineCommand::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    fn signal_stop_nonblocking(&mut self) {
        if let Some(forwarder) = &self.input_forwarder {
            forwarder.signal_stop_nonblocking();
        }
        let _ = self.cmd_tx.try_send(EngineCommand::Stop);
    }
}

impl Drop for EngineHandle {
    /// Signals stop and **detaches** the worker without joining.
    ///
    /// Non-blocking so dropping on the gpui foreground cannot stall the UI.
    /// For a confirmed worker exit (Sleep teardown, scrollback reclaim), call
    /// [`EngineHandle::stop`] from a background task instead.
    fn drop(&mut self) {
        if self.join.is_some() {
            #[cfg(test)]
            self.chunk_barrier.release();
            self.signal_stop_nonblocking();
            let _ = self.input_forwarder.take();
            let _ = self.join.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::command::{KeyAction, KeyInput, KeyMods, LensKey};
    use crate::engine::forwarder::InputForwarder;
    use crate::engine::inspect::InspectEventKind;
    use crate::engine::worker::{EgressFrame, EgressKind, EngineCommand};

    fn test_config() -> EngineConfig {
        EngineConfig {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    fn wait_for_frame(h: &EngineHandle) -> Arc<Frame> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(f) = h.latest_frame() {
                return f;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("timeout waiting for frame");
    }

    fn recv_frame(rx: &crossbeam_channel::Receiver<EgressFrame>) -> EgressFrame {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(f) = rx.try_recv() {
                return f;
            }
            if Instant::now() >= deadline {
                panic!("timeout waiting for egress frame");
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn egress_goes_to_the_currently_attached_channel() {
        let h = EngineHandle::spawn(test_config());
        let rx1 = h.attach_test_egress();
        h.feed(b"\x1b[c".to_vec()).unwrap(); // Primary DA → reply (kind Other)
        h.build_now().ok();
        let f = recv_frame(&rx1);
        assert_eq!(f.kind, EgressKind::Other);
        assert!(!f.bytes.is_empty());

        // Swap to a fresh channel; the old one receives nothing further.
        let rx2 = h.attach_test_egress();
        h.feed(b"\x1b[c".to_vec()).unwrap();
        h.build_now().ok();
        let f2 = recv_frame(&rx2);
        assert_eq!(f2.kind, EgressKind::Other);
        assert!(
            rx1.try_recv().is_err(),
            "old channel must not receive after swap"
        );
        h.stop();
    }

    #[test]
    fn try_enqueue_never_blocks_when_engine_channel_full() {
        let h = EngineHandle::spawn_with_cmd_cap_for_test(test_config(), 1);
        h.test_stall_worker();
        h.cmd_sender()
            .send(EngineCommand::BuildNow)
            .expect("fill cmd channel");

        let key = KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods::default(),
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let start = Instant::now();
        for _ in 0..1000 {
            h.enqueue_input(EngineCommand::Key(key.clone_without_ack()))
                .expect("try_enqueue must not block");
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "1000 try_enqueues took {:?}",
            start.elapsed()
        );
        h.test_release_worker();
        h.stop();
    }

    #[test]
    fn sever_unblocks_forwarder_after_blocked_barrier() {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(1);
        let _hold_rx = cmd_rx;
        cmd_tx
            .send(EngineCommand::BuildNow)
            .expect("fill cmd channel");

        let mut forwarder = InputForwarder::spawn(cmd_tx, Arc::new(AtomicU64::new(0)));
        let key = KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods::default(),
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        forwarder
            .try_enqueue(EngineCommand::Key(key.clone_without_ack()))
            .expect("enqueue key");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !forwarder.blocked_in_retry().load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                panic!("timeout waiting for forwarder blocked-in-retry barrier");
            }
            thread::yield_now();
        }

        let done_rx = forwarder.take_sever_done_rx().expect("sever done rx");
        thread::spawn(move || forwarder.sever_and_join());
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("forwarder must exit after sever");
    }

    #[test]
    fn drop_engine_handle_does_not_block_on_full_cmd_channel() {
        let h = EngineHandle::spawn_with_cmd_cap_for_test(test_config(), 1);
        h.test_stall_worker();
        h.cmd_sender()
            .send(EngineCommand::BuildNow)
            .expect("fill cmd channel");
        let start = Instant::now();
        drop(h);
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "Drop blocked for {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn forwarder_delivers_key_via_enqueue_input() {
        let h = EngineHandle::spawn(test_config());
        let _egress = h.attach_test_egress();
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }))
        .unwrap();
        let ack = ack_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(ack.accepted);
        h.stop();
    }

    #[test]
    fn inspect_records_events_when_enabled_and_ring_empty_when_disabled() {
        let h = EngineHandle::spawn(test_config());
        h.set_inspect_enabled(true);
        h.feed(b"inspect-me".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let _ = wait_for_frame(&h);
        let snap = h.inspect();
        assert!(snap.frames_built >= 1);
        assert!(snap.bytes_fed > 0);
        assert!(
            snap.recent
                .iter()
                .any(|e| matches!(e.kind, InspectEventKind::FrameBuilt { .. }))
        );

        h.set_inspect_enabled(false);
        h.feed(b"more".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let _ = wait_for_frame(&h);
        let snap_off = h.inspect();
        assert!(snap_off.recent.is_empty());
        h.stop();
    }

    #[test]
    fn feed_publishes_a_coalesced_frame_and_wakes() {
        let h = EngineHandle::spawn(test_config());
        let woke = Arc::new(AtomicUsize::new(0));
        {
            let w = Arc::clone(&woke);
            h.set_waker(Box::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }));
        }
        h.feed(b"AB".to_vec()).expect("feed");
        h.feed(b"CD".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let f = wait_for_frame(&h);
        assert!(
            f.grid[0]
                .cells
                .iter()
                .any(|c| c.grapheme == "A" || c.grapheme == "C")
        );
        assert!(woke.load(Ordering::Relaxed) >= 1);
        h.stop();
    }

    #[test]
    fn primary_da_query_emits_reply_on_egress_channel() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        h.feed(b"\x1b[c".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let reply = recv_frame(&rx);
        assert!(!reply.bytes.is_empty());
        h.stop();
    }

    #[test]
    fn hidden_tab_suppresses_publish_until_visible() {
        let h = EngineHandle::spawn(test_config());
        let woke = Arc::new(AtomicUsize::new(0));
        {
            let w = Arc::clone(&woke);
            h.set_waker(Box::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }));
        }

        h.feed(b"XY".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let _ = wait_for_frame(&h);
        let wakes_after_first_publish = woke.load(Ordering::Relaxed);
        assert!(wakes_after_first_publish >= 1);

        h.set_visible(false).expect("set_visible");
        h.build_now().expect("build_now");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            woke.load(Ordering::Relaxed),
            wakes_after_first_publish,
            "no wake while hidden"
        );

        // Hidden → shown with no new feed must still publish + wake once.
        h.set_visible(true).expect("set_visible");
        let deadline = Instant::now() + Duration::from_secs(2);
        while woke.load(Ordering::Relaxed) <= wakes_after_first_publish {
            if Instant::now() >= deadline {
                panic!("show-after-hide must wake even without a new feed");
            }
            thread::sleep(Duration::from_millis(1));
        }
        let f = h.latest_frame().expect("frame");
        assert!(f.grid[0].cells.iter().any(|c| c.grapheme == "X"));
        h.stop();
    }

    #[test]
    fn build_failure_retries_on_next_pump() {
        let h = EngineHandle::spawn(test_config());
        h.set_inspect_enabled(true);
        let woke = Arc::new(AtomicUsize::new(0));
        {
            let w = Arc::clone(&woke);
            h.set_waker(Box::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }));
        }

        h.feed(b"warm".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let _ = wait_for_frame(&h);
        let wakes_before_failure = woke.load(Ordering::Relaxed);
        let built_before_failure = h.inspect().frames_built;

        h.test_inject_build_failures(1);
        h.feed(b"\x1b[2J\x1b[HRE".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        assert_eq!(
            h.inspect().frames_built,
            built_before_failure,
            "injected build failure must not publish"
        );
        assert_eq!(
            woke.load(Ordering::Relaxed),
            wakes_before_failure,
            "injected build failure must not wake"
        );

        h.test_inject_build_failures(0);
        h.build_now().expect("build_now");
        let deadline = Instant::now() + Duration::from_secs(2);
        while h.inspect().frames_built <= built_before_failure {
            if Instant::now() >= deadline {
                panic!("retry must publish on the next pump");
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            woke.load(Ordering::Relaxed) > wakes_before_failure,
            "retry must wake"
        );
        h.stop();
    }

    #[test]
    fn stop_publishes_final_frame_before_join() {
        let h = EngineHandle::spawn(test_config());
        let slot = h.frame_slot();
        h.feed(b"warm".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let _ = wait_for_frame(&h);

        h.feed(b"FINAL".to_vec()).expect("feed");
        h.stop();
        let f = slot.load_full().expect("stop must publish dirty frame");
        assert!(f.grid[0].cells.iter().any(|c| c.grapheme == "F"));
    }

    #[test]
    fn drop_signals_stop_without_blocking_join() {
        let h = EngineHandle::spawn(test_config());
        h.feed(b"Z".to_vec()).expect("feed");
        let start = Instant::now();
        drop(h);
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "Drop must not block on worker join"
        );
    }

    #[test]
    fn stop_joins_worker_and_feed_returns_stopped() {
        let h = EngineHandle::spawn(test_config());
        let tx = h.cmd_sender();
        h.feed(b"Z".to_vec()).expect("feed");
        h.stop();
        assert!(matches!(
            tx.try_send(EngineCommand::Feed(vec![])),
            Err(TrySendError::Disconnected(_))
        ));
    }

    #[test]
    fn key_encodes_against_live_modes_via_ordered_feed_then_ack() {
        let h = EngineHandle::spawn(test_config());
        let _egress = h.attach_test_egress();
        h.feed(b"\x1b[?1h".to_vec()).expect("feed");

        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                action: KeyAction::Press,
                key: LensKey::ArrowUp,
                mods: KeyMods::default(),
                utf8: None,
                composing: false,
                access_epoch: 0,
                ack: Some(ack_tx),
            }))
            .expect("send key");

        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("ack timeout");
        assert_eq!(ack.encoded, b"\x1bOA");
        assert!(ack.accepted);
        h.stop();
    }

    #[test]
    fn feed_is_atomic_key_after_feed_sees_post_feed_modes() {
        let h = EngineHandle::spawn(test_config());
        h.feed(b"\x1b[?1h".to_vec()).unwrap();
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }))
        .unwrap();
        let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ack.encoded, b"\x1bOA");
        h.stop();
    }

    #[test]
    fn key_before_feed_sees_pre_feed_modes() {
        // Input-vs-feed cross-path ordering is inherently concurrent (different
        // threads) — this test pins the worker-level cmd_tx ordering contract,
        // not the forwarder path.
        let h = EngineHandle::spawn(test_config());
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                action: KeyAction::Press,
                key: LensKey::ArrowUp,
                mods: KeyMods::default(),
                utf8: None,
                composing: false,
                access_epoch: 0,
                ack: Some(ack_tx),
            }))
            .expect("send key before feed");
        let mut mode_and_pad = Vec::with_capacity(64 * 1024);
        mode_and_pad.extend_from_slice(b"\x1b[?1h");
        mode_and_pad.resize(64 * 1024, b' ');
        h.feed(mode_and_pad).unwrap();
        let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ack.encoded, b"\x1b[A");
        h.stop();
    }

    #[test]
    fn stop_preempts_feed_between_chunks_deterministically() {
        use crate::engine::worker::MAX_FEED_CHUNK;

        let h = EngineHandle::spawn(test_config());
        h.set_inspect_enabled(true);
        h.test_arm_chunk_barrier();
        h.feed(vec![b'X'; 64 * 1024]).unwrap();
        h.test_wait_after_first_chunk();
        h.cmd_sender().send(EngineCommand::Stop).unwrap();
        h.test_release_chunk_barrier();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match h.cmd_sender().try_send(EngineCommand::BuildNow) {
                Ok(()) => {}
                Err(TrySendError::Disconnected(_)) => break,
                Err(TrySendError::Full(_)) => {
                    assert!(Instant::now() < deadline, "worker did not exit after Stop");
                    thread::yield_now();
                }
            }
        }
        let fed = h.inspect().bytes_fed;
        h.stop();
        assert!(fed >= MAX_FEED_CHUNK as u64);
        assert!(fed < 64 * 1024);
    }

    #[test]
    fn mid_feed_key_defers_until_after_feed() {
        use crate::engine::worker::MAX_FEED_CHUNK;

        let h = EngineHandle::spawn(test_config());
        let _egress = h.attach_test_egress();
        let mut feed = vec![b' '; MAX_FEED_CHUNK];
        feed.extend_from_slice(b"\x1b[?1h");

        h.test_arm_chunk_barrier();
        h.feed(feed).unwrap();
        h.test_wait_after_first_chunk();

        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                action: KeyAction::Press,
                key: LensKey::ArrowUp,
                mods: KeyMods::default(),
                utf8: None,
                composing: false,
                access_epoch: 0,
                ack: Some(ack_tx),
            }))
            .expect("send key during feed pause");

        h.test_release_chunk_barrier();

        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("ack timeout");
        assert_eq!(ack.encoded, b"\x1bOA");
        assert!(ack.accepted);
        h.stop();
    }

    #[test]
    fn decset_straddling_feed_chunk_boundary_still_applies() {
        use crate::engine::worker::MAX_FEED_CHUNK;

        assert_eq!(MAX_FEED_CHUNK, 4096);
        let h = EngineHandle::spawn(test_config());
        let mut buf = vec![b' '; 4094];
        buf.extend_from_slice(b"\x1b[?1h");
        h.feed(buf).unwrap();
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }))
        .unwrap();
        let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ack.encoded, b"\x1bOA");
        h.stop();
    }

    #[test]
    fn focus_report_suppressed_when_report_false_with_ack_barrier() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        while rx.try_recv().is_ok() {}
        h.feed(b"\x1b[?1004h".to_vec()).expect("feed");
        h.cmd_sender()
            .send(EngineCommand::Focus {
                focused: true,
                report: false,
                access_epoch: 0,
            })
            .expect("focus");
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                action: KeyAction::Press,
                key: LensKey::Z,
                mods: KeyMods::default(),
                utf8: Some("z".into()),
                composing: false,
                access_epoch: 0,
                ack: Some(ack_tx),
            }))
            .expect("barrier key");
        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("barrier ack");
        assert!(ack.accepted);
        let mut egress = Vec::new();
        while let Ok(f) = rx.try_recv() {
            egress.extend_from_slice(&f.bytes);
        }
        assert_eq!(egress, b"z");
        h.stop();
    }

    #[test]
    fn focus_report_emits_csi_i_when_mode_on_and_report_true() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        while rx.try_recv().is_ok() {}
        h.feed(b"\x1b[?1004h".to_vec()).expect("feed");
        h.cmd_sender()
            .send(EngineCommand::Focus {
                focused: true,
                report: true,
                access_epoch: 0,
            })
            .expect("focus");
        let frame = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("focus egress");
        assert_eq!(frame.bytes, b"\x1b[I");
        h.stop();
    }

    #[test]
    fn focus_report_suppressed_when_mode_1004_off() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        while rx.try_recv().is_ok() {}
        h.cmd_sender()
            .send(EngineCommand::Focus {
                focused: true,
                report: true,
                access_epoch: 0,
            })
            .expect("focus");
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                action: KeyAction::Press,
                key: LensKey::Z,
                mods: KeyMods::default(),
                utf8: Some("z".into()),
                composing: false,
                access_epoch: 0,
                ack: Some(ack_tx),
            }))
            .expect("barrier key");
        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("barrier ack");
        assert!(ack.accepted);
        let mut egress = Vec::new();
        while let Ok(f) = rx.try_recv() {
            egress.extend_from_slice(&f.bytes);
        }
        assert_eq!(egress, b"z");
        h.stop();
    }

    /// C2 Critical proof: stale-epoch keys held on `cmd_tx` during teardown must not
    /// encode onto any egress channel. Without step 5 (`bump_access_epoch`), the
    /// released Key would encode onto `rx1` (processed before the `SetEgress` swap in
    /// FIFO order), so `rx1` would be non-empty — this test fails without the bump.
    #[test]
    fn upstream_key_revoked_by_epoch_bump_reaches_no_channel() {
        let h = EngineHandle::spawn(test_config());
        let rx1 = h.attach_test_egress();
        h.test_stall_worker();
        h.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods::default(),
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        }))
        .expect("enqueue key");
        h.bump_access_epoch();
        let rx2 = h.attach_test_egress();
        h.test_release_worker();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if rx1.try_recv().is_ok() || rx2.try_recv().is_ok() {
                panic!("stale-epoch key must not reach any egress channel");
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(rx1.try_recv().is_err(), "rx1 must stay empty");
        assert!(rx2.try_recv().is_err(), "rx2 must stay empty");
        h.stop();
    }

    #[test]
    fn downgrade_revokes_queued_key_before_egress() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        h.test_stall_worker();
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.enqueue_input(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::Z,
            mods: KeyMods::default(),
            utf8: Some("z".into()),
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }))
        .expect("enqueue key");
        h.bump_access_epoch();
        h.test_release_worker();
        let ack = ack_rx.recv_timeout(Duration::from_secs(2)).expect("ack");
        assert!(!ack.accepted);
        assert!(rx.try_recv().is_err());
        h.stop();
    }

    fn grid_text(f: &Frame) -> String {
        f.grid
            .iter()
            .flat_map(|row| row.cells.iter().map(|c| c.grapheme.as_str()))
            .collect()
    }

    #[test]
    fn local_scroll_allowed_in_read_only_without_egress() {
        use crate::engine::command::ScrollDelta;

        let h = EngineHandle::spawn(test_config());
        for i in 0..30 {
            h.feed(format!("L{i:02}\r\n").into_bytes()).expect("feed");
        }
        h.build_now().expect("build_now");
        let before = wait_for_frame(&h);
        let before_text = grid_text(&before);
        assert!(
            before_text.contains("L29"),
            "viewport should show tail lines"
        );

        h.enqueue_local_scroll(ScrollDelta::Top).expect("scroll");
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        h.set_waker(Box::new(move || {
            let _ = done_tx.try_send(());
        }));
        h.build_now().expect("build_now");
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("scroll frame wake");
        let after = h.latest_frame().expect("frame after scroll");
        assert!(grid_text(&after).contains("L00"));
        assert!(!grid_text(&after).contains("L29"));
        let rx = h.attach_test_egress();
        assert!(rx.try_recv().is_err());
        h.stop();
    }

    #[test]
    fn user_input_egress_full_does_not_drop_or_false_ack() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();
        let first_key = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Z,
            mods: KeyMods::default(),
            utf8: Some("z".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        h.cmd_sender()
            .send(EngineCommand::Key(first_key.clone_without_ack()))
            .expect("send first key");

        let fill_key = KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods::default(),
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        for _ in 0..63 {
            h.cmd_sender()
                .send(EngineCommand::Key(fill_key.clone_without_ack()))
                .expect("send fill key");
        }

        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        h.cmd_sender()
            .send(EngineCommand::Key(KeyInput {
                ack: Some(ack_tx),
                ..fill_key.clone_without_ack()
            }))
            .expect("send key with ack");

        let ack = ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("ack timeout");
        assert!(!ack.accepted, "must not ACK accepted when egress is full");
        assert_eq!(ack.encoded, b"a");

        let mut drained = Vec::new();
        while let Ok(f) = rx.try_recv() {
            drained.push(f.bytes);
        }
        assert_eq!(
            drained.len(),
            64,
            "prior egress must not be drop-oldest evicted"
        );
        assert_eq!(
            drained[0], b"z",
            "oldest egress must remain at front (no drop-oldest)"
        );
        assert!(drained[1..].iter().all(|b| b == b"a"));
        assert_eq!(h.inspect().user_egress_rejected, 1);
        h.stop();
    }

    #[test]
    fn engine_handle_exposes_presentation_rx_after_title_feed() {
        use crate::engine::presentation::EnginePresentationEvent;

        let h = EngineHandle::spawn(EngineConfig {
            cols: 40,
            rows: 8,
            max_scrollback: 32,
            cell_w_px: 8,
            cell_h_px: 16,
        });
        h.feed(b"\x1b]2;ViaHandle\x1b\\".to_vec()).unwrap();
        let title = h
            .take_latest_title()
            .or_else(|| {
                h.presentation_rx()
                    .recv_timeout(Duration::from_secs(2))
                    .ok()
                    .and_then(|ev| match ev {
                        EnginePresentationEvent::TitleChanged(t) => Some(t),
                        _ => None,
                    })
            })
            .expect("presentation title");
        assert_eq!(title, "ViaHandle");
        h.stop();
    }

    #[test]
    fn latest_title_wins_when_channel_full() {
        use crate::engine::presentation::{EnginePresentationEvent, resolve_drain_title};
        use crate::engine::vt::VtEngine;

        let (tx, rx) = crossbeam_channel::bounded(1);
        let mut engine = VtEngine::new(&test_config(), |_| {}, tx.clone()).unwrap();
        tx.try_send(EnginePresentationEvent::TitleChanged("Stale".into()))
            .unwrap();
        assert!(
            tx.try_send(EnginePresentationEvent::TitleChanged("Blocked".into()))
                .is_err(),
            "channel must be full"
        );
        engine.feed(b"\x1b]2;FinalTitle\x1b\\");
        let slot = engine.take_latest_title();
        assert_eq!(
            slot.as_deref(),
            Some("FinalTitle"),
            "latest-title slot must hold the final OSC title when channel is saturated"
        );
        let first = rx.try_recv().unwrap();
        let stale = match first {
            EnginePresentationEvent::TitleChanged(t) => t,
            _ => panic!("expected TitleChanged"),
        };
        assert_eq!(stale, "Stale");
        assert!(rx.try_recv().is_err());
        assert_eq!(
            resolve_drain_title(slot, &[stale]).as_deref(),
            Some("FinalTitle"),
            "drain apply must not let a stale channel title overwrite the slot"
        );
    }

    #[test]
    fn reply_egress_full_does_not_evict_user_input() {
        let h = EngineHandle::spawn(test_config());
        let rx = h.attach_test_egress();

        for i in 0..64u8 {
            let ch = char::from(b'A' + i);
            h.cmd_sender()
                .send(EngineCommand::Key(KeyInput {
                    action: KeyAction::Press,
                    key: LensKey::Unidentified,
                    mods: KeyMods::default(),
                    utf8: Some(ch.to_string()),
                    composing: false,
                    access_epoch: 0,
                    ack: None,
                }))
                .expect("send key");
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        while rx.len() < 64 {
            if Instant::now() >= deadline {
                panic!("timeout waiting for egress to fill with user input");
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rx.len(), 64);

        h.feed(b"\x1b[c".to_vec()).expect("feed DA query");
        h.build_now().expect("build_now");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if rx.len() > 64 {
                panic!("reply egress evicted user input from shared queue");
            }
            thread::sleep(Duration::from_millis(1));
        }

        let mut drained = Vec::new();
        while let Ok(f) = rx.try_recv() {
            drained.push(f.bytes);
        }
        assert_eq!(
            drained.len(),
            64,
            "reply must be dropped, not evict never-drop user egress"
        );
        for (i, bytes) in drained.iter().enumerate() {
            let expected = char::from(b'A' + i as u8).to_string();
            assert_eq!(bytes.as_slice(), expected.as_bytes(), "egress order at {i}");
        }
        h.stop();
    }
}
