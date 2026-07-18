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
use super::vt::EngineConfig;
use super::worker::{self, EngineCommand, WakerSlot};

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
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
    egress_rx: Receiver<Vec<u8>>,
    inspect: Arc<InspectShared>,
    join: Option<JoinHandle<()>>,
    /// Per-handle build-failure injection counter (see `spawn_worker`). Test-only
    /// — set via `test_inject_build_failures`, shared with this handle's worker.
    #[cfg(test)]
    test_build_failures: Arc<AtomicUsize>,
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
            egress_tx,
            egress_rx,
        } = worker::worker_channels();
        Self::spawn_from_parts(cfg, cmd_tx, cmd_rx, egress_tx, egress_rx)
    }

    #[cfg(test)]
    fn spawn_with_cmd_cap(cfg: EngineConfig, cmd_cap: usize) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(cmd_cap);
        let (egress_tx, egress_rx) = crossbeam_channel::bounded(64);
        Self::spawn_from_parts(cfg, cmd_tx, cmd_rx, egress_tx, egress_rx)
    }

    fn spawn_from_parts(
        cfg: EngineConfig,
        cmd_tx: Sender<EngineCommand>,
        cmd_rx: Receiver<EngineCommand>,
        egress_tx: Sender<Vec<u8>>,
        egress_rx: Receiver<Vec<u8>>,
    ) -> Self {
        let frame_slot = Arc::new(ArcSwapOption::from(None));
        let frame_ready = Arc::new(AtomicBool::new(false));
        let waker: WakerSlot = Arc::new(Mutex::new(None));
        let inspect = Arc::new(InspectShared::new(cfg.cols, cfg.rows, cfg.max_scrollback));
        let test_build_failures = Arc::new(AtomicUsize::new(0));
        let access_epoch = Arc::new(AtomicU64::new(0));
        let input_forwarder = InputForwarder::spawn(cmd_tx.clone());

        let join = worker::spawn_worker(
            cfg,
            cmd_rx,
            egress_tx,
            egress_rx.clone(),
            Arc::clone(&frame_slot),
            Arc::clone(&frame_ready),
            Arc::clone(&waker),
            Arc::clone(&inspect),
            Arc::clone(&test_build_failures),
        );

        Self {
            cmd_tx,
            input_forwarder: Some(input_forwarder),
            access_epoch,
            frame_slot,
            frame_ready,
            waker,
            egress_rx,
            inspect,
            join: Some(join),
            #[cfg(test)]
            test_build_failures,
        }
    }

    /// Enqueue a user-input command via the off-fg forwarder (never blocks the caller).
    ///
    /// Stamps the current [`access_epoch`](Self::bump_access_epoch) onto `Key` / `Focus` at
    /// enqueue time.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "TerminalTab input path wired in Task 5+")
    )]
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

    /// Bump the access epoch and purge pending local input (read-only downgrade).
    #[allow(dead_code, reason = "read-only downgrade wired in Task 6")]
    pub(crate) fn bump_access_epoch(&self) -> u64 {
        let new = self.access_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(forwarder) = &self.input_forwarder {
            forwarder.purge();
        }
        new
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

    pub fn egress_rx(&self) -> &Receiver<Vec<u8>> {
        &self.egress_rx
    }

    pub fn set_inspect_enabled(&self, enabled: bool) {
        self.inspect.set_enabled(enabled);
    }

    pub fn inspect(&self) -> EngineInspect {
        self.inspect.snapshot()
    }

    /// Test hook: the next `count` `build_frame` attempts on **this handle's**
    /// worker fail synthetically. Per-handle (not a process-global) so parallel
    /// tests cannot consume each other's injected failures.
    #[cfg(test)]
    pub(crate) fn test_inject_build_failures(&self, count: usize) {
        self.test_build_failures.store(count, Ordering::SeqCst);
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
    use crate::engine::worker::EngineCommand;

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

    #[test]
    fn try_enqueue_never_blocks_when_engine_channel_full() {
        let h = EngineHandle::spawn_with_cmd_cap_for_test(test_config(), 1);
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
        h.stop();
    }

    #[test]
    fn sever_unblocks_forwarder_after_blocked_barrier() {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded(1);
        let _hold_rx = cmd_rx;
        cmd_tx
            .send(EngineCommand::BuildNow)
            .expect("fill cmd channel");

        let mut forwarder = InputForwarder::spawn(cmd_tx);
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
        forwarder.sever_and_join();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("forwarder must exit after sever");
    }

    #[test]
    fn drop_engine_handle_does_not_block_on_full_cmd_channel() {
        let h = EngineHandle::spawn_with_cmd_cap_for_test(test_config(), 1);
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
        h.feed(b"\x1b[c".to_vec()).expect("feed");
        h.build_now().expect("build_now");
        let deadline = Instant::now() + Duration::from_secs(2);
        let reply = loop {
            if let Ok(r) = h.egress_rx().try_recv() {
                break r;
            }
            if Instant::now() >= deadline {
                panic!("timeout waiting for DA/DSR reply");
            }
            thread::sleep(Duration::from_millis(1));
        };
        assert!(!reply.is_empty());
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
    fn user_input_egress_full_does_not_drop_or_false_ack() {
        let h = EngineHandle::spawn(test_config());
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
        while let Ok(b) = h.egress_rx().try_recv() {
            drained.push(b);
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
}
