//! The injected-time seam (§4.1). `reduce()` stamps `Item.created_at` from a
//! `Clock` so replay is deterministic — it never reads the wall clock directly.

/// A monotonic-ish millisecond clock. The production impl reads `SystemTime`
/// (added by the P3 actor); tests use `ManualClock` for deterministic replay.
pub trait Clock {
    /// Epoch milliseconds.
    fn now_millis(&self) -> i64;
}

/// Test/replay double: returns a fixed instant (settable). Deterministic — the
/// P1 replay gate needs "reduce the same events under the same clock twice ⇒
/// identical state", which a wall clock cannot satisfy.
#[derive(Clone, Debug)]
pub struct ManualClock {
    now: std::cell::Cell<i64>,
}

impl ManualClock {
    pub fn new(now_millis: i64) -> Self {
        Self {
            now: std::cell::Cell::new(now_millis),
        }
    }
    /// Advance the clock (for tests that assert ordering by `created_at`).
    pub fn set(&self, now_millis: i64) {
        self.now.set(now_millis);
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> i64 {
        self.now.get()
    }
}
