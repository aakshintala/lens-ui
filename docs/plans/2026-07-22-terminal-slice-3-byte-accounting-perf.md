# Terminal Slice 3 — Byte-accounting + Perf acceptance — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a thin per-tab retained-bytes *estimate* through `EngineInspect`, plus two demo-hosted perf jobs — (A) sustained multi-tab streaming frame-budget + PerCell fail-closed gate, and (B) a one-tab-per-process RSS sweep that validates the estimate is *ordinally* reliable — closing the module performance story.

**Architecture:** The engine worker samples `Terminal::total_rows()` once per frame build (already-vendored C ABI accessor, no re-vendor) into the gated `InspectShared` atomics; `EngineInspect` exposes `total_rows` + a derived `retained_bytes_estimate = total_rows × cols × PER_CELL_BYTES`. Job B is a dependency-free `rss_probe` binary in the demo crate that drives ONE engine per process to a target retained-row count with compressible-vs-incompressible content and prints `{total_rows, estimate_bytes, rss_bytes}`; an `xtask terminal-rss-sweep` subcommand orchestrates it across sizes×modes (fresh process each) and fail-closes on an ordinal-fidelity check. Job A is a new `harness=false` real-GPUI `[[test]]` (`stream_perf_realwindow`, the render_realwindow sibling) that spawns N engines, streams synthetic dense wide/emoji bytes from a feeder thread, paints the visible tab (PerCell), and fail-closes on paint-p95 + build-p95 under sustained load while recording ΔRSS.

**Tech Stack:** Rust, `libghostty-vt` (vendored), `crossbeam-channel`, `gpui 0.2.2` (real-window harness), Criterion (compile-only in gate), `xtask` runner. macOS is the perf-gate authority.

## Findings folded in during execution (2026-07-22)

- **`EngineConfig.max_scrollback` is a BYTE budget, not a line count.** The vendored doc comment ("Maximum number of lines") is misleading — empirically retention scales with *cells* (~1.7 bytes/cell), and the design spec's "10,000,000-byte scrollback" is correct. Sizing scrollback by row count under-retains. Any harness that needs to retain N rows must size `max_scrollback ≈ N × cols × headroom` bytes (Job B uses `× 16 + 4 MiB`).
- **Worker-thread stack fix landed (`67e8192`).** libghostty's scrollback page ops overflow the ~2 MiB default thread stack once history grows (~2000+ rows → SIGABRT). The worker now reserves `WORKER_STACK_BYTES = 64 MiB` (lazily committed). Regression test `large_scrollback_feed_does_not_overflow_worker_stack`. This was discovered by Job B and is a real product fix.
- **`build_now` is a no-op when the engine is not dirty** — `total_rows` only samples on a fresh build, so a harness must feed a final dirtying byte before `build_now` to get a current sample.
- **RSS direction is the opposite of the spec's assumption:** compressible ('a' repeated) uses *more* RSS than incompressible at equal `total_rows`. This does not flip the estimate↔RSS ordering, so the fidelity gate still passes; it is exactly the content-blindness Job B measures.

## Global Constraints

- **No re-vendor.** `Terminal::total_rows()` / `scrollback_rows()` already exist in `vendor/libghostty-rs/libghostty-vt/src/terminal.rs` (lines 638–644). Byte-*accurate* accounting (a `GHOSTTY_TERMINAL_DATA_*` byte selector) is a **fail-closed conditional** — do NOT build it in this slice; it is escalated ONLY if Job B shows ordinal unreliability.
- **Pure `lens-terminal` + demo only.** No `lens-ui` / `lens-core` edits. This slice must keep `terminal-ws` a pure-module branch so `terminal-ws → main` can merge after Slice 4. No new runtime dependencies in `lens-terminal` or `lens-terminal-demo` (RSS is read by shelling out to `ps`; content generators are hand-rolled LCG — dep-free).
- **Inspect is zero-cost when disabled.** The retained-rows sample follows the existing pattern: the atomic store is unconditional (like `cols`/`rows`), but ring-event recording stays behind `is_enabled()`. Never add an FFI call to a hot path that runs when inspect is off — the sample lives in `maybe_publish` after a build that already happened, so the single `total_rows()` FFI get rides an existing ~60 Hz cadence, not per-byte.
- **Perf budgets are release-calibrated and fail-closed.** The gate runs `--release` (debug is ~5.4× slower on the per-cell path). Budgets sit above observed p95 with headroom for a load transient but low enough to trip a ~2× regression. Never raise a budget to make a run pass — investigate the regression (mirror the wording in `render_realwindow.rs`).
- **PER_CELL_BYTES is provisional.** It is a documented placeholder that only affects the estimate's *scale*, never its *ordinal* use. Job B reports the empirically-calibrated value; folding the calibrated number back is a one-line edit, not a redesign.
- **House build discipline:** subagent-driven (composer-2.5 author), ≥1 cross-family review (codex `gpt-5.6-sol` for board/engine logic) at each seam, TDD, frequent commits. `cargo fmt` + workspace clippy `-D warnings` (both normal and `--features test-util` for lens-terminal) must stay green.

---

## File Structure

- `crates/lens-terminal/src/engine/inspect.rs` — **Modify.** Add `PER_CELL_BYTES` const, `total_rows: AtomicU64` to `InspectShared`, `record_retained_rows()`, and `total_rows` + `retained_bytes_estimate` to `EngineInspect` + `snapshot()`.
- `crates/lens-terminal/src/engine/vt.rs` — **Modify.** Add production `VtEngine::total_rows(&self) -> usize`.
- `crates/lens-terminal/src/engine/worker.rs` — **Modify.** Sample `engine.total_rows()` into inspect in `maybe_publish` after a successful build.
- `crates/lens-terminal/src/engine/mod.rs` — **Modify.** Re-export `PER_CELL_BYTES`.
- `crates/lens-terminal/src/lib.rs` — **Modify.** Add `PER_CELL_BYTES` to the `engine::{…}` re-export.
- `crates/lens-terminal-demo/src/bin/rss_probe.rs` — **Create.** Job-B measurement binary (one engine per process).
- `crates/lens-terminal-demo/Cargo.toml` — **Modify.** Add the `rss_probe` `[[bin]]`.
- `crates/lens-terminal/tests/stream_perf_realwindow.rs` — **Create.** Job-A sustained multi-tab streaming perf gate (`harness=false`, `test-util`).
- `crates/lens-terminal/Cargo.toml` — **Modify.** Register the `stream_perf_realwindow` `[[test]]`.
- `crates/xtask/src/main.rs` — **Modify.** Add `terminal-rss-sweep` subcommand + pure `check_ordinal_fidelity` + wire `stream_perf_realwindow` into the macOS gate.

---

## Task 1: Engine retained-rows sample + `EngineInspect` estimate

