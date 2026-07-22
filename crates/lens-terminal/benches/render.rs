//! Criterion **Frame-construction** benches (Slice 1c). No paint, no `Window` —
//! the fail-closed paint p95 gate lives in the real-window harness
//! (`tests/render_realwindow.rs`), which the perf assertion needs a real text
//! system for. These benches only measure building the synthetic frames.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lens_terminal::render_bench_api::{ascii_frame, dense_wide_emoji_frame};

fn bench_frame_build(c: &mut Criterion) {
    c.bench_function("render_ascii_frame_200x50", |b| {
        b.iter(|| black_box(ascii_frame(200, 50, 'a')));
    });
    c.bench_function("render_dense_wide_emoji_frame_200x50", |b| {
        b.iter(|| black_box(dense_wide_emoji_frame(200, 50)));
    });
    c.bench_function("render_dense_wide_emoji_frame_400x100", |b| {
        b.iter(|| black_box(dense_wide_emoji_frame(400, 100)));
    });
}

criterion_group!(benches, bench_frame_build);
criterion_main!(benches);
