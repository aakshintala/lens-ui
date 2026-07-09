//! Persistence throughput (AGENTS.md benchmark-or-it's-not-done). Persistence is
//! I/O-bound; this measures the write-through + reload cost over a realistic item
//! count so regressions are visible, not to claim a CPU budget.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lens_core::domain::ids::{ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::{SqliteTranscriptStore, TranscriptStore};

fn item(i: usize) -> Item {
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
                text: Some("lorem ipsum dolor".into()),
                data: serde_json::Value::Null,
            }],
        },
    }
}

fn bench_transcript(c: &mut Criterion) {
    let items: Vec<Item> = (0..200).map(item).collect();
    c.bench_function("transcript_write_200_then_load", |b| {
        b.iter(|| {
            let dir = tempfile::tempdir().unwrap();
            let s = SqliteTranscriptStore::open(
                &dir.path().join("b.db"),
                &ConnectionId::new("c"),
                &SessionId::new("s"),
            )
            .unwrap();
            for (ord, it) in items.iter().enumerate() {
                s.upsert_item(ord as i64, it).unwrap();
            }
            black_box(s.load_items().unwrap());
        });
    });
}

criterion_group!(benches, bench_transcript);
criterion_main!(benches);
