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
            recent,
        }
    }
}
