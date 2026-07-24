//! Gated engine introspection — zero cost when disabled.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

use serde::Serialize;

const RING_CAP: usize = 32;

/// Provisional retained-bytes-per-cell multiplier for the fleet-accounting
/// **estimate** (`total_rows × cols × PER_CELL_BYTES`). This is a documented
/// placeholder: it affects only the estimate's *scale*, never its *ordinal*
/// use (LRV trimming compares estimates against each other). Slice 3 Job B
/// (`xtask terminal-rss-sweep`) reports the empirically-calibrated value from
/// RSS-vs-total_rows; folding it back here is a one-line edit. Byte-*accurate*
/// accounting (a Ghostty byte selector) is a fail-closed conditional escalated
/// ONLY if Job B shows the estimate is ordinally unreliable.
pub const PER_CELL_BYTES: usize = 4;

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
    TitleApplied,
    TitleSlotOverwrite,
    HyperlinkOpen,
    PresentationChannelFullDrop,
}

/// Point-in-time engine snapshot for introspection tooling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineInspect {
    pub cols: u16,
    pub rows: u16,
    pub max_scrollback: usize,
    pub total_rows: usize,
    /// **Ordinal ranking score, NOT an absolute byte count.** Computed as
    /// `total_rows × cols × PER_CELL_BYTES` with a placeholder per-cell figure,
    /// so it tracks memory *ordering* across tabs (its only sanctioned use — LRV
    /// trimming compares tabs against each other) but is a large undercount in
    /// absolute terms: Job B measured real RSS at ~2.5–3.7× this at scale, and
    /// up to ~19× for small tabs (fixed overhead dominates). It is also
    /// content-blind — equal `total_rows` yields an equal score even when real
    /// RSS diverges by ~50% (compressible vs incompressible). NEVER surface or
    /// budget against this as bytes; for that you need byte-accurate FFI (the
    /// parked escalation). See `PER_CELL_BYTES`.
    pub retained_bytes_estimate: usize,
    pub visible: bool,
    pub frames_built: u64,
    pub last_build_micros: u64,
    pub bytes_fed: u64,
    pub egress_emitted: u64,
    pub user_egress_accepted: u64,
    pub user_egress_rejected: u64,
    pub keys_encoded: u64,
    pub feed_chunks: u64,
    pub stop_preempts: u64,
    pub titles_applied: u64,
    pub title_slot_overwrites: u64,
    pub hyperlink_opens: u64,
    pub presentation_channel_full_drops: u64,
    pub osc52_forwarded: u64,
    pub osc52_over_cap_drops: u64,
    pub clipboard_writes_allowed: u64,
    pub clipboard_writes_denied: u64,
    pub pastes_sent: u64,
    pub paste_over_cap_rejects: u64,
    pub paste_warn_prompts: u64,
    pub mouse_encoded: u64,
    pub mouse_reports_coalesced: u64,
    pub mouse_suppressed: u64,
    pub wheel_reported: u64,
    pub copy_started: u64,
    pub copy_completed: u64,
    pub copy_empty: u64,
    pub local_clicks_dropped: u64,
    pub recent: Vec<InspectEvent>,
}

/// Shared inspect state between the worker and [`super::handle::EngineHandle`].
#[derive(Debug)]
pub(crate) struct InspectShared {
    enabled: AtomicBool,
    cols: AtomicU16,
    rows: AtomicU16,
    max_scrollback: AtomicU64,
    total_rows: AtomicU64,
    visible: AtomicBool,
    frames_built: AtomicU64,
    last_build_micros: AtomicU64,
    bytes_fed: AtomicU64,
    egress_emitted: AtomicU64,
    user_egress_accepted: AtomicU64,
    user_egress_rejected: AtomicU64,
    keys_encoded: AtomicU64,
    feed_chunks: AtomicU64,
    stop_preempts: AtomicU64,
    titles_applied: AtomicU64,
    title_slot_overwrites: AtomicU64,
    hyperlink_opens: AtomicU64,
    presentation_channel_full_drops: AtomicU64,
    osc52_forwarded: AtomicU64,
    osc52_over_cap_drops: AtomicU64,
    clipboard_writes_allowed: AtomicU64,
    clipboard_writes_denied: AtomicU64,
    pastes_sent: AtomicU64,
    paste_over_cap_rejects: AtomicU64,
    paste_warn_prompts: AtomicU64,
    mouse_encoded: AtomicU64,
    mouse_reports_coalesced: AtomicU64,
    mouse_suppressed: AtomicU64,
    wheel_reported: AtomicU64,
    copy_started: AtomicU64,
    copy_completed: AtomicU64,
    copy_empty: AtomicU64,
    local_clicks_dropped: AtomicU64,
    ring: Mutex<VecDeque<InspectEvent>>,
}

