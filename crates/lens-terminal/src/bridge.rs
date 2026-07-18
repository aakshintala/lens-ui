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
    /// Signal stop and **join** the bridge thread. Drops this handle's engine Arc.
    pub fn join(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.stop_tx.try_send(());
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
                Ok(msg) => handle_inbound(&engine, &policy_tx, msg),
                Err(_) => {
                    let _ = policy_tx.try_send(BridgeEvent::AttachDisconnected);
                    LoopExit::Stop
                }
            },
            i if i == egress_idx => match oper.recv(&egress_rx) {
                Ok(frame) => forward_egress(&outbound, frame.bytes, stop, &policy_tx),
                Err(_) => {
                    let _ = policy_tx.try_send(BridgeEvent::EngineStopped);
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
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
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
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
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
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
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
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
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
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
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
}
