//! Owned attach + bridge + engine lifecycle (Slice 1d Task 2).

use std::sync::Arc;

use lens_client::AttachHandle;

use crate::bridge::BridgeHandle;
use crate::engine::EngineHandle;

/// Owned by TerminalTab. Foreground may only `take()` this and hand it off.
pub(crate) struct TerminalRuntime {
    pub(crate) bridge: Option<BridgeHandle>,
    pub(crate) attach: Option<AttachHandle>,
    pub(crate) engine: Option<Arc<EngineHandle>>,
}

impl TerminalRuntime {
    /// Take bridge + attach for off-foreground teardown, leaving the engine in place (reconnect).
    pub fn take_transport(&mut self) -> (Option<BridgeHandle>, Option<AttachHandle>) {
        (self.bridge.take(), self.attach.take())
    }

    pub fn install_transport(&mut self, bridge: BridgeHandle, attach: AttachHandle) {
        self.bridge = Some(bridge);
        self.attach = Some(attach);
    }

    pub fn engine_arc(&self) -> Option<Arc<EngineHandle>> {
        self.engine.clone()
    }

    pub(crate) fn attach_ref(&self) -> Option<&AttachHandle> {
        self.attach.as_ref()
    }

    #[cfg_attr(not(feature = "live-tests"), allow(dead_code))]
    pub(crate) fn take_attach(&mut self) -> Option<AttachHandle> {
        self.attach.take()
    }

    pub(crate) fn engine_ref(&self) -> Option<&Arc<EngineHandle>> {
        self.engine.as_ref()
    }

    pub(crate) fn bridge_is_present(&self) -> bool {
        self.bridge.is_some()
    }

    /// Foreground-safe: take self's fields into a background task that joins.
    #[expect(dead_code, reason = "consumed by Slice 1d convergence (Task 3+)")]
    pub fn teardown_off_foreground(self, cx: &mut gpui::AsyncApp) {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .spawn(async move {
                    self.teardown_blocking();
                })
                .await;
        })
        .detach();
    }

    pub(crate) fn teardown_blocking(mut self) {
        // 1. signal+join bridge (stops feeding / da_dsr forward)
        if let Some(b) = self.bridge.take() {
            b.join();
        }
        // 2. attach.close() joins the I/O thread (MUST NOT run on gpui fg)
        if let Some(a) = self.attach.take() {
            a.close();
        }
        // 3. bridge.join already dropped its engine Arc clone
        // 4. normally unique Arc → stop; tolerate a transient extra clone
        if let Some(engine) = self.engine.take()
            && let Ok(owned) = Arc::try_unwrap(engine)
        {
            owned.stop();
        }
        // If not unique, Arc drop alone; worker Drop detaches.
    }
}

#[allow(clippy::collapsible_if, reason = "locked C3 Drop teardown shape")]
impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        // Never join on the dropping thread (may be gpui foreground).
        let bridge = self.bridge.take();
        let attach = self.attach.take();
        let engine = self.engine.take();
        let _ = std::thread::Builder::new()
            .name("lens-terminal-teardown".into())
            .spawn(move || {
                if let Some(b) = bridge {
                    b.join();
                }
                if let Some(a) = attach {
                    a.close();
                }
                if let Some(engine) = engine {
                    if let Ok(owned) = Arc::try_unwrap(engine) {
                        owned.stop();
                    }
                    // If not unique, Arc drop alone; worker Drop detaches.
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use lens_client::{WsInbound, WsOutbound};

    use super::TerminalRuntime;
    use crate::bridge::spawn_bridge;
    use crate::engine::handle::EngineHandle;
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

    #[test]
    fn teardown_blocking_unwraps_unique_arc_and_stops() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let weak = Arc::downgrade(&engine);
        let (inbound_tx, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
        let (outbound_tx, _) = crossbeam_channel::bounded::<WsOutbound>(1);
        let (policy_tx, _) = async_channel::bounded(1);
        let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);
        drop(inbound_tx);
        let rt = TerminalRuntime {
            bridge: Some(bridge),
            attach: None,
            engine: Some(engine),
        };
        rt.teardown_blocking();
        assert!(
            weak.upgrade().is_none(),
            "engine Arc was not uniquely unwrapped + dropped by teardown"
        );
    }

    #[test]
    fn drop_runtime_does_not_join_on_calling_thread() {
        let engine = Arc::new(EngineHandle::spawn(test_cfg()));
        let (_t, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
        let (outbound_tx, _) = crossbeam_channel::bounded::<WsOutbound>(1);
        let (policy_tx, _) = async_channel::bounded(1);
        let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);
        let rt = TerminalRuntime {
            bridge: Some(bridge),
            attach: None,
            engine: Some(engine),
        };
        let start = Instant::now();
        drop(rt); // must return quickly — joins happen on teardown thread
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