impl InspectShared {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            max_scrollback: AtomicU64::new(max_scrollback as u64),
            total_rows: AtomicU64::new(0),
            visible: AtomicBool::new(true),
            frames_built: AtomicU64::new(0),
            last_build_micros: AtomicU64::new(0),
            bytes_fed: AtomicU64::new(0),
            egress_emitted: AtomicU64::new(0),
            user_egress_accepted: AtomicU64::new(0),
            user_egress_rejected: AtomicU64::new(0),
            keys_encoded: AtomicU64::new(0),
            feed_chunks: AtomicU64::new(0),
            stop_preempts: AtomicU64::new(0),
            titles_applied: AtomicU64::new(0),
            title_slot_overwrites: AtomicU64::new(0),
            hyperlink_opens: AtomicU64::new(0),
            presentation_channel_full_drops: AtomicU64::new(0),
            osc52_forwarded: AtomicU64::new(0),
            osc52_over_cap_drops: AtomicU64::new(0),
            clipboard_writes_allowed: AtomicU64::new(0),
            clipboard_writes_denied: AtomicU64::new(0),
            pastes_sent: AtomicU64::new(0),
            paste_over_cap_rejects: AtomicU64::new(0),
            paste_warn_prompts: AtomicU64::new(0),
            mouse_encoded: AtomicU64::new(0),
            mouse_reports_coalesced: AtomicU64::new(0),
            mouse_suppressed: AtomicU64::new(0),
            wheel_reported: AtomicU64::new(0),
            copy_started: AtomicU64::new(0),
            copy_completed: AtomicU64::new(0),
            copy_empty: AtomicU64::new(0),
            local_clicks_dropped: AtomicU64::new(0),
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled && let Ok(mut ring) = self.ring.lock() {
            ring.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn record_bytes_fed(&self, count: u64) {
        self.bytes_fed.fetch_add(count, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::BytesFed { count },
        });
    }

