//! Offline replay tests — drive `VtEngine` from Spike-B capture bytes.

use lens_terminal::{EngineConfig, Frame, VtEngine};

const ATTACH_CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-07-15-pty-attach/attach.frames.jsonl"
);
const RESIZE_CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-07-15-pty-attach/resize.frames.jsonl"
);

fn default_config() -> EngineConfig {
    EngineConfig {
        cols: 80,
        rows: 24,
        max_scrollback: 1000,
        cell_w_px: 8,
        cell_h_px: 16,
    }
}

/// Concatenate inbound binary frame hex from a Spike-B `.frames.jsonl` capture.
fn load_replay_bytes(path: &str) -> Vec<u8> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read replay capture {path}: {e}"));
    let mut bytes = Vec::new();
    for line in text.lines() {
        if !line.contains(r#""direction":"in""#) || !line.contains(r#""kind":"binary""#) {
            continue;
        }
        let Some(i) = line.find(r#""hex":""#) else {
            continue;
        };
        let rest = &line[i + r#""hex":""#.len()..];
        let Some(j) = rest.find('"') else {
            continue;
        };
        let hex = &rest[..j];
        for pair in hex.as_bytes().chunks_exact(2) {
            let s = std::str::from_utf8(pair).unwrap_or("00");
            if let Ok(b) = u8::from_str_radix(s, 16) {
                bytes.push(b);
            }
        }
    }
    bytes
}

/// Load each inbound binary chunk as a separate byte vec (preserves frame order).
fn load_replay_chunks(path: &str) -> Vec<Vec<u8>> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read replay capture {path}: {e}"));
    let mut chunks = Vec::new();
    for line in text.lines() {
        if !line.contains(r#""direction":"in""#) || !line.contains(r#""kind":"binary""#) {
            continue;
        }
        let Some(i) = line.find(r#""hex":""#) else {
            continue;
        };
        let rest = &line[i + r#""hex":""#.len()..];
        let Some(j) = rest.find('"') else {
            continue;
        };
        let hex = &rest[..j];
        let mut bytes = Vec::new();
        for pair in hex.as_bytes().chunks_exact(2) {
            let s = std::str::from_utf8(pair).unwrap_or("00");
            if let Ok(b) = u8::from_str_radix(s, 16) {
                bytes.push(b);
            }
        }
        chunks.push(bytes);
    }
    chunks
}

fn assert_frame_invariants(f: &Frame) {
    assert_eq!(
        f.grid.len(),
        f.rows as usize,
        "grid row count must match declared rows"
    );
    for row in &f.grid {
        let mut prev: Option<u16> = None;
        for cell in &row.cells {
            assert!(cell.col < f.cols, "cell col must be within grid width");
            if let Some(p) = prev {
                assert!(p < cell.col, "cell cols must increase monotonically");
            }
            prev = Some(cell.col);
        }
    }
    let has_content = f.grid.iter().any(|row| {
        row.cells
            .iter()
            .any(|c| c.grapheme != " " && !c.grapheme.is_empty())
    });
    assert!(has_content, "frame must not be all blank");
}

#[test]
fn replay_attach_produces_coherent_frame() {
    let bytes = load_replay_bytes(ATTACH_CAPTURE);
    assert!(!bytes.is_empty(), "capture must yield VT bytes");

    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&default_config(), |_| {}, tx).expect("engine");
    engine.feed(&bytes);
    let frame = engine.build_frame().expect("frame");
    assert_eq!((frame.cols, frame.rows), (80, 24));
    assert_frame_invariants(&frame);
}

#[test]
fn replay_resize_reflows_without_panic() {
    let chunks = load_replay_chunks(RESIZE_CAPTURE);
    assert!(
        chunks.len() >= 2,
        "resize capture must have pre/post chunks"
    );

    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut engine = VtEngine::new(&default_config(), |_| {}, tx).expect("engine");
    engine.feed(&chunks[0]);
    engine
        .resize(120, 40)
        .expect("resize must not error on reflow");
    for chunk in &chunks[1..] {
        engine.feed(chunk);
    }
    let frame = engine.build_frame().expect("frame after resize");
    assert_eq!((frame.cols, frame.rows), (120, 40));
    assert_frame_invariants(&frame);
}
