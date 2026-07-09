//! Reducer throughput (AGENTS.md benchmark-or-it's-not-done).
//!
//! Two variants:
//!   1. `reduce_happy_path_full_replay` — real corpus shape, small n. The
//!      representative-shape tripwire.
//!   2. `reduce_window_scale/build_{375,750,1500}_item_tail` — `push_item`
//!      (items.rs) does a linear dedup scan over ALL items on every append, so
//!      building an N-item tail is O(N²). happy_path keeps n tiny and HIDES this;
//!      at D11 window scale (~1500 live items) that per-append scan bounds the
//!      reconcile tail. This variant sweeps THREE sizes (each a 2× step) so the
//!      time RATIO between them exposes the growth rate: O(N²) ⇒ ~4× per
//!      doubling, O(N) ⇒ ~2×. A single size could only catch a constant-factor
//!      regression; the sweep catches a complexity regression too.

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use lens_client::stream::decode_all;
use lens_core::reduce::bench_push_message;
use lens_core::{AgentId, ConnectionId, ItemId, ManualClock, SessionId, SessionState, reduce};

const HAPPY_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse"
);

/// D11 live-window sizes — a 2× sweep up to the ~1500-item tail the reconcile
/// path must stay bounded against (large-transcript-latency spike). Three sizes
/// so the inter-size time ratio, not just the absolute number, is the signal.
const WINDOWS: [u64; 3] = [375, 750, 1500];

fn fresh_state() -> SessionState {
    SessionState::new(
        ConnectionId::new("conn_1"),
        SessionId::new("conv_1"),
        AgentId::new("ag"),
    )
}

fn bench_full_replay(c: &mut Criterion) {
    let bytes = std::fs::read(HAPPY_PATH).expect("happy_path corpus");
    let events = decode_all(&bytes);
    let clock = ManualClock::new(1_700_000_000_000);
    c.bench_function("reduce_happy_path_full_replay", |b| {
        // fresh_state() in the setup closure keeps allocation out of the timed body.
        b.iter_batched(
            fresh_state,
            |mut s| {
                for ev in &events {
                    black_box(reduce(&mut s, ev, &clock));
                }
                s // return so the built state's Drop (dealloc) is untimed
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_window_scale(c: &mut Criterion) {
    let clock = ManualClock::new(1_700_000_000_000);
    let mut g = c.benchmark_group("reduce_window_scale");
    for window in WINDOWS {
        // IDs built once per size, up front, so id *formatting* stays out of the
        // timed body. The measured cost is the real per-append path at window
        // scale — the O(n) dedup scan (String eq × n) plus item construction,
        // exactly what `reduce` does per item. The scan dominates and grows with
        // the tail; construction is a per-append constant. Across sizes, the O(N²)
        // total means each 2× step should take ~4× as long.
        let ids: Vec<ItemId> = (0..window)
            .map(|i| ItemId::new(format!("item_{i}")))
            .collect();
        g.throughput(Throughput::Elements(window));
        g.bench_with_input(
            BenchmarkId::new("build_item_tail", window),
            &ids,
            |b, ids| {
                b.iter_batched(
                    fresh_state,
                    |mut s| {
                        for id in ids {
                            black_box(bench_push_message(&mut s, id.clone(), &clock));
                        }
                        s // return so the built state's Drop is untimed
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    g.finish();
}

criterion_group!(benches, bench_full_replay, bench_window_scale);
criterion_main!(benches);
