//! Out-of-gate latency/RAM sweep for disk-windowing focused transcript.
//! Windowless `Application::new().run()` harness; controller runs sandbox-disabled.

use std::path::Path;
use std::process;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Application, Entity};
use lens_core::domain::ids::{CallId, ConnectionId, ItemId, ResponseId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::{SqliteTranscriptStore, TranscriptStore};
use lens_ui::fleet::store::{ReaderFactory, ReconcileEpoch};
use lens_ui::focused::FocusedTranscript;

const LARGE_EVERY: i64 = 20;
const LARGE_PAYLOAD_BYTES: usize = 8_192;
const SMALL_PAYLOAD_BYTES: usize = 200;
const RESPONSE_IDS: [&str; 4] = ["resp_a", "resp_b", "resp_c", "resp_d"];
/// Must match private `RESIDENT_CAP_BYTES` in `focused/mod.rs` (24 MiB).
const RESIDENT_CAP_BYTES: usize = 24 * 1024 * 1024;
const BASELINE_TIMEOUT: Duration = Duration::from_secs(120);
const BACKWARD_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_SEED: u64 = 0x6A09_E667_F3BC_C908;

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn payload_bytes(i: i64, rng: &mut DeterministicRng) -> usize {
    if i % LARGE_EVERY == 0 {
        LARGE_PAYLOAD_BYTES
    } else {
        SMALL_PAYLOAD_BYTES + (rng.next_u64() as usize % 64)
    }
}

fn make_item(i: i64, rng: &mut DeterministicRng) -> Item {
    let nbytes = payload_bytes(i, rng);
    let response_id = RESPONSE_IDS[(i as usize) % RESPONSE_IDS.len()];
    if i % LARGE_EVERY == 0 {
        let text = "D".repeat(nbytes);
        Item {
            id: ItemId::new(format!("item_{i:08}")),
            seq: Some(i as u64),
            ctx: BlockContext {
                agent: Some("coder".into()),
                depth: 0,
                response_id: Some(ResponseId::new(response_id)),
            },
            created_at: 1_700_000_000_000 + i,
            kind: ItemKind::FunctionCallOutput {
                call_id: CallId::new(format!("call_{i:08}")),
                output: text,
                arguments: serde_json::Value::Null,
            },
        }
    } else {
        let text = format!("m{:0width$}", i, width = nbytes.saturating_sub(8));
        Item {
            id: ItemId::new(format!("item_{i:08}")),
            seq: Some(i as u64),
            ctx: BlockContext {
                agent: Some("coder".into()),
                depth: 0,
                response_id: Some(ResponseId::new(response_id)),
            },
            created_at: 1_700_000_000_000 + i,
            kind: ItemKind::Message {
                role: if i % 7 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }
}

fn seed_db(data_dir: &Path, count: usize) -> Result<(), String> {
    let conn_id = ConnectionId::new("sweep");
    let session_id = SessionId::new("sweep");
    let db_path = data_dir.join(format!("{}.db", session_id));
    let store = SqliteTranscriptStore::open(&db_path, &conn_id, &session_id)
        .map_err(|e| format!("open transcript store {}: {e}", db_path.display()))?;
    let mut rng = DeterministicRng::new(DEFAULT_SEED);
    for ordinal in 0..count {
        let item = make_item(ordinal as i64, &mut rng);
        store
            .upsert_item(ordinal as i64, &item, false)
            .map_err(|e| format!("upsert item at ordinal {ordinal}: {e}"))?;
    }
    Ok(())
}

fn fail(msg: &str) -> ! {
    eprintln!("FOCUSED_SWEEP FAIL: {msg}");
    process::exit(1);
}

async fn drive_sweep(
    replica: Entity<FocusedTranscript>,
    count: usize,
    cold_start: Instant,
    cx: &mut gpui::AsyncApp,
) -> ! {
    let baseline_deadline = cold_start + BASELINE_TIMEOUT;
    while Instant::now() < baseline_deadline {
        let landed = cx
            .update(|app| replica.read(app).rows().order().len() > 0)
            .unwrap_or(false);
        if landed {
            break;
        }
        cx.background_executor().timer(POLL_INTERVAL).await;
    }
    if !cx
        .update(|app| replica.read(app).rows().order().len() > 0)
        .unwrap_or(false)
    {
        fail("baseline Tail never landed within timeout");
    }
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.0;

    let backward_ms = {
        let lo_before = cx
            .update(|app| replica.read(app).resident_lo())
            .unwrap_or(-1);
        if lo_before <= 0 {
            0.0
        } else {
            let t0 = Instant::now();
            let _ = replica.update(cx, |r, cx| {
                r.page_older_if_near_top(0, cx);
            });
            let deadline = t0 + BACKWARD_TIMEOUT;
            let mut landed = false;
            while Instant::now() < deadline {
                let lo_now = cx
                    .update(|app| replica.read(app).resident_lo())
                    .unwrap_or(lo_before);
                if lo_now < lo_before {
                    landed = true;
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
            if !landed {
                fail(&format!(
                    "backward page never landed (resident_lo stayed at {lo_before})"
                ));
            }
            t0.elapsed().as_secs_f64() * 1000.0
        }
    };

    let resident_bytes = cx
        .update(|app| replica.read(app).resident_bytes())
        .unwrap_or(usize::MAX);

    if resident_bytes > RESIDENT_CAP_BYTES {
        fail(&format!(
            "resident_bytes {resident_bytes} exceeds cap {RESIDENT_CAP_BYTES}"
        ));
    }

    println!(
        "FOCUSED_SWEEP count={count} cold_ms={cold_ms:.3} backward_ms={backward_ms:.3} resident_bytes={resident_bytes} cap={RESIDENT_CAP_BYTES}"
    );
    process::exit(0);
}

fn main() {
    let count: usize = match std::env::args().nth(1) {
        Some(s) => s.parse().unwrap_or_else(|_| fail("count must be a usize")),
        None => fail("usage: focused_sweep_probe <count>"),
    };

    let tempdir = tempfile::tempdir().unwrap_or_else(|e| fail(&format!("tempdir: {e}")));
    seed_db(tempdir.path(), count).unwrap_or_else(|e| fail(&e));

    let conn_id = ConnectionId::new("sweep");
    let session_id = SessionId::new("sweep");
    let factory = ReaderFactory::new(tempdir.path().to_path_buf(), conn_id, session_id);

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        lens_ui::theme::install_at_startup(cx);

        let cold_start = Instant::now();
        let replica =
            cx.new(|cx| FocusedTranscript::new(factory, ReconcileEpoch::default(), 1, cx));

        let replica_for_spawn = replica.clone();
        cx.spawn(async move |cx| {
            drive_sweep(replica_for_spawn, count, cold_start, cx).await;
        })
        .detach();
    });

    fail("Application::run returned without emitting FOCUSED_SWEEP");
}
