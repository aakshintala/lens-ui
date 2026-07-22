//! Full-snapshot GPUI render for `lens-terminal` (Slice 1c).
//!
//! Paints an immutable [`crate::Frame`] into a GPUI window via full-snapshot
//! quads + glyphs. No `libghostty_vt` type crosses into this module — paint
//! operates only on Lens-owned `Frame`/`FrameRow`/`FrameCell`/`Rgb` values.
//!
//! The module is **private** (`mod render;` in `lib.rs`) with `pub` items, so
//! nothing leaks onto the crate's public API — mirrors the `engine` module.
//! The real-window test harness (`tests/render_realwindow.rs`) reaches these
//! items only through the feature-gated `render_test_api` (`test-util`);
//! Criterion reaches fixtures through `render_bench_api` (`bench`).
//!
//! See `docs/specs/2026-07-16-terminal-workstream-design.md` and
//! `docs/plans/2026-07-16-terminal-slice-1c-lens-terminal-render.md`.

#[cfg(any(test, feature = "test-util", feature = "bench"))]
pub mod fixtures;
pub mod inspect;
pub mod metrics;
pub mod paint;
pub mod state;
