//! Runnable demo of the B-4c drag reverse hit-test: prints how a few cursor points over a
//! backfilled masonry resolve to `(parent, ordinal)`. Run:
//!   cargo run --manifest-path spikes/board-drag/Cargo.toml
//! The tests (`cargo test --manifest-path spikes/board-drag/Cargo.toml`) are the real proof;
//! this is a human-legible sanity trace.

use board_drag::{TopItem, resolve_drop};
use lens_core::pack::{CARD_H, CARD_W, CELL_W, GAP, HEADER};

fn main() {
    // group(4) [2×2] then two loose cards, in 3 columns — the non-monotonic backfill layout.
    let board = [
        TopItem::Group {
            id: "g",
            members: vec!["m0", "m1", "m2", "m3"],
            collapsed: false,
        },
        TopItem::Card("x"),
        TopItem::Card("y"),
    ];
    let cols = 3;

    let probes = [
        ("over card x (col2, top)", (2.0 * CELL_W + CARD_W / 2.0, 4.0)),
        (
            "over card y (col2, below x)",
            (2.0 * CELL_W + CARD_W / 2.0, CARD_H + GAP + 4.0),
        ),
        (
            "into group body, member m0",
            (CARD_W / 2.0, HEADER + CARD_H / 2.0),
        ),
        (
            "into group body, right of m1",
            (CELL_W + CARD_W - 4.0, HEADER + CARD_H / 2.0),
        ),
        // NOT "append": at x=140 the nearest tile below is the GROUP (col 0/1), so this
        // resolves to "after the group" = ordinal 1, not end-of-list. Masonry columns end at
        // different heights → nearest-tile has no natural end-of-list target. (Open decision:
        // append when cursor.y > content_height. See findings doc.)
        ("below col-0/1 (nearest=group!)", (CARD_W / 2.0, 10_000.0)),
    ];

    println!("board: [group g(m0..m3, 2x2), card x, card y]  cols={cols}");
    println!("(masonry backfills x,y into column 2 beside the group — ordinal != reading order)\n");
    for (label, cursor) in probes {
        let t = resolve_drop(&board, cols, cursor);
        let parent = t.parent.unwrap_or("<top-level>");
        println!("  {label:32}  cursor=({:6.0},{:6.0})  ->  parent={parent:12} ordinal={}", cursor.0, cursor.1, t.ordinal);
    }
}
