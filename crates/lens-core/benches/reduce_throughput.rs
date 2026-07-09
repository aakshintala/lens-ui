//! Reducer throughput (AGENTS.md benchmark-or-it's-not-done).
//!
//! Two variants:
//!   1. `reduce_happy_path_full_replay` — real corpus shape, small n. The
//!      representative-shape tripwire.
//!   2. `reduce_window_scale/build_1500_item_tail` — `push_item` (items.rs) does
//!      a linear dedup scan over ALL items on every append, so building an
//!      N-item tail is O(N²). happy_path keeps n tiny and HIDES this; at D11
//!      window scale (~1500 live items) that per-append scan bounds the
//!      reconcile tail, so this variant makes the cost visible.

use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use lens_client::stream::decode_all;
use lens_core::reduce::bench_push_message;
use lens_core::{AgentId, ConnectionId, ItemId, ManualClock, SessionId, SessionState, reduce};

const HAPPY_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse"
);

/// D11 live-window size — the ~1500-item tail the reconcile path must stay
/// bounded against (large-transcript-latency spike).
const WINDOW: u64 = 1500;

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
    // IDs built once, up front, so id *formatting* stays out of the timed body.
    // The measured cost is the real per-append path at window scale — the O(n)
    // dedup scan (String eq × n) plus item construction, exactly what `reduce`
    // does per item. The scan is the term that dominates and regresses as the
    // tail grows; construction is a per-append constant.
    let ids: Vec<ItemId> = (0..WINDOW)
        .map(|i| ItemId::new(format!("item_{i}")))
        .collect();

    let mut g = c.benchmark_group("reduce_window_scale");
    g.throughput(Throughput::Elements(WINDOW));
    g.bench_function("build_1500_item_tail", |b| {
        b.iter_batched(
            fresh_state,
            |mut s| {
                for id in &ids {
                    black_box(bench_push_message(&mut s, id.clone(), &clock));
                }
                s // return so the 1500-item state's Drop is untimed
            },
            BatchSize::SmallInput,
        );
    });
    g.finish();
}

criterion_group!(benches, bench_full_replay, bench_window_scale);
criterion_main!(benches);