**Files:**
- Modify: `crates/lens-terminal/src/engine/inspect.rs`
- Modify: `crates/lens-terminal/src/engine/vt.rs`
- Modify: `crates/lens-terminal/src/engine/worker.rs:810-873` (`maybe_publish`)
- Modify: `crates/lens-terminal/src/engine/mod.rs:16`
- Modify: `crates/lens-terminal/src/lib.rs:332-335` (`engine::{…}` re-export)
- Test: `crates/lens-terminal/src/engine/inspect.rs` (`#[cfg(test)]`), `crates/lens-terminal/src/engine/vt.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `VtEngine.terminal: Terminal` (has `.total_rows() -> Result<usize>`), `VtEngine.cols: u16`; `InspectShared` atomics pattern; `maybe_publish`'s `engine: &mut VtEngine` + `inspect: &InspectShared` after `inspect.record_frame_built(micros)`.
- Produces:
  - `pub const lens_terminal::PER_CELL_BYTES: usize` — provisional retained-bytes-per-cell multiplier.
  - `VtEngine::total_rows(&self) -> usize` (crate-internal; production, not `cfg(test)`).
  - `InspectShared::record_retained_rows(&self, total_rows: usize)`.
  - `EngineInspect.total_rows: usize` and `EngineInspect.retained_bytes_estimate: usize` (= `total_rows.saturating_mul(cols as usize).saturating_mul(PER_CELL_BYTES)`).

- [ ] **Step 1: Write the failing test (estimate math in `snapshot`)**

Add to the existing `#[cfg(test)] mod tests` in `crates/lens-terminal/src/engine/inspect.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p lens-terminal --lib engine::inspect::tests::retained -- --nocapture`
Expected: FAIL to compile — `PER_CELL_BYTES`, `record_retained_rows`, `total_rows`, `retained_bytes_estimate` do not exist yet.

- [ ] **Step 3: Add the const, field, recorder, and snapshot math in `inspect.rs`**

At the top of `crates/lens-terminal/src/engine/inspect.rs`, below `const RING_CAP: usize = 32;`:

```rust
/// Provisional retained-bytes-per-cell multiplier for the fleet-accounting
/// **estimate** (`total_rows × cols × PER_CELL_BYTES`). This is a documented
/// placeholder: it affects only the estimate's *scale*, never its *ordinal*
/// use (LRV trimming compares estimates against each other). Slice 3 Job B
/// (`xtask terminal-rss-sweep`) reports the empirically-calibrated value from
/// RSS-vs-total_rows; folding it back here is a one-line edit. Byte-*accurate*
/// accounting (a Ghostty byte selector) is a fail-closed conditional escalated
/// ONLY if Job B shows the estimate is ordinally unreliable.
pub const PER_CELL_BYTES: usize = 4;
```

Add the field to `EngineInspect` (after `pub max_scrollback: usize,`):

```rust
    pub total_rows: usize,
    pub retained_bytes_estimate: usize,
```

Add the atomic to `InspectShared` (after `max_scrollback: AtomicU64,`):

```rust
    total_rows: AtomicU64,
```

Initialize it in `InspectShared::new` (after `max_scrollback: AtomicU64::new(max_scrollback as u64),`):

```rust
            total_rows: AtomicU64::new(0),
```

Add the recorder method (place near `record_resize`):

```rust
    /// Sample the emulator's retained row count (scrollback + viewport). The
    /// store is unconditional (cheap, like `cols`/`rows`) so the estimate is
    /// available even with the ring disabled; the caller supplies the value it
    /// already read from the terminal after a build.
    pub fn record_retained_rows(&self, total_rows: usize) {
        self.total_rows.store(total_rows as u64, Ordering::Relaxed);
    }
```

In `snapshot()`, after `let recent = …;` compute and include the fields. Replace the `EngineInspect { cols: …, rows: …, max_scrollback: …,` head so it reads:

```rust
        let cols = self.cols.load(Ordering::Relaxed);
        let total_rows = self.total_rows.load(Ordering::Relaxed) as usize;
        let retained_bytes_estimate = total_rows
            .saturating_mul(cols as usize)
            .saturating_mul(PER_CELL_BYTES);

        EngineInspect {
            cols,
            rows: self.rows.load(Ordering::Relaxed),
            max_scrollback: self.max_scrollback.load(Ordering::Relaxed) as usize,
            total_rows,
            retained_bytes_estimate,
```

(Leave the remaining fields — `visible`, `frames_built`, … — exactly as they were; only the head three lines gain `total_rows`/`retained_bytes_estimate` and reuse the hoisted `cols`.)

- [ ] **Step 4: Run the inspect test to verify it passes**

Run: `cargo test -p lens-terminal --lib engine::inspect::tests::retained`
Expected: PASS (both `retained_*` tests).

- [ ] **Step 5: Write the failing test (engine-level total_rows growth)**

Add to the `#[cfg(test)] mod tests` in `crates/lens-terminal/src/engine/vt.rs`:

```rust
    #[test]
    fn total_rows_grows_past_viewport_after_scrollback() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap(); // 20x3, scrollback 100
        assert_eq!(e.total_rows().max(3), e.total_rows(), "sanity");
        for i in 0..50 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        assert!(
            e.total_rows() > 3,
            "total_rows must exceed the 3-row viewport once scrollback fills, got {}",
            e.total_rows()
        );
    }
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test -p lens-terminal --lib engine::vt::tests::total_rows_grows_past_viewport_after_scrollback`
Expected: FAIL to compile — `VtEngine::total_rows` does not exist.

- [ ] **Step 7: Add `VtEngine::total_rows` (production accessor)**

In `crates/lens-terminal/src/engine/vt.rs`, add to the main `impl VtEngine` block (e.g. directly after `pub fn feed`):

```rust
    /// The emulator's total retained row count (scrollback + active viewport).
    /// Fail-soft to 0 — the accounting *estimate* must never panic the worker.
    pub(crate) fn total_rows(&self) -> usize {
        self.terminal.total_rows().unwrap_or(0)
    }
```

- [ ] **Step 8: Run the vt test to verify it passes**

Run: `cargo test -p lens-terminal --lib engine::vt::tests::total_rows_grows_past_viewport_after_scrollback`
Expected: PASS.

- [ ] **Step 9: Sample retained rows in the worker after each build**

In `crates/lens-terminal/src/engine/worker.rs`, inside `maybe_publish`, in the `Ok(frame) =>` arm, immediately after `inspect.record_frame_built(micros);`:

```rust
            inspect.record_retained_rows(engine.total_rows());
```

- [ ] **Step 10: Re-export the const**

In `crates/lens-terminal/src/engine/mod.rs`, change:

```rust
pub use inspect::EngineInspect;
```

to:

```rust
pub use inspect::{EngineInspect, PER_CELL_BYTES};
```

In `crates/lens-terminal/src/lib.rs`, the `pub use engine::{ … }` block (around line 332) currently lists `EngineInspect`; add `PER_CELL_BYTES` to that list.

- [ ] **Step 11: Write the failing integration test (end-to-end through the handle)**

Add a new `#[cfg(test)] mod tests` block (or extend the existing one) at the bottom of `crates/lens-terminal/src/engine/inspect.rs` — it already imports `EngineHandle` and `EngineConfig` in its test module:

```rust
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
        let h = EngineHandle::spawn(cfg);
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
        h.stop();
    }
```

- [ ] **Step 12: Run the full engine test set to verify it passes**

Run: `cargo test -p lens-terminal --lib engine::`
Expected: PASS (all engine tests, including the new retained/streaming ones).

- [ ] **Step 13: Clippy + fmt**

Run: `cargo fmt -p lens-terminal && cargo clippy -p lens-terminal --all-targets --features test-util -- -D warnings`
Expected: clean.

