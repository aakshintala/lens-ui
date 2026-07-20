//! Engine throughput benches — VT parse, frame build, scroll, reflow, input path.

use std::sync::Arc;

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use lens_terminal::engine_bench_api::{encode_arrow_up_press, feed_app_cursor_mode_then_arrow_up};
use lens_terminal::{EngineConfig, EngineHandle, VtEngine};

const BENCH_COLS: u16 = 200;
const BENCH_ROWS: u16 = 50;

fn bench_config() -> EngineConfig {
    EngineConfig {
        cols: BENCH_COLS,
        rows: BENCH_ROWS,
        max_scrollback: 1000,
        cell_w_px: 8,
        cell_h_px: 16,
    }
}

fn full_redraw_bytes(cols: u16, rows: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(usize::from(cols) * usize::from(rows) + 16);
    bytes.extend_from_slice(b"\x1b[2J\x1b[H");
    for _ in 0..rows {
        for _ in 0..cols {
            bytes.push(b'X');
        }
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

fn seeded_engine() -> VtEngine {
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
    engine.feed(&full_redraw_bytes(BENCH_COLS, BENCH_ROWS));
    engine
}

fn bench_vt_parse(c: &mut Criterion) {
    let bytes = full_redraw_bytes(BENCH_COLS, BENCH_ROWS);
    c.bench_function("engine_vt_parse_200x50_redraw", |b| {
        b.iter_batched(
            || {
                let (tx, _rx) = crossbeam_channel::bounded(1);
                VtEngine::new(&bench_config(), |_| {}, tx).expect("engine")
            },
            |mut engine| {
                engine.feed(black_box(&bytes));
                engine
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_frame_construction(c: &mut Criterion) {
    c.bench_function("engine_frame_build_200x50", |b| {
        b.iter_batched(
            seeded_engine,
            |mut engine| black_box(engine.build_frame().expect("frame")),
            BatchSize::SmallInput,
        );
    });
}

fn bench_scroll(c: &mut Criterion) {
    let scroll_bytes: Vec<u8> = b"\r\n".repeat(200);
    c.bench_function("engine_scroll_200_newlines", |b| {
        b.iter_batched(
            seeded_engine,
            |mut engine| {
                engine.feed(black_box(&scroll_bytes));
                engine
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_reflow(c: &mut Criterion) {
    c.bench_function("engine_reflow_200x50_120x40_200x50", |b| {
        b.iter_batched(
            seeded_engine,
            |mut engine| {
                engine.resize(120, 40).expect("shrink");
                engine.resize(BENCH_COLS, BENCH_ROWS).expect("restore");
                engine
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_key_encode_arrow_up(c: &mut Criterion) {
    c.bench_function("key_encode_arrow_up", |b| {
        b.iter_batched(
            || {
                let (tx, _rx) = crossbeam_channel::bounded(1);
                let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
                engine.feed(b"\x1b[?1h");
                engine
            },
            |mut engine| black_box(encode_arrow_up_press(&mut engine).expect("encode")),
            BatchSize::SmallInput,
        );
    });
}

fn bench_ordered_stream_feed_then_key_throughput(c: &mut Criterion) {
    c.bench_function("ordered_stream_feed_then_key_throughput", |b| {
        b.iter_batched(
            || Arc::new(EngineHandle::spawn(bench_config())),
            |handle| black_box(feed_app_cursor_mode_then_arrow_up(&handle).expect("throughput")),
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    engine,
    bench_vt_parse,
    bench_frame_construction,
    bench_scroll,
    bench_reflow,
    bench_key_encode_arrow_up,
    bench_ordered_stream_feed_then_key_throughput
);
criterion_main!(engine);
