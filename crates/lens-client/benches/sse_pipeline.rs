//! SSE pipeline throughput — bytes → frames → typed events → normalized.
//! Run: `cargo bench -p lens-client --features bench`

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use lens_client::stream::ServerStreamEvent;
use lens_client::stream::bench_api::{Normalizer, SseFrame, SseParser, parse_event};

const HAPPY_PATH: &[u8] =
    include_bytes!("../../../docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse");
const INTERRUPT: &[u8] =
    include_bytes!("../../../docs/spikes/captures/2026-06-26-sse/interrupt.stream.sse");
const REASONING_EFFORT_HIGH: &[u8] =
    include_bytes!("../../../docs/spikes/captures/2026-06-26-sse/reasoning_effort_high.stream.sse");

const CHUNK_SIZE: usize = 8192;

const CORPORA: &[(&str, &[u8])] = &[
    ("happy_path", HAPPY_PATH),
    ("interrupt", INTERRUPT),
    ("reasoning_effort_high", REASONING_EFFORT_HIGH),
];

fn parse_all_frames(bytes: &[u8]) -> Vec<SseFrame> {
    let mut parser = SseParser::default();
    let mut frames = Vec::new();
    for chunk in bytes.chunks(CHUNK_SIZE) {
        frames.extend(parser.push(chunk));
    }
    frames.extend(parser.finish());
    frames
}

fn decode_all_events(frames: &[SseFrame]) -> Vec<ServerStreamEvent> {
    frames.iter().map(parse_event).collect()
}

fn bench_sse_frame_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("sse_frame_parse");
    for (name, bytes) in CORPORA {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), bytes, |b, input| {
            b.iter(|| {
                let mut parser = SseParser::default();
                let mut frames = Vec::new();
                for chunk in input.chunks(CHUNK_SIZE) {
                    frames.extend(parser.push(chunk));
                }
                frames.extend(parser.finish());
                frames
            });
        });
    }
    group.finish();
}

fn bench_event_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_decode");
    for (name, bytes) in CORPORA {
        let frames = parse_all_frames(bytes);
        let input_len = bytes.len() as u64;
        group.throughput(Throughput::Bytes(input_len));
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &frames,
            |b, frame_list| {
                b.iter(|| {
                    for frame in frame_list {
                        std::hint::black_box(parse_event(frame));
                    }
                });
            },
        );
    }
    group.finish();
}

fn bench_normalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize");
    for (name, bytes) in CORPORA {
        let events = decode_all_events(&parse_all_frames(bytes));
        let input_len = bytes.len() as u64;
        group.throughput(Throughput::Bytes(input_len));
        // Clone the owned events in the (untimed) setup closure so the measured
        // body sees the same owned input the real pipeline gets from `parse_event`
        // — without charging clone cost to the normalize number.
        group.bench_with_input(BenchmarkId::from_parameter(name), &events, |b, evs| {
            b.iter_batched(
                || evs.clone(),
                |owned| {
                    let mut normalizer = Normalizer::default();
                    let mut out = Vec::new();
                    for ev in owned {
                        out.extend(normalizer.push(ev));
                    }
                    out.extend(normalizer.flush());
                    out
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    for (name, bytes) in CORPORA {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), bytes, |b, input| {
            b.iter(|| {
                let mut parser = SseParser::default();
                let mut normalizer = Normalizer::default();
                let mut last_seen_seq: Option<u64> = None;
                let mut out = Vec::new();
                // Mirror reader.rs: per-frame seq peek + max-tracking before parse.
                let mut frames: Vec<SseFrame> = Vec::new();
                for chunk in input.chunks(CHUNK_SIZE) {
                    frames.extend(parser.push(chunk));
                }
                frames.extend(parser.finish());
                for frame in &frames {
                    if let Some(s) = frame.sequence_number() {
                        last_seen_seq = Some(last_seen_seq.map_or(s, |p| p.max(s)));
                    }
                    out.extend(normalizer.push(parse_event(frame)));
                }
                out.extend(normalizer.flush());
                std::hint::black_box(last_seen_seq);
                out
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_sse_frame_parse,
    bench_event_decode,
    bench_normalize,
    bench_full_pipeline
);
criterion_main!(benches);
