//! Bridge thread: multiplexes attach I/O ↔ engine (Slice 1d).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Select, Sender, TrySendError};
use lens_client::{CloseCause, WsInbound, WsOutbound};

use crate::engine::handle::{EngineHandle, FeedError};

/// Policy events emitted by the bridge thread (off gpui foreground).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BridgeEvent {
    Closed(CloseCause),
    FeedSaturated,
    OutboundSaturated,
    AttachDisconnected,
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
) -> BridgeHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let (stop_tx, stop_rx) = crossbeam_channel::bounded(1);
    let da_dsr_rx = engine.da_dsr_rx().clone();
    let engine_thread = Arc::clone(&engine);

    let join = thread::spawn(move || {
        bridge_loop(
            inbound,
            outbound,
            engine_thread,
            da_dsr_rx,
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
    da_dsr_rx: Receiver<Vec<u8>>,
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
        let da_idx = sel.recv(&da_dsr_rx);
        let stop_idx = sel.recv(&stop_rx);
        let oper = sel.select();

        let exit = match oper.index() {
            i if i == stop_idx => break,
            i if i == inbound_idx => match oper.recv(&inbound) {
                Ok(msg) => handle_inbound(&engine, &policy_tx, msg),
                Err(_) => {
                    let _ = policy_tx.try_send(BridgeEvent::AttachDisconnected);
                    LoopExit::Stop
                }
            },
            i if i == da_idx => match oper.recv(&da_dsr_rx) {
                Ok(bytes) => forward_da_dsr(&outbound, bytes, stop, &policy_tx),
                Err(_) => LoopExit::Stop,
            },
            _ => unreachable!(),
        };

        if matches!(exit, LoopExit::Stop) {
            break;
        }
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
            Err(FeedError::Stopped) => LoopExit::Stop,
        },
        WsInbound::Text(_) => LoopExit::Continue,
        WsInbound::Closed(cause) => {
            let _ = policy_tx.try_send(BridgeEvent::Closed(cause));
            LoopExit::Stop
        }
    }
}

fn forward_da_dsr(
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
        let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);

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
        let before_da = engine.inspect().da_dsr_emitted;
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
        assert!(engine.inspect().da_dsr_emitted > before_da);
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
        let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);

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
        let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);

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
}
