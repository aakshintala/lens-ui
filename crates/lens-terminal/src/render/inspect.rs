//! Render Inspect ring (Slice 1c). Zero-cost when disabled; the full ring +
//! counters land in Task 7. T1 provides the shared handle the canvas records
//! into.

use std::cell::RefCell;
use std::rc::Rc;

use super::paint::RenderStats;

/// Cheaply-cloneable handle the canvas paint closure records into. Interior
/// state lands in Task 7; T1 is a no-op recorder so the paint path compiles.
#[derive(Clone)]
pub struct RenderInspectShared {
    _inner: Rc<RefCell<()>>,
}

impl RenderInspectShared {
    pub fn new() -> Self {
        Self {
            _inner: Rc::new(RefCell::new(())),
        }
    }

    /// Record a completed paint. No-op until Task 7 wires the ring.
    pub fn record_paint(&self, _stats: &RenderStats) {}
}

impl Default for RenderInspectShared {
    fn default() -> Self {
        Self::new()
    }
}
