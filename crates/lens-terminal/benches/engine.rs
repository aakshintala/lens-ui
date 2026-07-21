//! Engine throughput benches — VT parse, frame build, scroll, reflow, input path.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use lens_terminal::engine_bench_api::{
    encode_arrow_up_press, encode_mouse_move_bench, encode_paste_bench,
    feed_app_cursor_mode_then_arrow_up,
};
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

const TITLE_FEED_ROUNDS: usize = 64;
const PRESENTATION_CHANNEL_CAP: usize = 64;

fn bench_presentation_title_callback_throughput(c: &mut Criterion) {
    let title_bytes = b"\x1b]2;BenchTitle\x1b\\";
    c.bench_function("presentation_title_callback_throughput", |b| {
        b.iter_batched(
            || {
                let (tx, _rx) = crossbeam_channel::bounded(PRESENTATION_CHANNEL_CAP);
                VtEngine::new(&bench_config(), |_| {}, tx).expect("engine")
            },
            |mut engine| {
                for _ in 0..TITLE_FEED_ROUNDS {
                    engine.feed(black_box(title_bytes));
                }
                engine
            },
            BatchSize::SmallInput,
        );
    });
}

fn dense_hyperlink_seed_bytes(cols: u16, rows: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(usize::from(cols) * usize::from(rows) + 48);
    bytes.extend_from_slice(b"\x1b[2J\x1b[H\x1b]8;;https://example.com/bench\x1b\\");
    for _ in 0..rows {
        for _ in 0..cols {
            bytes.push(b'X');
        }
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"\x1b]8;;\x1b\\");
    bytes
}

fn seeded_dense_hyperlink_engine() -> VtEngine {
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
    engine.feed(&dense_hyperlink_seed_bytes(BENCH_COLS, BENCH_ROWS));
    engine
}

fn bench_dense_hyperlink_frame_build(c: &mut Criterion) {
    c.bench_function("engine_frame_build_dense_hyperlink_200x50", |b| {
        b.iter_batched(
            seeded_dense_hyperlink_engine,
            |mut engine| black_box(engine.build_frame().expect("frame")),
            BatchSize::SmallInput,
        );
    });
}

const OSC52_FEED_ROUNDS: usize = 64;
const OSC52_WRITE_PAYLOAD: &[u8] = b"bench-osc52-payload";

fn osc52_write_bytes(decoded: &[u8]) -> Vec<u8> {
    let mut v = Vec::from(b"\x1b]52;c;");
    v.extend_from_slice(STANDARD.encode(decoded).as_bytes());
    v.push(0x07);
    v
}

fn bench_osc52_callback_throughput(c: &mut Criterion) {
    let osc52 = osc52_write_bytes(OSC52_WRITE_PAYLOAD);
    c.bench_function("osc52_callback_throughput", |b| {
        b.iter_batched(
            || {
                let (tx, rx) = crossbeam_channel::bounded(PRESENTATION_CHANNEL_CAP);
                let engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
                (engine, rx)
            },
            |(mut engine, rx)| {
                for _ in 0..OSC52_FEED_ROUNDS {
                    engine.feed(black_box(&osc52));
                    while rx.try_recv().is_ok() {}
                }
                engine
            },
            BatchSize::SmallInput,
        );
    });
}

const PASTE_ENCODE_PAYLOAD: &[u8] = b"hello paste bench payload\nline-two";

fn bench_paste_encode_throughput(c: &mut Criterion) {
    c.bench_function("paste_encode_throughput", |b| {
        b.iter_batched(
            || {
                let (tx, _rx) = crossbeam_channel::bounded(1);
                let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
                engine.feed(b"\x1b[?2004h");
                engine
            },
            |mut engine| {
                black_box(encode_paste_bench(&mut engine, PASTE_ENCODE_PAYLOAD).expect("encode"))
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_mouse_encode_throughput(c: &mut Criterion) {
    // SgrPixels never coalesces -> every call encodes (measures full encode path).
    c.bench_function("mouse_encode_throughput", |b| {
        b.iter_batched(
            || {
                let (tx, _rx) = crossbeam_channel::bounded(1);
                let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
                engine.feed(b"\x1b[?1003h\x1b[?1016h"); // Any + SgrPixels
                engine
            },
            |mut engine| {
                black_box(encode_mouse_move_bench(&mut engine, 16.0, 0.0).expect("encode"))
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_mouse_motion_coalesced(c: &mut Criterion) {
    // Same-cell motion under SGR coalesces after the first -> measures the dedup fast path.
    // Persistent engine across iterations so all but the first call coalesce.
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&bench_config(), |_| {}, tx).expect("engine");
    engine.feed(b"\x1b[?1003h\x1b[?1006h"); // Any + SGR
    let _ = encode_mouse_move_bench(&mut engine, 16.0, 0.0); // prime the last-cell
    c.bench_function("mouse_motion_coalesced", |b| {
        b.iter(|| black_box(encode_mouse_move_bench(&mut engine, 16.0, 0.0).expect("encode")));
    });
}

criterion_group!(
    engine,
    bench_vt_parse,
    bench_frame_construction,
    bench_scroll,
    bench_reflow,
    bench_key_encode_arrow_up,
    bench_ordered_stream_feed_then_key_throughput,
    bench_presentation_title_callback_throughput,
    bench_dense_hyperlink_frame_build,
    bench_osc52_callback_throughput,
    bench_paste_encode_throughput,
    bench_mouse_encode_throughput,
    bench_mouse_motion_coalesced
);
criterion_main!(engine);
