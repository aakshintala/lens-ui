//! Measurement helpers: sample accumulation + percentile tables.

use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct SampleSet {
    samples: Vec<Duration>,
}

impl SampleSet {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(512),
        }
    }

    pub fn push(&mut self, d: Duration) {
        self.samples.push(d);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Compute p50 / p95 / p99 / max over samples after discarding `warmup`.
    pub fn percentiles_after_warmup(&self, warmup: usize) -> Option<Percentiles> {
        if self.samples.len() <= warmup {
            return None;
        }
        let mut vals: Vec<f64> = self.samples[warmup..]
            .iter()
            .map(|d| d.as_secs_f64() * 1000.0)
            .collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(Percentiles {
            n: vals.len(),
            p50: percentile(&vals, 0.50),
            p95: percentile(&vals, 0.95),
            p99: percentile(&vals, 0.99),
            max: *vals.last().unwrap(),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Percentiles {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Percentiles {
    pub fn row_ms(&self) -> String {
        format!(
            "n={:<4}  p50={:7.3}  p95={:7.3}  p99={:7.3}  max={:7.3}",
            self.n, self.p50, self.p95, self.p99, self.max
        )
    }
}

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx.min(sorted_ms.len() - 1)]
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub fixture: FixtureKind,
    pub cols: u16,
    pub rows: u16,
    pub strategy: crate::paint::Strategy,
    pub placement: crate::paint::TextPlacement,
    pub total_frames: usize,
    pub warmup: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureKind {
    FullRedraw,
    PartialUpdate,
    WideAndSgr,
}

impl FixtureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullRedraw => "full_redraw",
            Self::PartialUpdate => "partial_update",
            Self::WideAndSgr => "wide_and_sgr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full_redraw" => Some(Self::FullRedraw),
            "partial_update" => Some(Self::PartialUpdate),
            "wide_and_sgr" => Some(Self::WideAndSgr),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RunResult {
    pub label: String,
    pub paint: Option<Percentiles>,
    pub snapshot: Option<Percentiles>,
    pub input_to_first_paint_ms: Option<f64>,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub placement: String,
    pub alignment_ok: bool,
}

impl RunResult {
    pub fn print_block(&self) {
        println!("=== {} ===", self.label);
        println!("  placement={}  alignment_ok={}", self.placement, self.alignment_ok);
        if let Some(p) = self.paint {
            println!("  paint_ms     {}", p.row_ms());
        }
        if let Some(s) = self.snapshot {
            println!("  snapshot_ms  {}", s.row_ms());
        }
        if let Some(ms) = self.input_to_first_paint_ms {
            println!("  input→first-paint_ms  {ms:.3}");
        }
        if self.cache_hits + self.cache_misses > 0 {
            println!(
                "  cache hits={} misses={}",
                self.cache_hits, self.cache_misses
            );
        }
        println!();
    }
}
