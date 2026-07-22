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

#[cfg_attr(not(test), allow(dead_code))]
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

    // `max_scrollback` is a BYTE budget (not a line count — the vendored doc
    // comment is misleading; verified empirically). To let `total_rows` actually
    // reach `target_rows` for BOTH content modes (incompressible compresses worse
    // → costs more bytes/row), size the budget generously so the row count, not
    // the byte cap, is the binding constraint: `target × cols × 16 + 4 MiB`
    // headroom. This is the point of Job B — the estimate must stay ordinally
    // reliable even though retention is byte-budgeted and content-dependent.
    let viewport_rows: u16 = 50;
    let max_scrollback = target_rows
        .saturating_mul(cols as usize)
        .saturating_mul(16)
        .saturating_add(4_000_000);
    let cfg = EngineConfig {
        cols,
        rows: viewport_rows,
        max_scrollback,
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
    // `total_rows` is sampled only on a fresh frame build, and `build_now` is a
    // no-op when the engine is not dirty (the last feed's frame was already
    // built and cleared the dirty flag). Feed one final CRLF to re-dirty, then
    // force a build so the sample reflects the fully-grown scrollback.
    loop {
        match handle.feed(b"\r\n".to_vec()) {
            Ok(()) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    let _ = handle.build_now();
    // Give the build a beat to land the sample.
    std::thread::sleep(Duration::from_millis(80));

    let snap = handle.inspect();
    let rss = rss_bytes();
    // Read RSS BEFORE stopping the engine (stop drops the worker + terminal).
    println!(
        "RSS_PROBE mode={mode} target_rows={target_rows} total_rows={} cols={} estimate_bytes={} rss_bytes={}",
        snap.total_rows, snap.cols, snap.retained_bytes_estimate, rss,
    );

    handle.stop();
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
