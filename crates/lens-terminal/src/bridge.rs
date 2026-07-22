//! Bridge thread: multiplexes attach I/O ↔ engine (Slice 1d).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Select, Sender, TrySendError};
use lens_client::{CloseCause, WsInbound, WsOutbound};

use crate::engine::handle::{EngineHandle, FeedError};
use crate::engine::worker::{EgressFrame, EgressKind};

/// Policy events emitted by the bridge thread (off gpui foreground).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BridgeEvent {
    Closed(CloseCause),
    FeedSaturated,
    OutboundSaturated,
    AttachDisconnected,
    EngineStopped,
    /// The bridge stopped with un-forwarded user `Input` still queued in its egress
    /// channel; that residue was dropped (not replayed to any connection). Coalesced to
    /// one per bridge stop. Reply/focus (`Other`) residue is dropped silently.
    StaleInputDiscarded,
}

pub(crate) struct BridgeHandle {
    stop: Arc<AtomicBool>,
    stop_tx: Sender<()>,
    join: Option<JoinHandle<()>>,
    _engine: Arc<EngineHandle>,
}

pub(crate) fn spawn_bridge(
    inbound: Receiver<WsInbound>,
    outbound: Sender<WsOutbound>,
    engine: Arc<EngineHandle>,
    policy_tx: async_channel::Sender<BridgeEvent>,
    egress_rx: Receiver<EgressFrame>,
) -> BridgeHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (stop_tx, stop_rx) = crossbeam_channel::bounded(1);
    let engine_thread = Arc::clone(&engine);

    let join = thread::spawn(move || {
        bridge_loop(
            inbound,
            outbound,
            engine_thread,
            egress_rx,
            stop_rx,
            policy_tx,
            &stop_thread,
        );
    });

    BridgeHandle {
        stop,
        stop_tx,
        join: Some(join),
        _engine: engine,
    }
}

impl BridgeHandle {
    /// Synchronously ask the bridge loop to stop, WITHOUT joining. Idempotent.
    ///
    /// The foreground calls this at the top of teardown, before any egress swap can
    /// drop this bridge's sender. Once `stop` is set, a bridge that then observes its
    /// egress channel `Disconnected` recognises the teardown and exits quietly instead
    /// of emitting a false [`BridgeEvent::EngineStopped`] (see the egress-`Disconnected`
    /// arm in `bridge_loop`). That closes Critical 1 (false EngineStopped tearing down
    /// the fresh transport) by construction.
    ///
    /// This only *requests* stop — the bridge thread keeps running until it next
    /// observes the flag. It does NOT on its own quiesce feeding, so it does not fully
    /// close the reply-source path (Critical 2). See the C2 invariant documented in
    /// `TerminalTab::teardown_transport_off_foreground`.
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.stop_tx.try_send(());
    }

    /// Signal stop and **join** the bridge thread. Drops this handle's engine Arc.
    pub fn join(mut self) {
        self.signal_stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

enum LoopExit {
    Continue,
    Stop,
}

fn bridge_loop(
    inbound: Receiver<WsInbound>,
    outbound: Sender<WsOutbound>,
    engine: Arc<EngineHandle>,
    egress_rx: Receiver<EgressFrame>,
    stop_rx: Receiver<()>,
    policy_tx: async_channel::Sender<BridgeEvent>,
    stop: &AtomicBool,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let mut sel = Select::new();
        let inbound_idx = sel.recv(&inbound);
        let egress_idx = sel.recv(&egress_rx);
        let stop_idx = sel.recv(&stop_rx);
        let oper = sel.select();

        let exit = match oper.index() {
            // Complete the selected op before breaking: crossbeam's `SelectedOperation`
            // panics on drop if not consumed via recv/send.
            i if i == stop_idx => {
                let _ = oper.recv(&stop_rx);
                break;
            }
            i if i == inbound_idx => match oper.recv(&inbound) {
                // Once stop is requested (teardown in progress), do NOT feed the
                // outgoing connection's VT into the engine: a query reply it produces
                // could encode onto the next connection's egress. This narrows the C2
                // reply-source window — the bridge stops feeding as soon as it observes
                // the flag. Full closure still requires join-before-attach (see
                // `TerminalTab::teardown_transport_off_foreground`).
                Ok(_) if stop.load(Ordering::Relaxed) => LoopExit::Stop,
                Ok(msg) => handle_inbound(&engine, &policy_tx, msg),
                Err(_) => {
                    let _ = policy_tx.try_send(BridgeEvent::AttachDisconnected);
                    LoopExit::Stop
                }
            },
            i if i == egress_idx => match oper.recv(&egress_rx) {
                Ok(frame) => forward_egress(&outbound, frame.bytes, stop, &policy_tx),
                Err(_) => {
                    // The egress sender dropped. If `stop` is already set, this is an
                    // intentional teardown (the foreground called `signal_stop` before
                    // swapping/clearing egress) — exit quietly. Only a sender drop that
                    // is NOT part of a teardown (stop still false) is a genuine engine
                    // death worth surfacing. Suppressing here prevents a false
                    // `EngineStopped` from tearing down the freshly reconnected
                    // transport (C2 hardening).
                    if !stop.load(Ordering::Relaxed) {
                        let _ = policy_tx.try_send(BridgeEvent::EngineStopped);
                    }
                    LoopExit::Stop
                }
            },
            _ => unreachable!(),
        };

        if matches!(exit, LoopExit::Stop) {
            break;
        }
    }

    // Best-effort drain-drop of per-bridge egress residue. A frame the worker pushes
    // after this drain sees `Empty` lingers until the owned receiver is dropped on
    // thread return; still never delivered elsewhere. The notice may undercount.
    let mut dropped_input = false;
    while let Ok(frame) = egress_rx.try_recv() {
        if frame.kind == EgressKind::Input && !frame.bytes.is_empty() {
            dropped_input = true;
        }
    }
    if dropped_input {
        let _ = policy_tx.try_send(BridgeEvent::StaleInputDiscarded);
    }
}