    pub fn record_frame_built(&self, micros: u64) {
        // Store the duration BEFORE publishing the incremented count, and use
        // Release/Acquire (see `snapshot`) so a reader that observes the new
        // `frames_built` is guaranteed to also see THIS build's `micros` — not
        // the previous build's. Consumers that pair the two (the Job-A
        // per-distinct-build sampler) would otherwise associate a fresh count
        // with a stale duration. codex I7-follow-up.
        self.last_build_micros.store(micros, Ordering::Release);
        self.frames_built.fetch_add(1, Ordering::Release);
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

    /// Sample the emulator's retained row count (scrollback + viewport). The
    /// store is unconditional (cheap, like `cols`/`rows`) so the estimate is
    /// available even with the ring disabled; the caller supplies the value it
    /// already read from the terminal after a build.
    pub fn record_retained_rows(&self, total_rows: usize) {
        self.total_rows.store(total_rows as u64, Ordering::Relaxed);
    }

    /// Lightweight ordinal retained-bytes estimate (`total_rows × cols ×
    /// `PER_CELL_BYTES`) from two atomics — no ring lock, no full snapshot.
    pub fn retained_bytes_estimate(&self) -> usize {
        let cols = self.cols.load(Ordering::Relaxed) as usize;
        let total_rows = self.total_rows.load(Ordering::Relaxed) as usize;
        total_rows
            .saturating_mul(cols)
            .saturating_mul(PER_CELL_BYTES)
    }

    pub fn record_egress(&self, len: usize) {
        self.egress_emitted.fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::DaDsr { len },
        });
    }

    pub fn record_user_egress_accepted(&self) {
        self.user_egress_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_user_egress_rejected(&self) {
        self.user_egress_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_keys_encoded(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.keys_encoded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_feed_chunk(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.feed_chunks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_stop_preempt(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.stop_preempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_title_applied(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.titles_applied.fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::TitleApplied,
        });
    }

    pub fn record_title_slot_overwrite(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.title_slot_overwrites.fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::TitleSlotOverwrite,
        });
    }

    pub fn record_hyperlink_open(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.hyperlink_opens.fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::HyperlinkOpen,
        });
    }

    pub fn record_presentation_channel_full_drop(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.presentation_channel_full_drops
            .fetch_add(1, Ordering::Relaxed);
        self.record_event(InspectEvent {
            kind: InspectEventKind::PresentationChannelFullDrop,
        });
    }

    pub fn record_osc52_forwarded(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.osc52_forwarded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_osc52_over_cap_drop(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.osc52_over_cap_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_clipboard_write_allowed(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.clipboard_writes_allowed
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_clipboard_write_denied(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.clipboard_writes_denied.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_paste_sent(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.pastes_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_paste_over_cap_reject(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.paste_over_cap_rejects.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_paste_warn_prompt(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.paste_warn_prompts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_mouse_encoded(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.mouse_encoded.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_mouse_report_coalesced(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.mouse_reports_coalesced.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_mouse_suppressed(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.mouse_suppressed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_wheel_reported(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.wheel_reported.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_copy_started(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.copy_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_copy_completed(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.copy_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_copy_empty(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.copy_empty.fetch_add(1, Ordering::Relaxed);
    }

    /// A `LocalClick` presentation event could not be delivered (channel full) — the
    /// hyperlink open is lost. Recorded so the drop is observable (codex whole-slice F9).
    pub fn record_local_click_dropped(&self) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        self.local_clicks_dropped.fetch_add(1, Ordering::Relaxed);
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

        let cols = self.cols.load(Ordering::Relaxed);
        let total_rows = self.total_rows.load(Ordering::Relaxed) as usize;
        let retained_bytes_estimate = self.retained_bytes_estimate();

        EngineInspect {
            cols,
            rows: self.rows.load(Ordering::Relaxed),
            max_scrollback: self.max_scrollback.load(Ordering::Relaxed) as usize,
            total_rows,
            retained_bytes_estimate,
            visible: self.visible.load(Ordering::Relaxed),
            // Load the count with Acquire FIRST (pairs with the Release in
            // `record_frame_built`), then the duration — so an observed count
            // implies its build's duration is visible. codex I7-follow-up.
            frames_built: self.frames_built.load(Ordering::Acquire),
            last_build_micros: self.last_build_micros.load(Ordering::Acquire),
            bytes_fed: self.bytes_fed.load(Ordering::Relaxed),
            egress_emitted: self.egress_emitted.load(Ordering::Relaxed),
            user_egress_accepted: self.user_egress_accepted.load(Ordering::Relaxed),
            user_egress_rejected: self.user_egress_rejected.load(Ordering::Relaxed),
            keys_encoded: self.keys_encoded.load(Ordering::Relaxed),
            feed_chunks: self.feed_chunks.load(Ordering::Relaxed),
            stop_preempts: self.stop_preempts.load(Ordering::Relaxed),
            titles_applied: self.titles_applied.load(Ordering::Relaxed),
            title_slot_overwrites: self.title_slot_overwrites.load(Ordering::Relaxed),
            hyperlink_opens: self.hyperlink_opens.load(Ordering::Relaxed),
            presentation_channel_full_drops: self
                .presentation_channel_full_drops
                .load(Ordering::Relaxed),
            osc52_forwarded: self.osc52_forwarded.load(Ordering::Relaxed),
            osc52_over_cap_drops: self.osc52_over_cap_drops.load(Ordering::Relaxed),
            clipboard_writes_allowed: self.clipboard_writes_allowed.load(Ordering::Relaxed),
            clipboard_writes_denied: self.clipboard_writes_denied.load(Ordering::Relaxed),
            pastes_sent: self.pastes_sent.load(Ordering::Relaxed),
            paste_over_cap_rejects: self.paste_over_cap_rejects.load(Ordering::Relaxed),
            paste_warn_prompts: self.paste_warn_prompts.load(Ordering::Relaxed),
            mouse_encoded: self.mouse_encoded.load(Ordering::Relaxed),
            mouse_reports_coalesced: self.mouse_reports_coalesced.load(Ordering::Relaxed),
            mouse_suppressed: self.mouse_suppressed.load(Ordering::Relaxed),
            wheel_reported: self.wheel_reported.load(Ordering::Relaxed),
            copy_started: self.copy_started.load(Ordering::Relaxed),
            copy_completed: self.copy_completed.load(Ordering::Relaxed),
            copy_empty: self.copy_empty.load(Ordering::Relaxed),
            local_clicks_dropped: self.local_clicks_dropped.load(Ordering::Relaxed),
            recent,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::handle::EngineHandle;
    use crate::engine::vt::EngineConfig;

    fn test_config() -> EngineConfig {
        EngineConfig {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    #[test]
    fn retained_rows_and_estimate_default_zero() {
        let shared = super::InspectShared::new(80, 24, 1000);
        let snap = shared.snapshot();
        assert_eq!(snap.total_rows, 0);
        assert_eq!(snap.retained_bytes_estimate, 0);
    }

    #[test]
    fn retained_estimate_is_total_rows_times_cols_times_per_cell() {
        let shared = super::InspectShared::new(200, 50, 100_000);
        shared.record_retained_rows(10_000);
        let snap = shared.snapshot();
        assert_eq!(snap.total_rows, 10_000);
        assert_eq!(
            snap.retained_bytes_estimate,
            10_000usize * 200 * super::PER_CELL_BYTES
        );
    }

    #[test]
    fn retained_bytes_estimate_fast_path_matches_snapshot() {
        let shared = super::InspectShared::new(200, 50, 100_000);
        shared.record_retained_rows(10_000);
        assert_eq!(
            shared.retained_bytes_estimate(),
            shared.snapshot().retained_bytes_estimate
        );
    }

    #[test]
    fn inspect_mouse_copy_counters_default_zero() {
        let h = EngineHandle::spawn(test_config()).expect("spawn engine for test");
        let snap = h.inspect();
        assert_eq!(snap.mouse_encoded, 0);
        assert_eq!(snap.mouse_reports_coalesced, 0);
        assert_eq!(snap.mouse_suppressed, 0);
        assert_eq!(snap.wheel_reported, 0);
        assert_eq!(snap.copy_started, 0);
        assert_eq!(snap.copy_completed, 0);
        assert_eq!(snap.copy_empty, 0);
        assert_eq!(snap.local_clicks_dropped, 0);
        h.stop();
    }

    #[test]
    fn handle_inspect_reports_retained_estimate_after_streaming() {
        use std::time::{Duration, Instant};
        let cfg = EngineConfig {
            cols: 40,
            rows: 4,
            max_scrollback: 500,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let h = EngineHandle::spawn(cfg).expect("spawn engine for test");
        for i in 0..200 {
            let _ = h.feed(format!("streaming line {i}\r\n").into_bytes());
        }
        let _ = h.build_now();
        // Poll until the worker has built at least one frame and sampled rows.
        let deadline = Instant::now() + Duration::from_secs(2);
        let snap = loop {
            let s = h.inspect();
            if s.frames_built > 0 && s.total_rows > cfg.rows as usize {
                break s;
            }
            if Instant::now() > deadline {
                panic!("engine never reported retained rows: {s:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(snap.total_rows > 4, "total_rows={}", snap.total_rows);
        assert_eq!(
            snap.retained_bytes_estimate,
            snap.total_rows * snap.cols as usize * crate::PER_CELL_BYTES
        );
        assert_eq!(
            h.retained_bytes_estimate(),
            snap.retained_bytes_estimate,
            "fast-path estimate must match inspect snapshot"
        );
        h.stop();
    }
}
