//! Foreground `apply` micro-bench — proves copy-assignment is O(1) vs transcript
//! length (item bodies are `Arc`, so an append is a pointer move).
//! Run: `cargo bench -p lens-store --features bench`

use std::sync::Arc;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use lens_core::domain::SessionState;
use lens_core::domain::ids::{AgentId, ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::{Role, SessionStatusValue};
use lens_core::reduce::StreamUpdate;
use lens_store::apply;

fn sample_item(id: &str) -> Item {
    Item {
        id: ItemId::new(id),
        seq: None,
        ctx: BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        },
        created_at: 0,
        kind: ItemKind::Message {
            role: Role::User,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some("x".into()),
                data: Default::default(),
            }],
        },
    }
}

fn state_with_n_items(n: usize) -> SessionState {
    let mut state = SessionState::new(
        ConnectionId::new("c"),
        SessionId::new("conv"),
        AgentId::new("ag"),
    );
    for i in 0..n {
        state
            .items
            .push(Arc::new(sample_item(&format!("item_{i}"))));
    }
    state
}

fn bench_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply");

    for n in [0usize, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::new("apply_item_appended", n), &n, |b, &n| {
            b.iter_batched(
                || state_with_n_items(n),
                |mut state| {
                    apply(
                        &mut state,
                        StreamUpdate::ItemAppended(Arc::new(sample_item("bench"))),
                    );
                    state
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.bench_function("apply_status_changed", |b| {
        b.iter_batched(
            || state_with_n_items(10_000),
            |mut state| {
                apply(
                    &mut state,
                    StreamUpdate::StatusChanged(SessionStatusValue::Running),
                );
                state
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_apply);
criterion_main!(benches);