fn handle_inbound(
    engine: &EngineHandle,
    policy_tx: &async_channel::Sender<BridgeEvent>,
    msg: WsInbound,
) -> LoopExit {
    match msg {
        WsInbound::Vt(bytes) => match engine.feed(bytes) {
            Ok(()) => LoopExit::Continue,
            Err(FeedError::Full) => {
                let _ = policy_tx.try_send(BridgeEvent::FeedSaturated);
                LoopExit::Stop
            }
            Err(FeedError::Stopped) => {
                let _ = policy_tx.try_send(BridgeEvent::EngineStopped);
                LoopExit::Stop
            }
        },
        WsInbound::Text(_) => LoopExit::Continue,
        WsInbound::Closed(cause) => {
            let _ = policy_tx.try_send(BridgeEvent::Closed(cause));
            LoopExit::Stop
        }
    }
}

fn forward_egress(
    outbound: &Sender<WsOutbound>,
    bytes: Vec<u8>,
    stop: &AtomicBool,
    policy_tx: &async_channel::Sender<BridgeEvent>,
) -> LoopExit {
    let deadline = Instant::now() + Duration::from_millis(50);
    let mut msg = WsOutbound::Input(bytes);
    loop {
        if stop.load(Ordering::Relaxed) {
            return LoopExit::Stop;
        }
        match outbound.try_send(msg) {
            Ok(()) => return LoopExit::Continue,
            Err(TrySendError::Full(returned)) => {
                msg = returned;
                if Instant::now() >= deadline {
                    let _ = policy_tx.try_send(BridgeEvent::OutboundSaturated);
                    return LoopExit::Stop;
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Disconnected(_)) => {
                let _ = policy_tx.try_send(BridgeEvent::AttachDisconnected);
                return LoopExit::Stop;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use lens_client::{WsInbound, WsOutbound};

    use super::*;
    use crate::engine::frame::Frame;
    use crate::engine::vt::EngineConfig;
    use crate::engine::worker::EGRESS_CHANNEL_CAP;

    fn test_cfg() -> EngineConfig {
        EngineConfig {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    fn wait_new_frame(engine: &EngineHandle, before: u64) -> Arc<Frame> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            engine.build_now().ok();
            if engine.inspect().frames_built > before
                && let Some(f) = engine.latest_frame()
            {
                return f;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("timeout waiting for new frame");
    }

    #[test]
    fn vt_inbound_feeds_engine_after_ack() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
        let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(8);
        let (policy_tx, _policy_rx) = async_channel::bounded(8);
        let before = engine.inspect().frames_built; // always-on counter
        let egress_rx = engine.attach_test_egress();
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        inbound_tx.send(WsInbound::Vt(b"AB".to_vec())).unwrap();
        // Wait until feed is observed — NOT a blind sleep before build_now.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            engine.build_now().ok();
            if engine.inspect().bytes_fed >= 2 {
                break;
            }
            assert!(Instant::now() < deadline, "bridge did not feed engine");
            std::thread::sleep(Duration::from_millis(1));
        }
        let f = wait_new_frame(&engine, before);
        assert!(
            f.grid[0]
                .cells
                .iter()
                .any(|c| c.grapheme == "A" || c.grapheme == "B")
        );

        // DA/DSR forward
        let before_egress = engine.inspect().egress_emitted;
        engine.feed(b"\x1b[c".to_vec()).unwrap();
        engine.build_now().ok();
        let deadline = Instant::now() + Duration::from_secs(2);
        let reply = loop {
            match outbound_rx.try_recv() {
                Ok(WsOutbound::Input(b)) if !b.is_empty() => break b,
                _ => {
                    assert!(Instant::now() < deadline, "DA/DSR not forwarded");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        assert!(!reply.is_empty());
        assert!(engine.inspect().egress_emitted > before_egress);
        bridge.join();
    }

    #[test]
    fn outbound_saturation_emits_event_and_joins() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(1);
        // Cap 1, pre-fill so DA/DSR forward cannot enqueue.
        let (outbound_tx, _outbound_rx) = crossbeam_channel::bounded(1);
        outbound_tx.send(WsOutbound::Input(vec![0])).unwrap();
        let (policy_tx, policy_rx) = async_channel::bounded(8);
        let egress_rx = engine.attach_test_egress();
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        engine.feed(b"\x1b[c".to_vec()).unwrap();
        engine.build_now().ok();
        // Poll async_channel from a tiny block_on / try_recv loop with timeout.
        let deadline = Instant::now() + Duration::from_secs(2);
        let ev = loop {
            if let Ok(ev) = policy_rx.try_recv() {
                break ev;
            }
            assert!(Instant::now() < deadline, "expected OutboundSaturated");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(matches!(ev, BridgeEvent::OutboundSaturated));
        bridge.join(); // confirmed join after saturation exit
    }

    #[test]
    fn outbound_disconnect_emits_attach_disconnected_and_joins() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
        let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(8);
        drop(outbound_rx);
        let (policy_tx, policy_rx) = async_channel::bounded(8);
        let egress_rx = engine.attach_test_egress();
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        engine.feed(b"\x1b[c".to_vec()).unwrap();
        engine.build_now().ok();
        let deadline = Instant::now() + Duration::from_secs(2);
        let ev = loop {
            if let Ok(ev) = policy_rx.try_recv() {
                break ev;
            }
            assert!(Instant::now() < deadline, "expected AttachDisconnected");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(matches!(ev, BridgeEvent::AttachDisconnected));
        bridge.join();
    }

    #[test]
    fn stopped_bridge_drops_input_residue_and_surfaces_once() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (egress_tx, egress_rx) = crossbeam_channel::bounded(EGRESS_CHANNEL_CAP);
        egress_tx
            .send(EgressFrame {
                kind: EgressKind::Input,
                bytes: b"ab".to_vec(),
            })
            .unwrap();
        egress_tx
            .send(EgressFrame {
                kind: EgressKind::Input,
                bytes: b"cd".to_vec(),
            })
            .unwrap();
        egress_tx
            .send(EgressFrame {
                kind: EgressKind::Other,
                bytes: b"\x1b[0n".to_vec(),
            })
            .unwrap();

        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
        let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(1);
        outbound_tx.send(WsOutbound::Input(vec![9])).unwrap();
        let (policy_tx, policy_rx) = async_channel::bounded(8);
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        bridge.join();

        let mut notices = 0;
        while let Ok(ev) = policy_rx.try_recv() {
            if matches!(ev, BridgeEvent::StaleInputDiscarded) {
                notices += 1;
            }
        }
        assert_eq!(notices, 1, "coalesced to one notice");
        let forwarded: Vec<_> = std::iter::from_fn(|| outbound_rx.try_recv().ok()).collect();
        assert!(forwarded.iter().all(|m| !matches!(
            m,
            WsOutbound::Input(b) if b == b"ab" || b == b"cd"
        )));
    }

    #[test]
    fn stopped_bridge_with_only_other_residue_is_silent() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (egress_tx, egress_rx) = crossbeam_channel::bounded(EGRESS_CHANNEL_CAP);
        egress_tx
            .send(EgressFrame {
                kind: EgressKind::Other,
                bytes: b"\x1b[0n".to_vec(),
            })
            .unwrap();

        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
        let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(1);
        outbound_tx.send(WsOutbound::Input(vec![9])).unwrap();
        let (policy_tx, policy_rx) = async_channel::bounded(8);
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        bridge.join();

        assert!(
            policy_rx.try_recv().is_err(),
            "Other-only residue must not emit StaleInputDiscarded"
        );
        let forwarded: Vec<_> = std::iter::from_fn(|| outbound_rx.try_recv().ok()).collect();
        assert!(forwarded.iter().all(|m| !matches!(
            m,
            WsOutbound::Input(b) if b == b"\x1b[0n"
        )));
    }

    // C2 hardening: a bridge whose egress sender is dropped as part of an intentional
    // teardown (stop already set) must exit QUIETLY — no false `EngineStopped`, which
    // would otherwise tear down the freshly reconnected transport. Driven at the
    // `bridge_loop` level so we own the raw `stop` flag and can set it WITHOUT sending
    // `stop_rx`, forcing the wake to come through the egress-`Disconnected` arm alone.
    #[test]
    fn dropped_egress_after_stop_flag_suppresses_engine_stopped() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
        let (outbound_tx, _outbound_rx) = crossbeam_channel::bounded::<WsOutbound>(1);
        let (egress_tx, egress_rx) = crossbeam_channel::bounded::<EgressFrame>(EGRESS_CHANNEL_CAP);
        let (_stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        let (policy_tx, policy_rx) = async_channel::bounded(8);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let engine_thread = Arc::clone(&engine);
        let handle = std::thread::spawn(move || {
            bridge_loop(
                inbound_rx,
                outbound_tx,
                engine_thread,
                egress_rx,
                stop_rx,
                policy_tx,
                &stop_thread,
            );
        });

        // Let the loop park in `select` (nothing else is ready), then set stop and drop
        // the sole egress sender: the wake arrives on the egress arm with stop already
        // true — the suppression path. Removing the `if !stop.load()` guard makes this
        // emit `EngineStopped` and the assertion below fail.
        std::thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        drop(egress_tx);
        handle.join().unwrap();

        let mut saw_engine_stopped = false;
        while let Ok(ev) = policy_rx.try_recv() {
            if matches!(ev, BridgeEvent::EngineStopped) {
                saw_engine_stopped = true;
            }
        }
        assert!(
            !saw_engine_stopped,
            "teardown egress-drop with stop set must not emit EngineStopped"
        );
    }

    // Converse of the above: with stop NOT set, dropping the egress sender is a genuine
    // engine death and MUST surface `EngineStopped`. Proves the suppression is guarding a
    // live path, not silencing a dead one.
    #[test]
    fn dropped_egress_without_stop_emits_engine_stopped() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
        let (outbound_tx, _outbound_rx) = crossbeam_channel::bounded::<WsOutbound>(8);
        let (egress_tx, egress_rx) = crossbeam_channel::bounded::<EgressFrame>(EGRESS_CHANNEL_CAP);
        let (policy_tx, policy_rx) = async_channel::bounded(8);
        let bridge = spawn_bridge(
            inbound_rx,
            outbound_tx,
            Arc::clone(&engine),
            policy_tx,
            egress_rx,
        );

        drop(egress_tx);
        let deadline = Instant::now() + Duration::from_secs(2);
        let saw = loop {
            if let Ok(BridgeEvent::EngineStopped) = policy_rx.try_recv() {
                break true;
            }
            assert!(Instant::now() < deadline, "expected EngineStopped");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(saw);
        bridge.join();
    }

    // C2 narrowing (inbound-arm stop re-check): once stop is requested, the bridge must
    // NOT feed the outgoing connection's VT into the engine — a reply it produced could
    // encode onto the next connection's egress. Driven at `bridge_loop` level so we own
    // the raw `stop` flag: set it WITHOUT signalling `stop_rx`, then deliver inbound, so
    // the wake comes through the inbound arm alone, which must drop the VT unfed.
    // Removing the `stop.load()` guard makes `bytes_fed` advance and this assertion fail.
    #[test]
    fn stopped_bridge_does_not_feed_inbound_vt() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()).expect("spawn engine for test"));
        let (inbound_tx, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
        let (outbound_tx, _outbound_rx) = crossbeam_channel::bounded::<WsOutbound>(1);
        // Held (not dropped) so the egress arm never wakes the select ahead of inbound.
        let (_egress_tx, egress_rx) = crossbeam_channel::bounded::<EgressFrame>(EGRESS_CHANNEL_CAP);
        let (_stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        let (policy_tx, _policy_rx) = async_channel::bounded(8);

        let baseline = engine.inspect().bytes_fed;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let engine_thread = Arc::clone(&engine);
        let handle = std::thread::spawn(move || {
            bridge_loop(
                inbound_rx,
                outbound_tx,
                engine_thread,
                egress_rx,
                stop_rx,
                policy_tx,
                &stop_thread,
            );
        });

        // Park in select, then request stop and deliver a VT that WOULD change the engine
        // if fed. The inbound arm observes stop and drops it unfed.
        std::thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        inbound_tx.send(WsInbound::Vt(b"Z".to_vec())).unwrap();
        handle.join().unwrap();

        // The bridge thread is gone; drain any (should-be-zero) pending feed cmd.
        for _ in 0..3 {
            engine.build_now().ok();
        }
        assert_eq!(
            engine.inspect().bytes_fed,
            baseline,
            "stopped bridge must not feed inbound VT to the engine"
        );
    }
}
