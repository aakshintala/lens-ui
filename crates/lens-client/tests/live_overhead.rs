//! Live overhead report — informational round-trip and parse-vs-I/O ratio.
//! Requires $LENS_OMNIGENT_URL and $LENS_OMNIGENT_SESSION_ID (idle, runner-backed).
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_overhead -- --nocapture
#![cfg(feature = "live-tests")]

use std::io::Read;
use std::time::{Duration, Instant};

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::GetOpts;
use lens_client::stream::bench_api::{SseFrame, SseParser, parse_event};
use lens_client::stream::{ResponseEvent, ServerStreamEvent};
use lens_client::{Auth, Connection, SessionEventInput};

const REST_SAMPLES: usize = 10;
const CHUNK_SIZE: usize = 8192;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    assert!(!sorted.is_empty());
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn report_percentiles(label: &str, samples: &mut [Duration]) {
    samples.sort();
    let p50 = percentile(samples, 0.50);
    let p90 = percentile(samples, 0.90);
    eprintln!("{label}: p50={p50:?} p90={p90:?} (n={})", samples.len());
}

fn open_raw_stream(conn: &Connection, sid: &SessionId) -> reqwest::blocking::Response {
    let http = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("http client");
    let url = conn
        .url(&format!("/v1/sessions/{sid}/stream"))
        .expect("stream url");
    let resp = conn
        .auth
        .apply(http.get(url))
        .send()
        .expect("open raw stream");
    resp.error_for_status().expect("stream status")
}

#[test]
fn live_overhead_report() {
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    let conn = Connection::new(ConnectionId::new("live"), base, Auth::None);
    let client = lens_client::Client::new(conn.clone()).unwrap();
    let sessions = client.sessions();

    eprintln!("=== lens-client live overhead ===");

    // REST round-trip: send_event and session snapshot read.
    let mut send_samples = Vec::with_capacity(REST_SAMPLES);
    let mut get_samples = Vec::with_capacity(REST_SAMPLES);
    let ping = SessionEventInput::Message {
        content: vec![serde_json::json!({"type":"input_text","text":"ping"})],
        model_override: None,
        tools: None,
    };
    for _ in 0..REST_SAMPLES {
        let t0 = Instant::now();
        sessions.send_event(&sid, &ping).expect("send_event");
        send_samples.push(t0.elapsed());
    }
    for _ in 0..REST_SAMPLES {
        let t0 = Instant::now();
        sessions
            .get(&sid, GetOpts::default())
            .expect("get snapshot");
        get_samples.push(t0.elapsed());
    }
    report_percentiles("REST send_event", &mut send_samples);
    report_percentiles("REST session get", &mut get_samples);

    // Time-to-first-event: subscribe → post → first typed event on the raw tail.
    let mut raw = open_raw_stream(&conn, &sid);
    sessions
        .send_event(
            &sid,
            &SessionEventInput::Message {
                content: vec![serde_json::json!({
                    "type": "input_text",
                    "text": "Say hello in one word."
                })],
                model_override: None,
                tools: None,
            },
        )
        .expect("post message");
    // Clock starts AFTER the post is acked, so this isolates stream-side latency
    // (the REST POST round-trip is measured separately in `send_event` above).
    let stream_t0 = Instant::now();

    let mut parser = SseParser::default();
    let mut buf = [0u8; CHUNK_SIZE];
    let mut frames: Vec<SseFrame> = Vec::new();
    let mut frame_arrivals: Vec<Duration> = Vec::new();
    let mut last_arrival: Option<Instant> = None;
    let mut first_event_at: Option<Duration> = None;
    let mut saw_completed = false;
    let deadline = Instant::now() + Duration::from_secs(120);

    while Instant::now() < deadline {
        match raw.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    // Timestamp per emitted frame: frames delivered in one read share
                    // a near-zero gap, so the gap series reflects real frame cadence.
                    let now = Instant::now();
                    if first_event_at.is_none() {
                        first_event_at = Some(now.duration_since(stream_t0));
                    }
                    if let Some(prev) = last_arrival.replace(now) {
                        frame_arrivals.push(now.duration_since(prev));
                    }
                    if matches!(
                        parse_event(&frame),
                        ServerStreamEvent::Response(ResponseEvent::Completed)
                    ) {
                        saw_completed = true;
                    }
                    frames.push(frame);
                }
                if saw_completed {
                    break;
                }
            }
            Err(e) => panic!("raw stream read: {e}"),
        }
    }
    for frame in parser.finish() {
        frames.push(frame);
    }

    eprintln!(
        "time-to-first-event (send ack → first frame): {:?}",
        first_event_at.unwrap_or(Duration::MAX)
    );

    // Inter-event gap vs. parse cost on the same captured frames.
    let mut parse_samples = Vec::with_capacity(frames.len());
    for frame in &frames {
        let t0 = Instant::now();
        std::hint::black_box(parse_event(frame));
        parse_samples.push(t0.elapsed());
    }
    parse_samples.sort();
    let parse_p50 = percentile(&parse_samples, 0.50);
    let mut gaps = frame_arrivals;
    gaps.sort();
    let gap_p50 = if gaps.is_empty() {
        Duration::ZERO
    } else {
        percentile(&gaps, 0.50)
    };
    let ratio = if parse_p50.is_zero() {
        f64::INFINITY
    } else {
        gap_p50.as_secs_f64() / parse_p50.as_secs_f64()
    };
    eprintln!(
        "inter-frame gap p50={gap_p50:?} parse_event p50={parse_p50:?} ratio(gap/parse)={ratio:.1}"
    );
    eprintln!("captured {} frames over one turn", frames.len());

    assert!(saw_completed, "turn did not reach response.completed");
    assert!(!frames.is_empty(), "no frames captured");
}
