use lens_core::domain::ids::{BoardId, BoardItemId};
use lens_core::pack::{
    resolve_drop, to_move_ordinal, DropTarget, DropTile, DraggedKind, Item,
};

use crate::board::replica::Op;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Idle,
    Dragging,
    Committing,
}

#[derive(Clone, Debug)]
pub struct DragSession {
    pub phase: DragPhase,
    pub dragged_id: BoardItemId,
    pub dragged_kind: DraggedKind,
    pub start_generation: u64,
    pub snapshot: Vec<DropTile>,
    pub target: DropTarget,
    pub dragged_sibling_index: Option<usize>,
    /// Parent at drag-start (`None` = top-level) — for same-parent shift.
    pub start_parent: Option<BoardItemId>,
    pub board_id: BoardId,
}

pub fn start_drag(
    dragged_id: BoardItemId,
    dragged_kind: DraggedKind,
    snapshot: Vec<DropTile>,
    start_generation: u64,
    initial_cursor: (f32, f32),
    dragged_sibling_index: Option<usize>,
    start_parent: Option<BoardItemId>,
    board_id: BoardId,
) -> DragSession {
    let target = resolve_drop(&snapshot, initial_cursor, dragged_kind);
    DragSession {
        phase: DragPhase::Dragging,
        dragged_id,
        dragged_kind,
        start_generation,
        snapshot,
        target,
        dragged_sibling_index,
        start_parent,
        board_id,
    }
}

pub fn on_cursor_move(
    session: &mut DragSession,
    cursor: (f32, f32),
    current_generation: u64,
) -> bool {
    if session.phase != DragPhase::Dragging {
        return false;
    }
    if current_generation != session.start_generation {
        cancel(session);
        return false;
    }
    session.target = resolve_drop(&session.snapshot, cursor, session.dragged_kind);
    true
}

pub fn begin_commit(
    session: &mut DragSession,
    current_generation: u64,
) -> Option<Op> {
    if session.phase != DragPhase::Dragging {
        return None;
    }
    if current_generation != session.start_generation {
        cancel(session);
        return None;
    }
    let sibling_idx = if session.target.parent == session.start_parent {
        session.dragged_sibling_index
    } else {
        None
    };
    let new_ordinal = to_move_ordinal(session.target.ordinal, sibling_idx) as i32;
    let op = Op::MoveItem {
        item_id: session.dragged_id.clone(),
        new_board_id: session.board_id.clone(),
        new_parent: session.target.parent.clone(),
        new_ordinal,
    };
    session.phase = DragPhase::Committing;
    Some(op)
}

pub fn on_wrote(session: &mut DragSession) {
    *session = idle_shell(session);
}

pub fn on_failed(session: &mut DragSession) {
    *session = idle_shell(session);
}

pub fn cancel(session: &mut DragSession) {
    *session = idle_shell(session);
}

fn idle_shell(session: &DragSession) -> DragSession {
    DragSession {
        phase: DragPhase::Idle,
        dragged_id: session.dragged_id.clone(),
        dragged_kind: session.dragged_kind,
        start_generation: session.start_generation,
        snapshot: Vec::new(),
        target: DropTarget {
            parent: None,
            ordinal: 0,
        },
        dragged_sibling_index: None,
        start_parent: None,
        board_id: session.board_id.clone(),
    }
}

pub fn reflow_preview_placeholder_footprint(item: &Item) -> (usize, usize) {
    (item.fc.max(1), item.fr.max(1))
}

#[cfg(test)]
mod tests {
    use lens_core::domain::board::DEFAULT_BOARD_ID;
    use lens_core::domain::ids::{BoardId, BoardItemId};
    use lens_core::pack::{pack, DropTile, DraggedKind, Item};

    use super::*;

    fn bid(s: &str) -> BoardItemId {
        BoardItemId::new(s)
    }
    fn board() -> BoardId {
        BoardId::new(DEFAULT_BOARD_ID)
    }

    fn three_card_snapshot() -> Vec<DropTile> {
        let items = [Item::card(), Item::card()];
        let packing = pack(&items, 1);
        packing
            .tiles
            .into_iter()
            .zip(["a", "c"])
            .map(|(placed, id)| DropTile {
                placed,
                id: bid(id),
                collapsed: false,
            })
            .collect()
    }

    fn start_b() -> DragSession {
        start_drag(
            bid("b"),
            DraggedKind::Card,
            three_card_snapshot(),
            1,
            (140.0, 4.0),
            Some(1),
            None,
            board(),
        )
    }

    #[test]
    fn dragging_to_committing_holds_preview_target() {
        let mut s = start_b();
        assert_eq!(s.phase, DragPhase::Dragging);
        let held = s.target.clone();
        assert!(begin_commit(&mut s, 1).is_some());
        assert_eq!(s.phase, DragPhase::Committing);
        assert_eq!(s.target, held, "Committing holds last reflow-preview target");
    }

    #[test]
    fn wrote_invisible_swap_returns_idle() {
        let mut s = start_b();
        let _ = begin_commit(&mut s, 1);
        on_wrote(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn failed_discards_preview_returns_idle() {
        let mut s = start_b();
        let _ = begin_commit(&mut s, 1);
        on_failed(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
        assert!(s.snapshot.is_empty());
    }

    #[test]
    fn cancel_returns_idle() {
        let mut s = start_b();
        cancel(&mut s);
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn external_commit_mid_drag_abandons_during_dragging() {
        let mut s = start_b();
        assert!(!on_cursor_move(&mut s, (140.0, 80.0), 2));
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn external_commit_aborts_at_drop() {
        let mut s = start_b();
        assert!(begin_commit(&mut s, 2).is_none(), "generation changed → abort");
        assert_eq!(s.phase, DragPhase::Idle);
    }
}
