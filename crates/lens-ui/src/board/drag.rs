use lens_core::domain::ids::{BoardId, BoardItemId};
use lens_core::pack::{DraggedKind, DropTarget, DropTile, Item, resolve_drop, to_move_ordinal};

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

#[allow(clippy::too_many_arguments)] // locked drag-start signature (plan §3 Interfaces)
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

pub fn begin_commit(session: &mut DragSession, current_generation: u64) -> Option<Op> {
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

/// Positive delta scrolls content down (cursor near bottom); negative toward top.
pub fn edge_scroll_delta(
    cursor_y: f32,
    viewport_top: f32,
    viewport_h: f32,
    band_px: f32,
    nudge_px: f32,
) -> f32 {
    let y = cursor_y - viewport_top;
    if y <= band_px {
        -nudge_px
    } else if y >= viewport_h - band_px {
        nudge_px
    } else {
        0.0
    }
}

use gpui::{Context, Render, div, prelude::*, px};

/// Lightweight ghost following the cursor during drag (real chrome is Task 6 polish).
pub struct DragGhost {
    #[allow(dead_code)] // payload identity; ghost chrome is Task 6 polish
    pub id: BoardItemId,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(lens_core::pack::CARD_W))
            .h(px(lens_core::pack::CARD_H))
            .rounded(px(8.0))
            .bg(gpui::rgb(0x3a3a44))
            .border_1()
            .border_color(gpui::rgb(0x6a6a74))
    }
}

#[cfg(test)]
mod tests {
    use lens_core::domain::board::DEFAULT_BOARD_ID;
    use lens_core::domain::ids::{BoardId, BoardItemId};
    use lens_core::pack::{DraggedKind, DropTile, Item, pack};

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
        assert_eq!(
            s.target, held,
            "Committing holds last reflow-preview target"
        );
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
        assert!(
            begin_commit(&mut s, 2).is_none(),
            "generation changed → abort"
        );
        assert_eq!(s.phase, DragPhase::Idle);
    }

    #[test]
    fn edge_scroll_nudges_near_top_and_bottom() {
        assert_eq!(edge_scroll_delta(10.0, 0.0, 600.0, 40.0, 12.0), -12.0);
        assert_eq!(edge_scroll_delta(590.0, 0.0, 600.0, 40.0, 12.0), 12.0);
        assert_eq!(edge_scroll_delta(300.0, 0.0, 600.0, 40.0, 12.0), 0.0);
    }

    #[test]
    fn reflow_preview_uses_gap_not_second_card() {
        let card = Item::card();
        assert_eq!(reflow_preview_placeholder_footprint(&card), (1, 1));
        let g = Item::group(4);
        assert_eq!(reflow_preview_placeholder_footprint(&g), (g.fc, g.fr));
    }
}
