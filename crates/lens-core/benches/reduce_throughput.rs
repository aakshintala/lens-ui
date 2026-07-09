//! Reducer throughput over the golden corpus (AGENTS.md benchmark-or-it's-not-done).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lens_client::stream::decode_all;
use lens_core::{AgentId, ConnectionId, ManualClock, SessionId, SessionState, reduce};

const HAPPY_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse"
);

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
        b.iter(|| {
            let mut s = fresh_state();
            for ev in &events {
                black_box(reduce(&mut s, ev, &clock));
            }
        });
    });
}

criterion_group!(benches, bench_full_replay);
criterion_main!(benches);
