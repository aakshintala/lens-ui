use lens_core::domain::ids::{BoardId, BoardItemId, SessionId};
use lens_core::pack::{
    DraggedKind, DropTarget, DropTile, Item, Kind, resolve_drop, to_move_ordinal,
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

/// One committed top-level pack row before drag reflow preview.
#[derive(Clone, Debug)]
pub struct ReflowPreviewInput {
    pub item: Item,
    pub item_id: BoardItemId,
    pub sessions: Vec<SessionId>,
}

/// One row after drag reflow preview (gap rows carry `is_gap: true`).
#[derive(Clone, Debug)]
pub struct ReflowPreviewRow {
    pub item: Item,
    pub item_id: BoardItemId,
    pub sessions: Vec<SessionId>,
    pub is_gap: bool,
}

/// Pure transform for drag reflow preview: remove the dragged top-level tile, insert a
/// top-level gap row, and/or grow an into-group target so packing reserves the slot.
/// Returns transformed rows and, for top-level targets, the gap row index.
pub fn apply_reflow_preview(
    rows: &[ReflowPreviewInput],
    dragged_id: &BoardItemId,
    target: &DropTarget,
    dragged_footprint: Item,
    start_parent: Option<&BoardItemId>,
) -> (Vec<ReflowPreviewRow>, Option<usize>) {
    let mut out: Vec<ReflowPreviewRow> = rows
        .iter()
        .map(|r| ReflowPreviewRow {
            item: r.item,
            item_id: r.item_id.clone(),
            sessions: r.sessions.clone(),
            is_gap: false,
        })
        .collect();

    let removed = out
        .iter()
        .position(|r| r.item_id == *dragged_id)
        .map(|pos| out.remove(pos));

    let mut gap_index = None;

    match &target.parent {
        None => {
            let item = removed
                .as_ref()
                .map(|r| r.item)
                .unwrap_or(dragged_footprint);
            let sessions = removed.map(|r| r.sessions).unwrap_or_default();
            let ord = target.ordinal.min(out.len());
            out.insert(
                ord,
                ReflowPreviewRow {
                    item,
                    item_id: dragged_id.clone(),
                    sessions,
                    is_gap: true,
                },
            );
            gap_index = Some(ord);
        }
        Some(group_id) => {
            let same_group = start_parent == Some(group_id);
            if !same_group && let Some(row) = out.iter_mut().find(|r| r.item_id == *group_id) {
                // Grow the group DOWNWARD only: keep the committed column width (`fc`) and add
                // rows to fit the incoming card. Re-running `foot(n+1)` could change `fc` (a full
                // 2×2 → 3×2, or 3×1 → 2×2), which shifts `used_cols` → `center_offset` → the whole
                // content block, making the group jump under the cursor and re-triggering the
                // reflow feedback loop the frozen snapshot exists to prevent (§4.1).
                let fc = row.item.fc.max(1);
                let members = row.sessions.len() + 1;
                let fr = members.div_ceil(fc).max(1);
                row.item = Item {
                    kind: Kind::Group { members },
                    fc,
                    fr,
                };
            }
        }
    }

    (out, gap_index)
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

use gpui::{Context, Hsla, Render, div, prelude::*, px};

/// Group drag ghost — cards use the live `SessionCardView` entity instead.
#[derive(Clone)]
pub struct DragGhost {
    #[allow(dead_code)]
    id: BoardItemId,
    name: String,
    accent: Hsla,
    spend_age: String,
}

impl DragGhost {
    pub fn group(id: BoardItemId, name: String, accent: Hsla, spend_age: String) -> Self {
        Self {
            id,
            name,
            accent,
            spend_age,
        }
    }
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(lens_core::pack::CARD_W))
            .h(px(lens_core::pack::CARD_H))
            .rounded(px(12.0))
            .bg(self.accent.opacity(0.12))
            .border_1()
            .border_color(self.accent)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(lens_core::pack::HEADER))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .px_1p5()
                    .child(div().size(px(8.0)).rounded_full().bg(self.accent))
                    .child(
                        div()
                            .text_color(gpui::rgb(0xd6d6de))
                            .overflow_hidden()
                            .child(self.name.clone()),
                    ),
            )
            .child(
                div()
                    .flex_grow()
                    .px_1p5()
                    .text_color(gpui::rgb(0x8a8a94))
                    .child(self.spend_age.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use lens_core::domain::board::DEFAULT_BOARD_ID;
    use lens_core::domain::ids::{BoardId, BoardItemId};
    use lens_core::pack::{DraggedKind, DropTile, Item, Kind, pack};

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

    fn sid(s: &str) -> SessionId {
        SessionId::new(s)
    }

    #[test]
    fn reflow_preview_into_group_grows_footprint() {
        let group_id = bid("g");
        let rows = vec![ReflowPreviewInput {
            item: Item::group(4),
            item_id: group_id.clone(),
            sessions: (0..4).map(|i| sid(&format!("s{i}"))).collect(),
        }];
        let (out, gap) = apply_reflow_preview(
            &rows,
            &bid("card"),
            &DropTarget {
                parent: Some(group_id),
                ordinal: 2,
            },
            Item::card(),
            None,
        );
        assert!(gap.is_none());
        assert_eq!(out.len(), 1);
        let grown = &out[0].item;
        // Grows DOWNWARD: keeps the committed 2-col width and adds a row (2×2 → 2×3),
        // never re-running foot() (which would widen to 3×2 and shift the block).
        assert_eq!((grown.fc, grown.fr), (2, 3));
        assert!(matches!(grown.kind, Kind::Group { members: 5 }));
    }

    #[test]
    fn reflow_preview_top_level_inserts_gap_at_ordinal() {
        let rows = vec![
            ReflowPreviewInput {
                item: Item::card(),
                item_id: bid("a"),
                sessions: vec![sid("s0")],
            },
            ReflowPreviewInput {
                item: Item::card(),
                item_id: bid("b"),
                sessions: vec![sid("s1")],
            },
            ReflowPreviewInput {
                item: Item::card(),
                item_id: bid("c"),
                sessions: vec![sid("s2")],
            },
        ];
        let (out, gap) = apply_reflow_preview(
            &rows,
            &bid("b"),
            &DropTarget {
                parent: None,
                ordinal: 0,
            },
            Item::card(),
            None,
        );
        assert_eq!(gap, Some(0));
        assert_eq!(out.len(), 3);
        assert!(out[0].is_gap);
        assert_eq!(out[0].item_id, bid("b"));
        assert!(!out[1].is_gap);
        assert_eq!(out[1].item_id, bid("a"));
    }

    #[test]
    fn reflow_preview_same_group_reorder_does_not_grow() {
        let group_id = bid("g");
        let sessions: Vec<_> = (0..3).map(|i| sid(&format!("s{i}"))).collect();
        let rows = vec![ReflowPreviewInput {
            item: Item::group(3),
            item_id: group_id.clone(),
            sessions: sessions.clone(),
        }];
        let (out, gap) = apply_reflow_preview(
            &rows,
            &bid("card-in-g"),
            &DropTarget {
                parent: Some(group_id.clone()),
                ordinal: 1,
            },
            Item::card(),
            Some(&group_id),
        );
        assert!(gap.is_none());
        assert_eq!(out.len(), 1);
        let unchanged = &out[0].item;
        assert_eq!((unchanged.fc, unchanged.fr), (3, 1));
        assert!(matches!(unchanged.kind, Kind::Group { members: 3 }));
    }
}
