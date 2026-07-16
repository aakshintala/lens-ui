//! Terminal transport throughput — frame codec + bounded queue drain.
//! Run: `cargo bench -p lens-client --features bench --bench terminal_transport`

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use crossbeam_channel::bounded;
use lens_client::terminal::bench_api::{
    WireInbound, WsInbound, WsOutbound, classify_inbound, encode_outbound,
};

const VT_FRAME: &[u8] = b"\x1b[0mhello terminal world";
const CHANNEL_CAP: usize = 256;
const QUEUE_OPS: usize = 10_000;

fn bench_classify_inbound_vt(c: &mut Criterion) {
    c.bench_function("classify_inbound_vt", |b| {
        b.iter_batched(
            || WireInbound::Binary(VT_FRAME.to_vec()),
            classify_inbound,
            BatchSize::SmallInput,
        );
    });
}

fn bench_encode_outbound(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_outbound");
    group.bench_with_input(BenchmarkId::from_parameter("input"), &(), |b, _| {
        let payload = vec![0x1b, b'[', b'A'];
        b.iter(|| encode_outbound(&WsOutbound::Input(payload.clone())));
    });
    group.bench_with_input(BenchmarkId::from_parameter("resize"), &(), |b, _| {
        b.iter(|| {
            encode_outbound(&WsOutbound::Resize {
                cols: 120,
                rows: 40,
            })
        });
    });
    group.finish();
}

fn bench_bounded_queue_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_queue_vt");
    group.throughput(Throughput::Elements(QUEUE_OPS as u64));
    group.bench_function("push_drain", |b| {
        b.iter(|| {
            let (tx, rx) = bounded(CHANNEL_CAP);
            let mut drained = Vec::with_capacity(QUEUE_OPS);
            let mut i = 0usize;
            while i < QUEUE_OPS {
                for _ in 0..CHANNEL_CAP.min(QUEUE_OPS - i) {
                    let payload = vec![(i % 256) as u8; 64];
                    tx.send(WsInbound::Vt(payload)).unwrap();
                    i += 1;
                }
                while let Ok(msg) = rx.try_recv() {
                    drained.push(msg);
                }
            }
            while let Ok(msg) = rx.try_recv() {
                drained.push(msg);
            }
            drained
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_classify_inbound_vt,
    bench_encode_outbound,
    bench_bounded_queue_throughput
);
criterion_main!(benches);
