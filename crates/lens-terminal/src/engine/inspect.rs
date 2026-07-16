//! Gated engine introspection — zero cost when disabled.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

use serde::Serialize;

const RING_CAP: usize = 32;

/// A single diagnostic event in the engine ring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectEvent {
    pub kind: InspectEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum InspectEventKind {
    FrameBuilt { micros: u64 },
    BytesFed { count: u64 },
    Resize { cols: u16, rows: u16 },
    DaDsr { len: usize },
}

/// Point-in-time engine snapshot for introspection tooling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineInspect {
    pub cols: u16,
    pub rows: u16,
    pub max_scrollback: usize,
    pub visible: bool,
    pub frames_built: u64,
    pub last_build_micros: u64,
    pub bytes_fed: u64,
    pub da_dsr_emitted: u64,
    pub recent: Vec<InspectEvent>,
}

/// Shared inspect state between the worker and [`super::handle::EngineHandle`].
#[derive(Debug)]
pub(crate) struct InspectShared {
    enabled: AtomicBool,
    cols: AtomicU16,
    rows: AtomicU16,
    max_scrollback: AtomicU64,
    visible: AtomicBool,
    frames_built: AtomicU64,
    last_build_micros: AtomicU64,
    bytes_fed: AtomicU64,
    da_dsr_emitted: AtomicU64,
    ring: Mutex<VecDeque<InspectEvent>>,
}

impl InspectShared {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            max_scrollback: AtomicU64::new(max_scrollback as u64),
            visible: AtomicBool::new(true),
            frames_built: AtomicU64::new(0),
            last_build_micros: AtomicU64::new(0),
            bytes_fed: AtomicU64::new(0),
            da_dsr_emitted: AtomicU64::new(0),
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled && let Ok(mut ring) = self.ring.lock() {
            ring.clear();
        }
    }

    pub fn record_bytes_fed(&self, count: u64) {
        self.bytes_fed.fetch_add(count, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::BytesFed { count },
        });
    }

    pub fn record_frame_built(&self, micros: u64) {
        self.frames_built.fetch_add(1, Ordering::Relaxed);
        self.last_build_micros.store(micros, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::FrameBuilt { micros },
        });
    }

    pub fn record_resize(&self, cols: u16, rows: u16) {
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::Resize { cols, rows },
        });
    }

    pub fn record_da_dsr(&self, len: usize) {
        self.da_dsr_emitted.fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::DaDsr { len },
        });
    }

    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
    }

    fn record_event(&self, event: InspectEvent) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut ring) = self.ring.lock() {
            if ring.len() >= RING_CAP {
                ring.pop_front();
            }
            ring.push_back(event);
        }
    }

    pub fn snapshot(&self) -> EngineInspect {
        let recent = if self.enabled.load(Ordering::Relaxed) {
            self.ring
                .lock()
                .map(|r| r.iter().cloned().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        EngineInspect {
            cols: self.cols.load(Ordering::Relaxed),
            rows: self.rows.load(Ordering::Relaxed),
            max_scrollback: self.max_scrollback.load(Ordering::Relaxed) as usize,
            visible: self.visible.load(Ordering::Relaxed),
            frames_built: self.frames_built.load(Ordering::Relaxed),
            last_build_micros: self.last_build_micros.load(Ordering::Relaxed),
            bytes_fed: self.bytes_fed.load(Ordering::Relaxed),
            da_dsr_emitted: self.da_dsr_emitted.load(Ordering::Relaxed),
            recent,
        }
    }
}
