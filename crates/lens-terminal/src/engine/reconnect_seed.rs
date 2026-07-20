//! Offline retained-engine reconnect-seed acceptance (Slice 1d Task 6).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::frame::Frame;
use super::handle::EngineHandle;
use super::vt::{EngineConfig, VtEngine};

const RECONNECT_CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/spikes/captures/2026-07-15-pty-attach/reconnect.frames.jsonl"
);

const LEG2_CLEAR_REDRAW_PREFIX: &[u8] = b"\x1b[H\x1b[2J";

#[derive(Debug, Deserialize)]
struct CaptureLine {
    ts_offset_ms: u64,
    direction: String,
    kind: String,
    hex: String,
}

fn reconnect_config() -> EngineConfig {
    EngineConfig {
        cols: 80,
        rows: 24,
        max_scrollback: 2000,
        cell_w_px: 8,
        cell_h_px: 16,
    }
}

fn decode_hex(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).expect("capture hex is ASCII");
            u8::from_str_radix(s, 16).expect("valid hex pair")
        })
        .collect()
}

fn load_capture_lines(path: &str) -> Vec<CaptureLine> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read reconnect capture {path}: {e}"));
    text.lines()
        .map(|line| serde_json::from_str(line).expect("valid JSONL capture line"))
        .collect()
}

fn ts_reset_index(lines: &[CaptureLine]) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(i, line)| line.ts_offset_ms < lines[i - 1].ts_offset_ms)
        .map(|(i, _)| i)
        .expect("capture must contain a ts_offset reset (reconnect boundary)")
}

fn concat_inbound_binary(lines: &[CaptureLine]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in lines {
        if line.direction == "in" && line.kind == "binary" {
            bytes.extend_from_slice(&decode_hex(&line.hex));
        }
    }
    bytes
}

struct ReconnectLegs {
    leg1_seed: Vec<u8>,
    leg2_seed: Vec<u8>,
}

fn split_reconnect_legs(lines: &[CaptureLine]) -> ReconnectLegs {
    let reset = ts_reset_index(lines);
    let leg1_seed = concat_inbound_binary(&lines[..reset]);
    let leg2_seed = concat_inbound_binary(&lines[reset..]);
    assert!(
        leg2_seed.starts_with(LEG2_CLEAR_REDRAW_PREFIX),
        "leg2 seed must begin with clear+redraw (\\x1b[H\\x1b[2J)"
    );
    ReconnectLegs {
        leg1_seed,
        leg2_seed,
    }
}

fn wait_new_frame(engine: &EngineHandle, min_frames_built: u64) -> Arc<Frame> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let built = engine.inspect().frames_built;
        if built > min_frames_built
            && let Some(f) = engine.latest_frame()
        {
            return f;
        }
        assert!(Instant::now() < deadline, "no new frame generation");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn stop_engine(engine: Arc<EngineHandle>) {
    if let Ok(h) = Arc::try_unwrap(engine) {
        h.stop();
    }
}

#[test]
fn retained_reconnect_seed_viewport_matches_fresh_engine() {
    let lines = load_capture_lines(RECONNECT_CAPTURE);
    assert_eq!(lines.len(), 10, "reconnect capture has 10 JSONL lines");
    let legs = split_reconnect_legs(&lines);

    let cfg = reconnect_config();
    let retained = Arc::new(EngineHandle::spawn(cfg));
    retained.feed(legs.leg1_seed).expect("feed leg1");
    retained.build_now().expect("build after leg1");
    let gen0 = retained.inspect().frames_built;
    retained.feed(legs.leg2_seed.clone()).expect("feed leg2");
    retained.build_now().expect("build after leg2");
    let retained_frame = wait_new_frame(&retained, gen0);

    let fresh = Arc::new(EngineHandle::spawn(cfg));
    fresh.feed(legs.leg2_seed).expect("feed leg2 only");
    fresh.build_now().expect("build");
    let fresh_frame = wait_new_frame(&fresh, 0);

    assert_eq!(
        *retained_frame, *fresh_frame,
        "retained engine after reconnect seed must match fresh engine fed only leg2"
    );

    stop_engine(retained);
    stop_engine(fresh);
}

#[test]
fn retained_reconnect_seed_does_not_duplicate_scrollback() {
    let lines = load_capture_lines(RECONNECT_CAPTURE);
    let legs = split_reconnect_legs(&lines);
    let cfg = reconnect_config();
    let rows = cfg.rows;

    let mut engine = VtEngine::new(&cfg, |_| {}, {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        tx
    })
    .expect("VtEngine");
    engine.feed(&legs.leg1_seed);
    let sb0 = engine.scrollback_rows_for_test();
    engine.feed(&legs.leg2_seed);
    let sb1 = engine.scrollback_rows_for_test();

    let delta = sb1.saturating_sub(sb0);
    assert!(
        delta <= rows as usize,
        "scrollback grew by {delta} (> viewport); retained seed duplicated history (sb0={sb0}, sb1={sb1})"
    );
}
