use anyhow::{Context, Result, bail};
use lens_core::domain::ids::{ConnectionId, ItemId, ResponseId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::{SqliteTranscriptStore, TranscriptStore};
use std::path::PathBuf;

const LARGE_EVERY: i64 = 20;
const LARGE_PAYLOAD_BYTES: usize = 8_192;
const SMALL_PAYLOAD_BYTES: usize = 200;
const RESPONSE_IDS: [&str; 4] = ["resp_a", "resp_b", "resp_c", "resp_d"];
const ALLOWED_COUNTS: [u64; 3] = [1_000, 10_000, 50_000];

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
                call_id: lens_core::domain::ids::CallId::new(format!("call_{i:08}")),
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

fn item_wire_bytes(item: &Item) -> usize {
    serde_json::to_vec(item).map(|v| v.len()).unwrap_or(0)
}

struct FocusedSeedArgs {
    out: PathBuf,
    session: String,
    count: u64,
    seed: u64,
}

fn parse_args(args: &[String]) -> Result<FocusedSeedArgs> {
    let mut out = None;
    let mut session = None;
    let mut count = None;
    let mut seed = 0x6A09_E667_F3BC_C908_u64;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(args.get(i).context("--out requires a path")?.clone().into());
            }
            "--session" => {
                i += 1;
                session = Some(args.get(i).context("--session requires an id")?.clone());
            }
            "--count" => {
                i += 1;
                count = Some(
                    args.get(i)
                        .context("--count requires a number")?
                        .parse()
                        .context("--count must be a positive integer")?,
                );
            }
            "--seed" => {
                i += 1;
                seed = args
                    .get(i)
                    .context("--seed requires a u64")?
                    .parse()
                    .context("--seed must be a u64")?;
            }
            other => bail!("unknown focused-seed argument: {other:?}"),
        }
        i += 1;
    }
    let out = out.context("--out is required")?;
    let session = session.context("--session is required")?;
    let count = count.context("--count is required")?;
    if !ALLOWED_COUNTS.contains(&count) {
        bail!("--count must be one of {:?}, got {count}", ALLOWED_COUNTS);
    }
    Ok(FocusedSeedArgs {
        out,
        session,
        count,
        seed,
    })
}

pub fn focused_seed(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("create data dir {}", args.out.display()))?;

    let db_path = args.out.join(format!("{}.db", args.session));
    if db_path.exists() {
        std::fs::remove_file(&db_path)
            .with_context(|| format!("remove existing {}", db_path.display()))?;
    }

    let conn_id = ConnectionId::new("focused_seed");
    let session_id = SessionId::new(&args.session);
    let store = SqliteTranscriptStore::open(&db_path, &conn_id, &session_id)
        .with_context(|| format!("open transcript store {}", db_path.display()))?;

    let mut rng = DeterministicRng::new(args.seed);
    let mut total_bytes = 0usize;
    for ordinal in 0..args.count {
        let item = make_item(ordinal as i64, &mut rng);
        total_bytes += item_wire_bytes(&item);
        store
            .upsert_item(ordinal as i64, &item, false)
            .with_context(|| format!("upsert item at ordinal {ordinal}"))?;
    }

    println!(
        "focused-seed: wrote {} items ({total_bytes} B payload-json) to {}",
        args.count,
        db_path.display()
    );
    Ok(())
}