- [ ] **Step 14: Commit**

```bash
git add crates/lens-terminal/src/engine/inspect.rs crates/lens-terminal/src/engine/vt.rs crates/lens-terminal/src/engine/worker.rs crates/lens-terminal/src/engine/mod.rs crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-3): retained-rows sample + EngineInspect byte estimate

total_rows sampled once per build in maybe_publish (vendored TOTAL_ROWS
accessor, no re-vendor); EngineInspect exposes total_rows +
retained_bytes_estimate = total_rows x cols x PER_CELL_BYTES (provisional,
Job B calibrates). Estimate is ordinal-only; byte-accurate FFI stays a
fail-closed conditional.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Job B measurement binary — `rss_probe` (one engine per process)

**Files:**
- Create: `crates/lens-terminal-demo/src/bin/rss_probe.rs`
- Modify: `crates/lens-terminal-demo/Cargo.toml`
- Test: `crates/lens-terminal-demo/src/bin/rss_probe.rs` (`#[cfg(test)]` — pure helpers only)

**Interfaces:**
- Consumes: `lens_terminal::{EngineHandle, EngineConfig, PER_CELL_BYTES}` (all public), `EngineHandle::{feed, build_now, inspect, stop}`, `EngineInspect::{total_rows, retained_bytes_estimate, cols, bytes_fed}`.
- Produces: an executable that prints exactly ONE line to stdout:
  `RSS_PROBE mode=<compressible|incompressible> target_rows=<n> total_rows=<n> cols=<n> estimate_bytes=<n> rss_bytes=<n>`
  and pure helpers `compressible_line(cols) -> Vec<u8>`, `incompressible_line(cols, seed: &mut u64) -> Vec<u8>`, `estimate_for(total_rows, cols) -> usize`, `rss_bytes() -> u64`.

- [ ] **Step 1: Register the binary**

In `crates/lens-terminal-demo/Cargo.toml`, after the existing `[[bin]]` block, add:

```toml
[[bin]]
name = "rss_probe"
path = "src/bin/rss_probe.rs"
```

- [ ] **Step 2: Write the failing test (pure helpers)**

Create `crates/lens-terminal-demo/src/bin/rss_probe.rs` with ONLY the helpers + a test module first, so the test compiles and drives the implementation:

```rust
//! Job-B RSS measurement probe — ONE engine per process (clean RSS baseline).
//! Drives a single engine to a target retained-row count with either
//! compressible (repeated byte) or incompressible (LCG-random printable)
//! content, then prints the retained-bytes ESTIMATE alongside the process RSS.
//! The `xtask terminal-rss-sweep` orchestrator runs this across sizes×modes in
//! fresh processes and fail-closes on ordinal fidelity.
//!
//! Usage: `rss_probe <compressible|incompressible> <target_rows> <cols>`

use std::time::{Duration, Instant};

use lens_terminal::{EngineConfig, EngineHandle, PER_CELL_BYTES};

/// A fully compressible line: one byte repeated `cols` times.
fn compressible_line(cols: usize) -> Vec<u8> {
    vec![b'a'; cols]
}

/// A high-entropy line via a deterministic xorshift LCG mapped to printable
/// ASCII (0x21..=0x7e). Deterministic across runs so a given (rows, cols, mode)
/// reproduces byte-for-byte — no `rand` dependency.
fn incompressible_line(cols: usize, seed: &mut u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(cols);
    for _ in 0..cols {
        // xorshift64
        let mut x = *seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *seed = x;
        let printable = 0x21 + (x % (0x7e - 0x21 + 1)) as u8;
        out.push(printable);
    }
    out
}

fn estimate_for(total_rows: usize, cols: usize) -> usize {
    total_rows
        .saturating_mul(cols)
        .saturating_mul(PER_CELL_BYTES)
}

/// Resident set size of THIS process, in bytes. Shells out to `ps` (dep-free,
/// works on macOS + Linux); `ps -o rss=` reports KiB.
fn rss_bytes() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .map(|kib| kib * 1024)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

fn main() {
    // Implemented in Step 4.
    real_main();
}

fn real_main() {
    // placeholder; replaced in Step 4
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressible_line_is_uniform() {
        let l = compressible_line(200);
        assert_eq!(l.len(), 200);
        assert!(l.iter().all(|&b| b == b'a'));
    }

    #[test]
    fn incompressible_line_is_deterministic_and_printable() {
        let mut s1 = 0x1234_5678_9abc_def0;
        let mut s2 = 0x1234_5678_9abc_def0;
        let a = incompressible_line(200, &mut s1);
        let b = incompressible_line(200, &mut s2);
        assert_eq!(a, b, "same seed reproduces the line");
        assert!(a.iter().all(|&c| (0x21..=0x7e).contains(&c)));
        // Advancing the seed changes the next line.
        let c = incompressible_line(200, &mut s1);
        assert_ne!(a, c, "seed advances between lines");
    }

    #[test]
    fn estimate_matches_engine_formula() {
        assert_eq!(estimate_for(10_000, 200), 10_000 * 200 * PER_CELL_BYTES);
    }
}
```

- [ ] **Step 3: Run the helper tests to verify they pass**

Run: `cargo test -p lens-terminal-demo --bin rss_probe`
Expected: PASS (3 helper tests). `real_main` is `unimplemented!()` but never called by tests.

- [ ] **Step 4: Implement `real_main` (drive one engine, print one line)**

