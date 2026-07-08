// Probe — instrumentation for the streaming render (Task 5).
//
// Task-1 finding: gpui-component's markdown re-parse runs INSIDE the dep, async
// and debounced (200ms), off our render path — we cannot count its parses
// without vendoring. So the probe measures what we CAN observe from our side:
//
//   * per-tick wall time to build+notify the view (Instant-based), and
//   * whether that cost stays FLAT as the accumulated document grows.
//
// A synchronous full-reparse-every-frame renderer would make per-tick cost grow
// with document size; a debounced/async one keeps it flat. Combined with the
// structural guarantee (we always pass the SAME ElementId → no remount) and the
// visual verdict (scroll/selection/flicker, observed by a human), this is the
// stable-identity evidence. See NOTES.md.

use std::time::Duration;

pub struct Probe {
    samples: Vec<Sample>,
    stable_id: &'static str,
}

struct Sample {
    bytes: usize,
    build_us: u128,
}

impl Probe {
    pub fn new(stable_id: &'static str) -> Self {
        Self { samples: Vec::new(), stable_id }
    }

    /// Record one streaming tick: the accumulated byte length and how long the
    /// view build+notify took.
    pub fn note_tick(&mut self, bytes: usize, build: Duration) {
        self.samples.push(Sample { bytes, build_us: build.as_micros() });
    }

    /// Correlation of per-tick build time vs. accumulated bytes. A value near 0
    /// means build cost does NOT grow with document size (evidence the render
    /// path is not doing O(n) reparse per frame). Returns None if < 3 samples.
    fn build_time_growth(&self) -> Option<f64> {
        if self.samples.len() < 3 {
            return None;
        }
        let n = self.samples.len() as f64;
        let (mut sx, mut sy, mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for s in &self.samples {
            let x = s.bytes as f64;
            let y = s.build_us as f64;
            sx += x;
            sy += y;
            sxy += x * y;
            sxx += x * x;
            syy += y * y;
        }
        let num = n * sxy - sx * sy;
        let den = ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
        if den == 0.0 {
            Some(0.0)
        } else {
            Some(num / den)
        }
    }

    pub fn summary(&self) -> String {
        let ticks = self.samples.len();
        let max_bytes = self.samples.last().map(|s| s.bytes).unwrap_or(0);
        let mean_us = if ticks == 0 {
            0
        } else {
            self.samples.iter().map(|s| s.build_us).sum::<u128>() / ticks as u128
        };
        let max_us = self.samples.iter().map(|s| s.build_us).max().unwrap_or(0);
        let growth = self
            .build_time_growth()
            .map(|c| format!("{c:+.2}"))
            .unwrap_or_else(|| "n/a".into());
        format!(
            "PROBE: id={:?} (stable ⇒ no remount by construction) | ticks={ticks} \
             final_bytes={max_bytes} | build/tick mean={mean_us}µs max={max_us}µs | \
             build-time↔bytes correlation={growth} (≈0 ⇒ no O(n) per-frame reparse)",
            self.stable_id
        )
    }
}
