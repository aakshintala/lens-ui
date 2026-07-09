//! Persistence throughput (AGENTS.md benchmark-or-it's-not-done). Persistence is
//! I/O-bound; this measures write-through + reload cost over a realistic,
//! bimodal item set so regressions are visible — not to claim a CPU budget.
//!
//! Two deliberate choices:
//!   - The store is opened in the `iter_batched` SETUP closure (fresh DB per
//!     iteration), so the WAL + DDL open cost stays OUT of the timed body. Timing
//!     it inline would let a write regression hide behind ~13ms of open cost.
//!   - Items are BIMODAL (~2.5% × 200KB blobs + the rest ~130B markers), matching
//!     the large-transcript-latency spike corpus where a few dumps dominate the
//!     bytes. Uniform small items would miss large-blob write regressions.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lens_core::domain::ids::{ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::{SqliteTranscriptStore, TranscriptStore};

const N: usize = 200;
/// Every 40th item is a large blob → 5/200 = 2.5% (spike-matched).
const LARGE_EVERY: usize = 40;
const LARGE_BYTES: usize = 200_000;

fn item(i: usize) -> Item {
    let text = if i.is_multiple_of(LARGE_EVERY) {
        "x".repeat(LARGE_BYTES) // ~200KB dump-scale blob
    } else {
        "lorem ipsum dolor sit amet ".repeat(5) // ~135B marker
    };
    Item {
        id: ItemId::new(format!("item_{i}")),
        seq: Some(i as u64),
        ctx: BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        },
        created_at: 1_700_000_000_000,
        kind: ItemKind::Message {
            role: Role::Assistant,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some(text),
                data: serde_json::Value::Null,
            }],
        },
    }
}

fn bench_transcript(c: &mut Criterion) {
    let items: Vec<Item> = (0..N).map(item).collect();
    c.bench_function("transcript_write_200_then_load", |b| {
        b.iter_batched(
            || {
                // Fresh store per iteration; open cost (WAL + DDL) is untimed.
                let dir = tempfile::tempdir().unwrap();
                let store = SqliteTranscriptStore::open(
                    &dir.path().join("b.db"),
                    &ConnectionId::new("c"),
                    &SessionId::new("s"),
                )
                .unwrap();
                // Return `dir` too: dropping it deletes the DB files, so it must
                // outlive the timed body.
                (dir, store)
            },
            |(dir, store)| {
                for (ord, it) in items.iter().enumerate() {
                    store.upsert_item(ord as i64, it).unwrap();
                }
                let loaded = store.load_items().unwrap();
                // Hand everything back so teardown — SQLite close, tempdir file
                // deletion, and the ~1MB loaded-Vec dealloc — is dropped OUTSIDE
                // the timed body (criterion drops routine outputs after timing).
                (dir, store, loaded)
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_transcript);
criterion_main!(benches);