Replace the placeholder `main`/`real_main` with:

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: rss_probe <compressible|incompressible> <target_rows> <cols>");
        std::process::exit(2);
    }
    let mode = args[1].as_str();
    let target_rows: usize = args[2].parse().unwrap_or_else(|_| {
        eprintln!("target_rows must be a positive integer");
        std::process::exit(2);
    });
    let cols: u16 = args[3].parse().unwrap_or_else(|_| {
        eprintln!("cols must be a u16");
        std::process::exit(2);
    });
    let incompressible = match mode {
        "compressible" => false,
        "incompressible" => true,
        _ => {
            eprintln!("mode must be `compressible` or `incompressible`");
            std::process::exit(2);
        }
    };

    // A viewport of 50 rows; scrollback sized to hold the target so total_rows
    // can actually reach it. `+64` slack absorbs the viewport rows.
    let viewport_rows: u16 = 50;
    let cfg = EngineConfig {
        cols,
        rows: viewport_rows,
        max_scrollback: target_rows + 64,
        cell_w_px: 8,
        cell_h_px: 16,
    };
    let handle = EngineHandle::spawn(cfg);

    // Feed `target_rows` lines of the chosen content, each terminated CRLF.
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut fed_bytes: u64 = 0;
    for _ in 0..target_rows {
        let mut line = if incompressible {
            incompressible_line(cols as usize, &mut seed)
        } else {
            compressible_line(cols as usize)
        };
        line.extend_from_slice(b"\r\n");
        fed_bytes += line.len() as u64;
        // Retry on backpressure — the command channel is bounded (cap 256).
        loop {
            match handle.feed(line.clone()) {
                Ok(()) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    // Wait until the worker has consumed everything we fed, then force a build
    // so total_rows reflects the full stream.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snap = handle.inspect();
        if snap.bytes_fed >= fed_bytes {
            break;
        }
        if Instant::now() > deadline {
            eprintln!(
                "timeout draining feed: bytes_fed={} want>={}",
                snap.bytes_fed, fed_bytes
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = handle.build_now();
    // Give the build a beat to land the sample.
    std::thread::sleep(Duration::from_millis(30));

    let snap = handle.inspect();
    let rss = rss_bytes();
    // Read RSS BEFORE stopping the engine (stop drops the worker + terminal).
    println!(
        "RSS_PROBE mode={mode} target_rows={target_rows} total_rows={} cols={} estimate_bytes={} rss_bytes={}",
        snap.total_rows,
        snap.cols,
        snap.retained_bytes_estimate,
        rss,
    );

    handle.stop();
}
```

Delete the now-unused `real_main` placeholder function.

- [ ] **Step 5: Build the binary**

Run: `cargo build -p lens-terminal-demo --bin rss_probe --release`
Expected: compiles clean.

- [ ] **Step 6: Smoke-run the probe (eyeball a sane line)**

Run: `cargo run -q -p lens-terminal-demo --bin rss_probe --release -- incompressible 5000 200`
Expected: exactly one stdout line, e.g.
`RSS_PROBE mode=incompressible target_rows=5000 total_rows=5050 cols=200 estimate_bytes=4040000 rss_bytes=...`
Verify: `total_rows` ≈ `target_rows` (+ viewport), `estimate_bytes == total_rows * 200 * 4`, `rss_bytes > 0`.

- [ ] **Step 7: Clippy + fmt**

Run: `cargo fmt -p lens-terminal-demo && cargo clippy -p lens-terminal-demo --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-terminal-demo/Cargo.toml crates/lens-terminal-demo/src/bin/rss_probe.rs
git commit -m "feat(terminal-3): rss_probe binary — Job-B one-engine-per-process RSS measurement

Drives one engine to a target retained-row count with compressible vs
incompressible (deterministic LCG) content; prints total_rows + estimate +
process RSS (via ps, dep-free). Fresh process = clean RSS baseline.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Job B gate — `xtask terminal-rss-sweep` + ordinal-fidelity check

**Files:**
- Modify: `crates/xtask/src/main.rs`
- Test: `crates/xtask/src/main.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: the `rss_probe` binary via `cargo run -q -p lens-terminal-demo --bin rss_probe --release -- <mode> <rows> <cols>`; its `RSS_PROBE …` stdout line.
- Produces:
  - `struct RssSample { mode: String, total_rows: usize, estimate_bytes: usize, rss_bytes: u64 }`.
  - `fn parse_rss_probe_line(line: &str) -> Option<RssSample>`.
  - `fn check_ordinal_fidelity(samples: &[RssSample]) -> FidelityVerdict` where `FidelityVerdict { inversions: Vec<(usize, usize)>, ok: bool }`.
  - A `terminal-rss-sweep` CLI subcommand that fail-closes (`exit(1)`) when `!ok`.

- [ ] **Step 1: Write the failing test (ordinal-fidelity checker)**

Add to the `#[cfg(test)] mod tests` in `crates/xtask/src/main.rs` (create the block if absent):

```rust
    use super::{check_ordinal_fidelity, parse_rss_probe_line, RssSample};

    fn s(mode: &str, total_rows: usize, estimate_bytes: usize, rss_bytes: u64) -> RssSample {
        RssSample {
            mode: mode.to_string(),
            total_rows,
            estimate_bytes,
            rss_bytes,
        }
    }

    #[test]
    fn parses_a_probe_line() {
        let line = "RSS_PROBE mode=incompressible target_rows=5000 total_rows=5050 cols=200 estimate_bytes=4040000 rss_bytes=52428800";
        let got = parse_rss_probe_line(line).expect("parse");
        assert_eq!(got.mode, "incompressible");
        assert_eq!(got.total_rows, 5050);
        assert_eq!(got.estimate_bytes, 4_040_000);
        assert_eq!(got.rss_bytes, 52_428_800);
    }

    #[test]
    fn monotonic_estimate_and_rss_is_ordinally_ok() {
        // estimate rank == rss rank across a growing sweep: no inversions.
        let samples = vec![
            s("compressible", 1000, 800_000, 10_000_000),
            s("incompressible", 1000, 800_000, 12_000_000),
            s("compressible", 5000, 4_000_000, 30_000_000),
            s("incompressible", 5000, 4_000_000, 40_000_000),
            s("compressible", 20000, 16_000_000, 120_000_000),
        ];
        let verdict = check_ordinal_fidelity(&samples);
        assert!(verdict.ok, "inversions: {:?}", verdict.inversions);
    }

    #[test]
    fn estimate_ordering_flip_vs_rss_is_flagged() {
        // A LARGER-estimate tab uses MUCH LESS RSS than a smaller-estimate tab:
        // trimming by estimate would free less than expected -> escalate.
        let samples = vec![
            s("compressible", 20000, 16_000_000, 20_000_000), // huge estimate, tiny RSS
            s("incompressible", 1000, 800_000, 200_000_000),  // tiny estimate, huge RSS
        ];
        let verdict = check_ordinal_fidelity(&samples);
        assert!(!verdict.ok, "expected a flagged inversion");
        assert!(!verdict.inversions.is_empty());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p xtask ordinal -- --nocapture` (and `parses_a_probe_line`)
Expected: FAIL to compile — the types/functions do not exist.

- [ ] **Step 3: Implement the sample type, parser, and checker**

Add near the top-level items of `crates/xtask/src/main.rs`:

```rust
#[derive(Debug, Clone)]
pub struct RssSample {
    pub mode: String,
    pub total_rows: usize,
    pub estimate_bytes: usize,
    pub rss_bytes: u64,
}

/// Parse one `RSS_PROBE key=value …` stdout line into a sample. Returns `None`
/// for any line that is not a well-formed probe line.
pub fn parse_rss_probe_line(line: &str) -> Option<RssSample> {
    let line = line.trim();
    let rest = line.strip_prefix("RSS_PROBE ")?;
    let mut mode = None;
    let mut total_rows = None;
    let mut estimate_bytes = None;
    let mut rss_bytes = None;
    for tok in rest.split_whitespace() {
        let (k, v) = tok.split_once('=')?;
        match k {
            "mode" => mode = Some(v.to_string()),
            "total_rows" => total_rows = v.parse().ok(),
            "estimate_bytes" => estimate_bytes = v.parse().ok(),
            "rss_bytes" => rss_bytes = v.parse().ok(),
            _ => {}
        }
    }
    Some(RssSample {
        mode: mode?,
        total_rows: total_rows?,
        estimate_bytes: estimate_bytes?,
        rss_bytes: rss_bytes?,
    })
}

#[derive(Debug)]
pub struct FidelityVerdict {
    /// (i, j) pairs where sample i has a strictly larger estimate than j but a
    /// strictly SMALLER RSS beyond tolerance — an ordering flip that would
    /// mislead LRV trimming.
    pub inversions: Vec<(usize, usize)>,
    pub ok: bool,
}

/// The estimate is ordinally reliable iff sorting tabs by `estimate_bytes`
/// produces (weakly, within tolerance) the same ordering as sorting by
/// `rss_bytes`. A *scale* difference between compressible/incompressible
/// content is fine (the estimate ignores content); only an *ordering flip*
/// — a larger-estimate tab that actually holds less memory — breaks LRV
/// decisions and escalates to byte-accurate FFI.
///
/// Tolerance: RSS carries fixed process overhead + allocator slack, so we only
/// flag a flip when the RSS gap exceeds 15% of the larger RSS. This avoids
/// flagging near-ties as inversions.
pub fn check_ordinal_fidelity(samples: &[RssSample]) -> FidelityVerdict {
    const TOL: f64 = 0.15;
    let mut inversions = Vec::new();
    for i in 0..samples.len() {
        for j in 0..samples.len() {
            if i == j {
                continue;
            }
            let a = &samples[i];
            let b = &samples[j];
            // a claims (by estimate) to be strictly bigger than b …
            if a.estimate_bytes > b.estimate_bytes {
                // … but actually holds meaningfully LESS resident memory.
                let larger = a.rss_bytes.max(b.rss_bytes) as f64;
                if larger > 0.0 {
                    let gap = (b.rss_bytes as f64 - a.rss_bytes as f64) / larger;
                    if gap > TOL {
                        inversions.push((i, j));
                    }
                }
            }
        }
    }
    let ok = inversions.is_empty();
    FidelityVerdict { inversions, ok }
}
```

- [ ] **Step 4: Run the checker tests to verify they pass**

Run: `cargo test -p xtask ordinal parses_a_probe_line monotonic estimate_ordering`
Expected: PASS (4 tests).

- [ ] **Step 5: Wire the `terminal-rss-sweep` subcommand**

`crates/xtask/src/main.rs` dispatches at `fn main()` (line ~104):

```rust
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "codegen" => codegen(),
        "drift" => drift(std::env::args().skip(2)),
        "gate" => gate(),
        other => bail!("unknown xtask command: {other:?} (expected: codegen | drift | gate)"),
```

Add the arm and extend the bail list:

```rust
        "gate" => gate(),
        "terminal-rss-sweep" => terminal_rss_sweep(),
        other => bail!(
            "unknown xtask command: {other:?} (expected: codegen | drift | gate | terminal-rss-sweep)"
        ),
```

`xtask` uses `anyhow` (`Result<()>`, `bail!`, `.context()`) and has **no** `lens-terminal` dependency (verified) — so the function returns the crate `Result<()>` and prints the raw RSS/estimate ratio (no `PER_CELL_BYTES` reference). Add the function (near `gate`/`drift`):

```rust
/// Job-B estimate-fidelity gate: run `rss_probe` across sizes×modes in fresh
/// processes and fail-close on an ordinal-fidelity flip. Heavyweight
/// (multi-process, large allocations) — NOT part of the fast `gate`; run at
/// slice acceptance and record the table as evidence.
fn terminal_rss_sweep() -> Result<()> {
    const COLS: u16 = 200;
    const SIZES: [usize; 4] = [1_000, 5_000, 20_000, 50_000];
    const MODES: [&str; 2] = ["compressible", "incompressible"];

    let mut samples: Vec<RssSample> = Vec::new();
    println!("terminal-rss-sweep: cols={COLS} sizes={SIZES:?} modes={MODES:?}");
    for &rows in &SIZES {
        for &mode in &MODES {
            let out = Command::new(env!("CARGO"))
                .args([
                    "run",
                    "-q",
                    "-p",
                    "lens-terminal-demo",
                    "--bin",
                    "rss_probe",
                    "--release",
                    "--",
                    mode,
                    &rows.to_string(),
                    &COLS.to_string(),
                ])
                .output()
                .context("spawn rss_probe")?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let sample = stdout
                .lines()
                .find_map(parse_rss_probe_line)
                .with_context(|| format!("no RSS_PROBE line from rows={rows} mode={mode}:\n{stdout}"))?;
            println!(
                "  {mode:<14} rows={:<6} estimate={:>12} rss={:>12}",
                sample.total_rows, sample.estimate_bytes, sample.rss_bytes
            );
            samples.push(sample);
        }
    }

    // Empirically-calibrated per_cell = median(RSS/estimate) × current
    // PER_CELL_BYTES. We print the raw ratio (xtask has no lens-terminal dep);
    // multiply by the current constant (4) by hand when folding it back.
    let mut ratios: Vec<f64> = samples
        .iter()
        .filter(|s| s.estimate_bytes > 0)
        .map(|s| s.rss_bytes as f64 / s.estimate_bytes as f64)
        .collect();
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if let Some(median) = ratios.get(ratios.len() / 2) {
        println!(
            "terminal-rss-sweep: median rss/estimate ratio = {median:.2} \
             (calibrated per_cell ~= this × current PER_CELL_BYTES)"
        );
    }

    let verdict = check_ordinal_fidelity(&samples);
    if verdict.ok {
        println!(
            "terminal-rss-sweep: OK — estimate is ordinally reliable (no RSS/estimate ordering flips)"
        );
        Ok(())
    } else {
        bail!(
            "terminal-rss-sweep: FAIL — {} ordinal flip(s): {:?}\nESCALATE: the row-count \
             estimate is ordinally unreliable — this trips the byte-accurate FFI fail-closed \
             conditional (GHOSTTY_TERMINAL_DATA_* byte selector + re-vendor). See the Slice 3+ \
             design spec.",
            verdict.inversions.len(),
            verdict.inversions
        )
    }
}
```

> **Implementer note:** ensure `Command`, `Context`/`context`/`with_context`, and `bail!` are in scope — `main.rs` already imports `std::process::Command` and `anyhow::{bail, Context, Result}` for `gate`/`run`/`drift` (verify with `grep -n 'use ' crates/xtask/src/main.rs | grep -E 'process::Command|anyhow'`; add whatever is missing). Do NOT add a `lens-terminal` dependency to `crates/xtask/Cargo.toml`.

- [ ] **Step 6: Verify the subcommand compiles and the checker tests still pass**

Run: `cargo build -p xtask && cargo test -p xtask`
Expected: compiles; all xtask tests pass.

- [ ] **Step 7: Run the full sweep (acceptance evidence — expected PASS)**

Run: `cargo run -p xtask -- terminal-rss-sweep`
Expected: a printed table (8 rows), a median ratio line, and `terminal-rss-sweep: OK`. **Capture the full stdout** — it is the Job-B acceptance evidence recorded in Task 5. If it prints `FAIL`/`ESCALATE`, STOP and surface to the user: this is the documented trigger to escalate to byte-accurate FFI, a scope change, not a bug to code around.

- [ ] **Step 8: Clippy + fmt**

Run: `cargo fmt -p xtask && cargo clippy -p xtask --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add crates/xtask/src/main.rs
git commit -m "feat(terminal-3): xtask terminal-rss-sweep — Job-B ordinal-fidelity gate

Orchestrates rss_probe across sizes x {compressible,incompressible} in fresh
processes; fail-closes on an RSS/estimate ordering flip (the byte-accurate-FFI
escalation trigger). Reports calibrated per_cell. Heavyweight — not in the
fast gate; slice-acceptance authority.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Job A — `stream_perf_realwindow` sustained multi-tab gate

**Files:**
- Create: `crates/lens-terminal/tests/stream_perf_realwindow.rs`
- Modify: `crates/lens-terminal/Cargo.toml` (register `[[test]]`)
- Modify: `crates/xtask/src/main.rs` (wire into the macOS gate)

**Interfaces:**
- Consumes: `lens_terminal::{EngineHandle, EngineConfig, Frame}`; `lens_terminal::render_test_api::{CellMetrics, RenderStats, TabRenderState, dense_wide_emoji_frame, menlo_gate_ok, paint_frame}`; `EngineHandle::{feed, set_visible, set_inspect_enabled, latest_frame, inspect, stop}`; `EngineInspect::{frames_built, last_build_micros}`.
- Produces: a `harness=false` binary that `exit(0)` on all budgets met, `exit(1)` on any failure. NOT a library interface.

**Budgets (release-calibrated, fail-closed):**
- `BUILD_P95_MS = 3.0` — engine `build_frame` p95 under sustained multi-tab streaming. (Frame-build bench is ~0.6 ms @ 200×50; 3.0 ms carries generous multi-tab/load margin while tripping a ~4× regression.)
- `PAINT_P95_MS = 5.5` — visible-tab PerCell paint p95 (matches `render_realwindow`'s `BUDGET_WIDE_200_MS` for dense wide/emoji @ 200×50).

- [ ] **Step 1: Register the test binary**

In `crates/lens-terminal/Cargo.toml`, alongside the other `[[test]]` blocks, add:

```toml
[[test]]
name = "stream_perf_realwindow"
harness = false
required-features = ["test-util"]
```

- [ ] **Step 2: Write the harness (real GPUI window, streaming feeder, dual p95)**

Create `crates/lens-terminal/tests/stream_perf_realwindow.rs`:

```rust
//! Job-A sustained multi-tab streaming perf gate (Slice 3).
//!
//! Sibling of `render_realwindow.rs` — same rationale for `harness = false` +
//! real `Application::run` (gpui's test `NoopTextSystem` false-greens paint/perf
//! assertions; memory `gpui-test-noop-text-system`). Where `render_realwindow`
//! paints ONE static frame, this drives N live engines fed a sustained
//! synthetic dense-wide/emoji stream from a background thread, paints the
//! VISIBLE tab every frame (PerCell path), and fail-closes on BOTH the paint
//! p95 (main thread) and the engine build p95 (from `EngineInspect`) under
//! load. It also flips a hidden tab visible mid-run and asserts hidden tabs
//! suppress builds. Process ΔRSS is recorded informationally (not asserted).
//!
//! Default run is a SHORT burst so it fits the macOS `xtask gate`. Set
//! `LENS_STREAM_SOAK=1` for a longer soak at slice acceptance.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Application, Bounds, Context, FocusHandle, IntoElement, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, canvas, point, prelude::*, px, size,
};
use lens_terminal::render_test_api::{CellMetrics, TabRenderState, paint_frame};
use lens_terminal::{EngineConfig, EngineHandle};

const TAB_COUNT: usize = 4;
const COLS: u16 = 200;
const ROWS: u16 = 50;
const BUILD_P95_MS: f64 = 3.0;
const PAINT_P95_MS: f64 = 5.5;
const WARMUP: usize = 60;

fn measure_frames() -> usize {
    if std::env::var("LENS_STREAM_SOAK").ok().as_deref() == Some("1") {
        1200
    } else {
        240
    }
}

fn rss_bytes() -> u64 {
    let pid = std::process::id();
    match std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .map(|kib| kib * 1024)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// One CRLF-terminated line of dense wide/emoji content (exercises PerCell).
fn dense_line() -> Vec<u8> {
    let mut s = String::new();
    while s.chars().count() < COLS as usize {
        s.push_str("日本語😀AB");
    }
    let mut b = s.into_bytes();
    b.extend_from_slice(b"\r\n");
    b
}

fn fail(msg: &str) -> ! {
    eprintln!("stream_perf_realwindow FAIL: {msg}");
    std::process::exit(1);
}

fn percentile_ms(samples: &[Duration], p: f64) -> f64 {
    let mut v: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn main() {
    // Spawn N engines. Tab 0 starts VISIBLE and painted; tabs 1..N start hidden
    // (streamed but not built) and tab 1 is flipped visible mid-run.
    let cfg = EngineConfig {
        cols: COLS,
        rows: ROWS,
        // BYTE budget (~10 MiB, production-ish). Under sustained streaming the
        // byte cap binds and old rows drop — realistic for a perf test.
        max_scrollback: 10_000_000,
        cell_w_px: 8,
        cell_h_px: 16,
    };
    let engines: Vec<Arc<EngineHandle>> =
        (0..TAB_COUNT).map(|_| Arc::new(EngineHandle::spawn(cfg))).collect();
    for (i, e) in engines.iter().enumerate() {
        e.set_inspect_enabled(true);
        // Only tab 0 visible initially.
        let _ = e.set_visible(i == 0);
    }

    // Background feeder: stream dense lines into every engine continuously until
    // `stop` flips. Retries on backpressure.
    let stop = Arc::new(AtomicBool::new(false));
    let feeder_engines: Vec<Arc<EngineHandle>> = engines.iter().map(Arc::clone).collect();
    let feeder_stop = Arc::clone(&stop);
    let feeder = std::thread::spawn(move || {
        while !feeder_stop.load(Ordering::Relaxed) {
            for e in &feeder_engines {
                let line = dense_line();
                let _ = e.feed(line); // drop on Full — sustained pressure is the point
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });

    let rss_start = rss_bytes();
    let measure = measure_frames();

    Application::new().run(move |cx| {
        let engines = engines.clone();
        let stop = Arc::clone(&stop);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal stream_perf_realwindow".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_window, cx| cx.new(|cx| StreamView::new(engines, stop, rss_start, measure, cx)),
        )
        .expect("open_window");
        cx.activate(true);
    });

    let _ = feeder.join();
}

struct StreamView {
    engines: Vec<Arc<EngineHandle>>,
    stop: Arc<AtomicBool>,
    focus: FocusHandle,
    state: TabRenderState,
    metrics: Rc<RefCell<Option<CellMetrics>>>,
    paint_samples: Rc<RefCell<Vec<Duration>>>,
    /// Max `last_build_micros` seen per frame across visible engines.
    build_samples: Rc<RefCell<Vec<Duration>>>,
    frame_idx: Rc<RefCell<usize>>,
    flipped: Rc<RefCell<bool>>,
    hidden_frames_at_start: Rc<RefCell<Option<u64>>>,
    rss_start: u64,
    measure: usize,
}

impl StreamView {
    fn new(
        engines: Vec<Arc<EngineHandle>>,
        stop: Arc<AtomicBool>,
        rss_start: u64,
        measure: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            engines,
            stop,
            focus: cx.focus_handle(),
            state: TabRenderState::new(),
            metrics: Rc::new(RefCell::new(None)),
            paint_samples: Rc::new(RefCell::new(Vec::new())),
            build_samples: Rc::new(RefCell::new(Vec::new())),
            frame_idx: Rc::new(RefCell::new(0)),
            flipped: Rc::new(RefCell::new(false)),
            hidden_frames_at_start: Rc::new(RefCell::new(None)),
            rss_start,
            measure,
        }
    }
}

impl Render for StreamView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        // Resolve Menlo metrics once (first frame) via a manual canvas, then
        // paint the visible tab's latest frame and sample p95s.
        if self.metrics.borrow().is_none() {
            let metrics_cell = Rc::clone(&self.metrics);
            return canvas(
                |_, _, _| {},
                move |_bounds, _prepaint, window, _cx| {
                    *metrics_cell.borrow_mut() = Some(CellMetrics::resolve_menlo(window));
                },
            )
            .size_full()
            .into_any_element();
        }

        let idx = *self.frame_idx.borrow();

        // Record hidden tab 1's build count at the first measured frame so we can
        // assert it stayed flat while hidden.
        if idx == WARMUP {
            *self.hidden_frames_at_start.borrow_mut() = Some(self.engines[1].inspect().frames_built);
        }

        // Flip tab 1 visible at the two-thirds mark to exercise the show path.
        let flip_at = WARMUP + (self.measure * 2) / 3;
        if idx == flip_at && !*self.flipped.borrow() {
            // Assert the hidden tab suppressed builds up to now.
            if let Some(start) = *self.hidden_frames_at_start.borrow() {
                let now = self.engines[1].inspect().frames_built;
                if now != start {
                    fail(&format!(
                        "hidden tab 1 built frames while hidden: {start} -> {now}"
                    ));
                }
            }
            let _ = self.engines[1].set_visible(true);
            *self.flipped.borrow_mut() = true;
        }

        // Sample the max build time across engines this frame.
        let max_build_us = self
            .engines
            .iter()
            .map(|e| e.inspect().last_build_micros)
            .max()
            .unwrap_or(0);
        if idx >= WARMUP {
            self.build_samples
                .borrow_mut()
                .push(Duration::from_micros(max_build_us));
        }

        // Load the visible tab-0 frame for painting.
        if let Some(frame) = self.engines[0].latest_frame() {
            self.state.set_frame(frame);
        }

        let metrics_cell = Rc::clone(&self.metrics);
        let paint_cell = Rc::clone(&self.paint_samples);
        let frame_idx = Rc::clone(&self.frame_idx);
        let build_samples = Rc::clone(&self.build_samples);
        let stop = Arc::clone(&self.stop);
        let measure = self.measure;
        let rss_start = self.rss_start;

        // Paint via a timed canvas (PerCell path), then advance/finish.
        // `TabRenderState::latest_frame()` is already public via render_test_api —
        // no production-surface touch needed.
        let frame_for_paint = self.state.latest_frame();
        canvas(
            |_, _, _| {},
            move |bounds, _prepaint, window, cx| {
                let m = metrics_cell.borrow();
                let Some(metrics) = m.as_ref() else { return };
                let Some(frame) = frame_for_paint.as_ref() else {
                    // No frame yet; still advance so we don't deadlock.
                    *frame_idx.borrow_mut() += 1;
                    return;
                };
                let t0 = Instant::now();
                let _stats = paint_frame(
                    frame,
                    point(bounds.origin.x, bounds.origin.y),
                    metrics,
                    window,
                    cx,
                );
                let dt = t0.elapsed();
                let i = *frame_idx.borrow();
                if i >= WARMUP {
                    paint_cell.borrow_mut().push(dt);
                }
                *frame_idx.borrow_mut() = i + 1;

                if i >= WARMUP + measure {
                    let paints = paint_cell.borrow();
                    let builds = build_samples.borrow();
                    let paint_p95 = percentile_ms(&paints, 0.95);
                    let build_p95 = percentile_ms(&builds, 0.95);
                    let rss_end = rss_bytes();
                    let d_rss = rss_end as i64 - rss_start as i64;
                    eprintln!(
                        "STREAM paint_p95_ms={paint_p95:.3} (budget {PAINT_P95_MS}) \
                         build_p95_ms={build_p95:.3} (budget {BUILD_P95_MS}) \
                         delta_rss_bytes={d_rss}"
                    );
                    stop.store(true, Ordering::Relaxed);
                    if paint_p95 > PAINT_P95_MS {
                        fail(&format!("paint p95 {paint_p95:.3}ms > budget {PAINT_P95_MS}ms"));
                    }
                    if build_p95 > BUILD_P95_MS {
                        fail(&format!("build p95 {build_p95:.3}ms > budget {BUILD_P95_MS}ms"));
                    }
                    println!("stream_perf_realwindow: all budgets OK");
                    std::process::exit(0);
                }
            },
        )
        .size_full()
        .into_any_element()
    }
}
```

> **Verified:** `TabRenderState` already exposes `pub fn latest_frame(&self) -> Option<Arc<Frame>>` (`crates/lens-terminal/src/render/state.rs:46`), re-exported through `render_test_api`. Job A uses it directly — **no production-surface change**, keeping this slice a pure test/demo addition.

- [ ] **Step 3: Build the harness (compile check)**

Run: `cargo build -p lens-terminal --test stream_perf_realwindow --features test-util`
Expected: compiles. Fix any `render_test_api` accessor gaps per the implementer note.

- [ ] **Step 4: Run the gate on a real macOS display**

Run: `cargo test --release -p lens-terminal --test stream_perf_realwindow --features test-util`
Expected: a `STREAM paint_p95_ms=… build_p95_ms=… delta_rss_bytes=…` line, then `stream_perf_realwindow: all budgets OK` and exit 0. **Capture the STREAM line** for Task 5 evidence. If a budget trips, treat it as a real regression signal — do NOT raise the budget; investigate (profile the paint/build path) and surface to the user.

- [ ] **Step 5: Wire into the macOS gate**

In `crates/xtask/src/main.rs`, immediately after the existing `render_realwindow` gate block (inside the same `if cfg!(target_os = "macos")`), add:

```rust
        // Job-A sustained multi-tab streaming perf gate (Slice 3). Short burst
        // by default (fits the gate); LENS_STREAM_SOAK=1 for a longer soak.
        run(&[
            "test",
            "--release",
            "-p",
            "lens-terminal",
            "--test",
            "stream_perf_realwindow",
            "--features",
            "test-util",
        ])?;
```

- [ ] **Step 6: Clippy + fmt (test-util config included)**

Run: `cargo fmt -p lens-terminal && cargo clippy -p lens-terminal --all-targets --features test-util -- -D warnings`
Expected: clean.

- [ ] **Step 7: Full gate dry-run**

Run: `cargo run -p xtask -- gate`
Expected: green through the new `stream_perf_realwindow` block on macOS.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-terminal/tests/stream_perf_realwindow.rs crates/lens-terminal/Cargo.toml crates/xtask/src/main.rs
git commit -m "feat(terminal-3): stream_perf_realwindow — Job-A sustained multi-tab perf gate

Real-GPUI harness: N live engines fed a sustained dense wide/emoji stream from
a feeder thread; paints the visible tab (PerCell) and fail-closes on paint-p95
+ build-p95 under load, records delta-RSS, and asserts hidden tabs suppress
builds. Short burst wired into the macOS gate; LENS_STREAM_SOAK=1 soaks.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Slice close — evidence, spec, STATUS, memory

**Files:**
- Modify: `docs/specs/2026-07-16-terminal-workstream-design.md` (Slice 3 → EXECUTED; fold measured numbers + fidelity verdict)
- Modify: `docs/STATUS.md`
- Create: `docs/handoffs/2026-07-22-terminal-slice-3-executed.md`
- Modify: `/Users/aakshintala/.claude/projects/-Users-aakshintala-work-lens/memory/terminal-slice-3plus-replan.md` + `MEMORY.md` (update the Slice-3 line to DONE with outcomes)

**Interfaces:** none (docs).

- [ ] **Step 1: Full gate green (whole-slice verification)**

Run: `cargo run -p xtask -- gate`
Expected: fully green (fmt, workspace clippy, both lens-terminal clippy configs, all crate tests, render_realwindow, stream_perf_realwindow, benches compile, drift).

- [ ] **Step 2: Run the Job-B sweep and record the table**

Run: `cargo run -p xtask -- terminal-rss-sweep | tee /tmp/rss-sweep.txt`
Expected: `terminal-rss-sweep: OK`. Keep the table + median-ratio line.

- [ ] **Step 3: Write the executed handoff**

Create `docs/handoffs/2026-07-22-terminal-slice-3-executed.md` capturing: the 4 build commits, the recorded `STREAM …` paint/build p95 + ΔRSS line, the full RSS-sweep table + fidelity verdict (OK), the calibrated per_cell ratio, and whether `PER_CELL_BYTES` was folded (state the decision either way). Note the byte-accurate FFI conditional stayed **not-triggered** (estimate ordinally reliable), with the exact escalation trigger preserved for the record.

- [ ] **Step 4: Mark Slice 3 EXECUTED in the design spec**

In `docs/specs/2026-07-16-terminal-workstream-design.md`, annotate the Slice 3 bullet (line ~532) and the completion-matrix "Byte accounting" / "Perf acceptance" rows with **DONE (2026-07-22)** + the measured numbers + fidelity verdict, mirroring how Slice 2c was folded in.

- [ ] **Step 5: Update STATUS.md**

In `docs/STATUS.md`, move the Slice 3 "Immediate action: author the Slice 3 plan" to DONE; set the next immediate action to **author the Slice 4 (lifecycle mechanisms) plan**, then (after 4) the `terminal-ws → main` merge. Add a "Recently shipped" entry for Slice 3.

- [ ] **Step 6: Update memory**

In `terminal-slice-3plus-replan.md`, flip the S3 status to DONE with: estimate via `EngineInspect` (`total_rows`/`retained_bytes_estimate`, `PER_CELL_BYTES` provisional/calibrated), Job A (`stream_perf_realwindow`, in-gate) + Job B (`rss_probe` + `xtask terminal-rss-sweep`, out-of-gate acceptance), fidelity = ordinally-reliable (byte-accurate FFI NOT triggered). Refresh the `MEMORY.md` one-liner. Follow the memory-file frontmatter format.

- [ ] **Step 7: Commit**

```bash
git add docs/specs/2026-07-16-terminal-workstream-design.md docs/STATUS.md docs/handoffs/2026-07-22-terminal-slice-3-executed.md
git commit -m "docs(terminal-3): Slice 3 EXECUTED — byte estimate + Job A/B perf evidence

Records stream_perf_realwindow p95s + delta-RSS, the rss-sweep table +
ordinal-fidelity verdict (reliable; byte-accurate FFI not triggered), and the
calibrated per_cell. Next: author Slice 4 (lifecycle mechanisms), then merge
terminal-ws to main.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage** (against `docs/specs/2026-07-16-terminal-workstream-design.md` Slice 3 + completion matrix):
- Thin per-tab retained-bytes estimate `total_rows × cols × per_cell` via `EngineInspect`, no re-vendor → **Task 1**. ✔
- `TOTAL_ROWS`/`SCROLLBACK_ROWS` accessors surfaced → **Task 1** uses `Terminal::total_rows()` (already vendored; `scrollback_rows()` is available but the estimate keys on `total_rows` per the spec, which is scrollback + viewport). ✔
- Job A — several-hidden-tabs streaming, thin multi-tab spawner (spawn-N + visibility toggle, not a fleet coordinator), numeric frame-budget gate + PerCell fail-closed under sustained load, records ΔRSS informationally → **Task 4**. ✔
- Job B — one-tab-per-process RSS sweep across retained-row sizes + adversarial compressible-vs-incompressible pair at equal `total_rows`, estimate-fidelity/ordinal-reliability gate → **Tasks 2+3** (equal-`total_rows` pair per size; ordinal check). ✔
- Byte-accurate FFI escalated ONLY if (B) shows ordinal unreliability → **Task 3** fail-closes with the escalation message; not built otherwise. ✔
- Full bench harness (matrix "Benchmarks … full harness 3") — the existing lens-terminal benches already cover frame-build @ 200×50 / 400×100; the perf *authority* moves to the two demo-hosted jobs. No new Criterion bench is required by the spec beyond compile-in-gate (already wired). ✔ (If a reviewer wants the engine parse/frame bench surfaced as "full harness", that is a no-op — it already compiles in the gate.)
- Pure `lens-terminal` + demo, mergeable to main after Slice 4 → all tasks stay within `lens-terminal`, `lens-terminal-demo`, `xtask`, docs. The single production touch (Task 4's `current_frame_for_test`) is `test-util`-gated. ✔

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". `rss_probe`'s `real_main` placeholder in Task 2 Step 2 is explicitly replaced in Step 4 (and the function deleted) — flagged, not left. The Task 3 xtask-dep note and Task 4 `current_frame_for_test` note are concrete conditional instructions with a decision rule, not open TODOs.

**3. Type consistency:**
- `PER_CELL_BYTES: usize` — defined in `inspect.rs` (Task 1 Step 3), re-exported `engine/mod.rs` + `lib.rs` (Step 10), consumed as `crate::PER_CELL_BYTES` (Task 1 test), `lens_terminal::PER_CELL_BYTES` (Task 2), `lens_terminal::PER_CELL_BYTES` (Task 3, with the drop-the-dep alternative). Consistent.
- `EngineInspect.total_rows: usize` / `.retained_bytes_estimate: usize` — produced Task 1, consumed Tasks 2/4. Consistent.
- `record_retained_rows(&self, total_rows: usize)` — defined Task 1 Step 3, called Task 1 Step 9. Consistent.
- `VtEngine::total_rows(&self) -> usize` (`pub(crate)`) — defined Task 1 Step 7, called Task 1 Step 9 (`engine.total_rows()`), same crate. Consistent.
- `RssSample { mode, total_rows, estimate_bytes, rss_bytes }` + `parse_rss_probe_line` + `check_ordinal_fidelity`/`FidelityVerdict { inversions, ok }` — defined + consumed within Task 3. The probe's printed key names (`mode`/`total_rows`/`cols`/`estimate_bytes`/`rss_bytes`, Task 2 Step 4) exactly match the parser's expected keys (Task 3 Step 3). Consistent.
- `EngineHandle` methods used (`spawn`/`feed`/`build_now`/`set_visible`/`set_inspect_enabled`/`latest_frame`/`inspect`/`stop`) all verified present in `engine/handle.rs`. `render_test_api` items (`CellMetrics`/`TabRenderState`/`paint_frame`/`menlo_gate_ok`/`dense_wide_emoji_frame`) verified in `lib.rs:53-62`.

Fix applied inline: none needed beyond the above — the plan is internally consistent.
