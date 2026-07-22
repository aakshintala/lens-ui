//! B-4a supporting bench: the pure `board_tree` walk at scale (1000 cards + a group),
//! a CI regression signal for the render read path. NOT the frame-budget proof — that is
//! the on-device E2E on the real BoardView (design §7 measure #1). Matches the existing
//! persist_throughput / reduce_throughput bench convention.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lens_core::domain::board::{
    Board, BoardItem, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME,
};
use lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};

/// A layout of `n` cards (half under one group, half loose) on the default board.
fn build_layout_with_group(n: usize) -> BoardLayout {
    let board = BoardId::new(DEFAULT_BOARD_ID);
    let conn = ConnectionId::new("c");
    let group_id = BoardItemId::new("g0");

    let mut items = Vec::with_capacity(n + 1);
    items.push(BoardItem {
        id: group_id.clone(),
        board_id: board.clone(),
        parent_item_id: None,
        ordinal: 0,
        kind: BoardItemKind::Group {
            name: "Group".into(),
            color_token: Some("blue".into()),
            collapsed: false,
            archived: false,
        },
        created_at: 0,
    });
    for i in 0..n {
        let under_group = i < n / 2;
        items.push(BoardItem {
            id: BoardItemId::new(format!("c{i}")),
            board_id: board.clone(),
            parent_item_id: under_group.then(|| group_id.clone()),
            ordinal: i as i32 + 1,
            kind: BoardItemKind::Card {
                conn: conn.clone(),
                session: SessionId::new(format!("s{i:04}")),
            },
            created_at: 0,
        });
    }

    BoardLayout {
        boards: vec![Board {
            id: board,
            name: DEFAULT_BOARD_NAME.into(),
            ordinal: 0,
            created_at: 0,
            updated_at: 0,
        }],
        items,
    }
}

fn bench_board_tree(c: &mut Criterion) {
    let layout = build_layout_with_group(1000);
    let board = layout.default_board_id().unwrap().clone();
    c.bench_function("board_tree_1000_with_group", |b| {
        b.iter(|| black_box(layout.board_tree(&board).unwrap().len()))
    });
}

criterion_group!(benches, bench_board_tree);
criterion_main!(benches);
