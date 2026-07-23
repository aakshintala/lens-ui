#![deny(warnings)]
//! Streaming/selection-preserve flag for the vendored TextView.
//! Lives OUTSIDE the `md/` cap-lints allow (see the inner `#![deny(warnings)]`)
//! so the P2 selection-preserve semantics stay reviewable and clippy-gated.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared flag telling the paint site whether a resize is driven by an in-flight
/// (or just-applied) markdown reparse, in which case the text selection must be
/// preserved across the height change. A genuine user resize clears the selection.
#[derive(Clone, Default)]
pub(crate) struct StreamingFlag {
    /// True while text updates are actively streaming/reparsing.
    streaming: Arc<AtomicBool>,
    /// One-shot latch: the single relayout caused by applying the FINAL reparse
    /// result must still preserve selection even though `streaming` has settled.
    preserve_once: Arc<AtomicBool>,
}

impl StreamingFlag {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A text delta arrived; a reparse is now in flight.
    pub(crate) fn mark_streaming(&self) {
        self.streaming.store(true, Ordering::Relaxed);
    }

    /// The reparse result has been applied and no more parses are pending.
    /// Clears streaming but arms a one-shot so the relayout triggered by THIS
    /// apply still preserves selection.
    pub(crate) fn mark_settled(&self) {
        self.streaming.store(false, Ordering::Relaxed);
        self.preserve_once.store(true, Ordering::Relaxed);
    }

    /// Read at the paint/prepaint site: preserve selection if actively streaming,
    /// or if this is the one relayout following the final apply (consumes the latch).
    pub(crate) fn preserve_on_resize(&self) -> bool {
        self.streaming.load(Ordering::Relaxed) || self.preserve_once.swap(false, Ordering::Relaxed)
    }
}
