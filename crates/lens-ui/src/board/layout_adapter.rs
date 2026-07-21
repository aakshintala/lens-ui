//! B-4-REPLACED STUB — provisional board tree derived from live `FleetStore`.
//!
//! Basis B (plan `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md`):
//! B-2 does NOT wire the persisted `SqliteBoardStore` into `lens-ui`. Until B-4
//! lands the store→replica seam alongside the first board writes, the packer
//! walks a `BoardLayout` fabricated here from the fleet's live cards — all loose,
//! deterministic order. This temporarily makes placement FleetStore-derived; the
//! two guardrails against that calcifying (per the plan's Global Constraints) are
//! that `board_tree` takes `&BoardLayout` (consumers are blind to the source) and
//! THIS comment. B-4 deletes this file and swaps in the real replica.

use crate::fleet::store::FleetStore;
use lens_core::domain::board::{
    Board, BoardItem, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME,
};
use lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};

/// Build a loose-card `BoardLayout` from the fleet's current cards, ordered
/// deterministically by session-id string (matches the retired placeholder's
/// order). No groups — none are creatable until B-4. `created_at`/ordinals are
/// synthetic (nothing is persisted).
pub fn build_ephemeral_layout(fleet: &FleetStore) -> BoardLayout {
    const EPOCH: i64 = 0;
    let board_id = BoardId::new(DEFAULT_BOARD_ID);
    let conn = ConnectionId::new("conn_ephemeral");

    let mut sessions: Vec<SessionId> = fleet.cards.keys().cloned().collect();
    sessions.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let items = sessions
        .into_iter()
        .enumerate()
        .map(|(i, session)| BoardItem {
            id: BoardItemId::new(format!("eph_{}", session.as_str())),
            board_id: board_id.clone(),
            parent_item_id: None,
            ordinal: i as i32,
            kind: BoardItemKind::Card { conn: conn.clone(), session },
            created_at: EPOCH,
        })
        .collect();

    BoardLayout {
        boards: vec![Board {
            id: board_id,
            name: DEFAULT_BOARD_NAME.into(),
            ordinal: 0,
            created_at: EPOCH,
            updated_at: EPOCH,
        }],
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualUiClock;
    use crate::clock::UiClock;
    use lens_core::domain::board::BoardNode;
    use std::sync::Arc;

    #[gpui::test]
    fn ephemeral_layout_is_ordered_loose_cards(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(ManualUiClock::new(0));
        let (layout, board) = cx.update(|cx| {
            let fleet = FleetStore::new(Arc::clone(&clock) as Arc<dyn UiClock>, cx);
            fleet.update(cx, |f, cx| {
                // Insert out of lexical order to prove the sort.
                f.spawn_fake_session(SessionId::new("s2"), cx);
                f.spawn_fake_session(SessionId::new("s1"), cx);
                f.spawn_fake_session(SessionId::new("s3"), cx);
            });
            let layout = build_ephemeral_layout(fleet.read(cx));
            let board = layout.default_board_id().unwrap().clone();
            (layout, board)
        });

        let nodes = layout.board_tree(&board).unwrap();
        assert_eq!(nodes.len(), 3);
        assert!(nodes.iter().all(|n| matches!(n, BoardNode::Card(_))));
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(
            sessions,
            vec![&SessionId::new("s1"), &SessionId::new("s2"), &SessionId::new("s3")]
        );
    }
}
