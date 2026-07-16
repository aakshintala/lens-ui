use std::sync::atomic::{AtomicI64, Ordering};

pub trait UiClock: Send + Sync {
    fn now_millis(&self) -> i64;
}

pub struct WallUiClock;

impl UiClock for WallUiClock {
    fn now_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

pub struct ManualUiClock {
    now: AtomicI64,
}

impl ManualUiClock {
    pub fn new(now_millis: i64) -> Self {
        Self {
            now: AtomicI64::new(now_millis),
        }
    }

    pub fn set(&self, now_millis: i64) {
        self.now.store(now_millis, Ordering::SeqCst);
    }
}

impl UiClock for ManualUiClock {
    fn now_millis(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}
