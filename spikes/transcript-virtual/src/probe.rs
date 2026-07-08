//! Probe instrumentation scaffold (spec §4). Fields only — wiring in Phase 1/2.

use std::time::{Duration, Instant};

use gpui::{ListOffset, Pixels};

/// Logical scroll anchor snapshot for contract-1b before/after comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnchorSnapshot {
    pub top_item_index: usize,
    pub sub_offset: Pixels,
}

impl From<ListOffset> for AnchorSnapshot {
    fn from(offset: ListOffset) -> AnchorSnapshot {
        AnchorSnapshot {
            top_item_index: offset.item_ix,
            sub_offset: offset.offset_in_item,
        }
    }
}

/// Auto-follow ↔ paused transition (UX demo probe).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FollowMode {
    Following,
    Paused,
}

#[derive(Clone, Debug)]
pub struct FollowTransition {
    pub at: Instant,
    pub from: FollowMode,
    pub to: FollowMode,
}

/// Backend-agnostic probe state. Counters and recorders are stubbed; backends
/// will drive these in Phase 1/2.
#[derive(Debug)]
pub struct ProbeState {
    // Windowing (contract 2)
    pub measure_calls: u64,
    pub paint_calls: u64,
    pub frame_timer: FrameTimer,

    // Variable heights (contract 3) — reuse measure_calls per item id in Phase 1

    // Anchoring (contracts 1a / 1b)
    pub anchor_before: Option<AnchorSnapshot>,
    pub anchor_after: Option<AnchorSnapshot>,

    // Jump-to-bottom (contract 4) — recorded on first layout
    pub initial_at_bottom: Option<bool>,

    // UX demo
    pub follow_mode: FollowMode,
    pub follow_log: Vec<FollowTransition>,
    pub new_while_paused: u64,
}

#[derive(Debug)]
pub struct FrameTimer {
    started: Option<Instant>,
    pub last_frame: Duration,
    pub peak_frame: Duration,
}

impl FrameTimer {
    pub fn new() -> Self {
        Self {
            started: None,
            last_frame: Duration::ZERO,
            peak_frame: Duration::ZERO,
        }
    }

    pub fn begin_frame(&mut self) {
        self.started = Some(Instant::now());
    }

    pub fn end_frame(&mut self) {
        if let Some(start) = self.started.take() {
            let elapsed = start.elapsed();
            self.last_frame = elapsed;
            if elapsed > self.peak_frame {
                self.peak_frame = elapsed;
            }
        }
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeState {
    pub fn new() -> Self {
        Self {
            measure_calls: 0,
            paint_calls: 0,
            frame_timer: FrameTimer::new(),
            anchor_before: None,
            anchor_after: None,
            initial_at_bottom: None,
            follow_mode: FollowMode::Following,
            follow_log: Vec::new(),
            new_while_paused: 0,
        }
    }

    pub fn record_anchor_before(&mut self, anchor: AnchorSnapshot) {
        self.anchor_before = Some(anchor);
    }

    pub fn record_anchor_after(&mut self, anchor: AnchorSnapshot) {
        self.anchor_after = Some(anchor);
    }

    pub fn set_follow_mode(&mut self, mode: FollowMode) {
        if mode != self.follow_mode {
            self.follow_log.push(FollowTransition {
                at: Instant::now(),
                from: self.follow_mode,
                to: mode,
            });
            self.follow_mode = mode;
        }
    }

    pub fn increment_new_while_paused(&mut self) {
        if self.follow_mode == FollowMode::Paused {
            self.new_while_paused += 1;
        }
    }
}

impl Default for ProbeState {
    fn default() -> Self {
        Self::new()
    }
}
