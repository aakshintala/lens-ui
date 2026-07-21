//! Board placement model (B-1): boards, ordinal-slot item trees, and pure mutation ops.
//! Content (session cards) lives elsewhere; items here are placement only.

use crate::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};
use std::collections::HashMap;
use thiserror::Error;

/// Stable id for the seeded default board (first run).
pub const DEFAULT_BOARD_ID: &str = "board_default";
/// Default board display name (B-5 owns rename UI).
pub const DEFAULT_BOARD_NAME: &str = "Main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub id: BoardId,
    pub name: String,
    pub ordinal: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardItem {
    pub id: BoardItemId,
    pub board_id: BoardId,
    /// `None` = board root.
    pub parent_item_id: Option<BoardItemId>,
    pub ordinal: i32,
    pub kind: BoardItemKind,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardItemKind {
    Card {
        conn: ConnectionId,
        session: SessionId,
    },
    Group {
        name: String,
        color_token: Option<String>,
        collapsed: bool,
        archived: bool,
    },
}

/// A node in the ordered board walk (`board_tree`). Recursive so nested groups
/// work by construction — depth-1 is committed/tested; deeper is PROVISIONAL
/// until B-5 makes nested groups reachable (spec §1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardNode<'a> {
    /// A loose card item (`kind == Card`).
    Card(&'a BoardItem),
    /// A group and its ordered child nodes.
    Group {
        item: &'a BoardItem,
        members: Vec<BoardNode<'a>>,
    },
}

impl<'a> BoardNode<'a> {
    /// All leaf card sessions under this node, in walk order (a loose card → 1;
    /// a group → its members flattened). Powers the packer member count and the
    /// per-tile session list the renderer looks card views up by.
    pub fn leaf_sessions(&self) -> Vec<&'a SessionId> {
        match self {
            BoardNode::Card(item) => match &item.kind {
                BoardItemKind::Card { session, .. } => vec![session],
                BoardItemKind::Group { .. } => vec![],
            },
            BoardNode::Group { members, .. } => {
                members.iter().flat_map(|m| m.leaf_sessions()).collect()
            }
        }
    }
}

/// In-memory board layout: ordered boards plus a flat item forest keyed by parent links.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BoardLayout {
    pub boards: Vec<Board>,
    pub items: Vec<BoardItem>,
}

/// Where to place a session card. All-`None` means the default board root append slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacementTarget {
    pub board_id: Option<BoardId>,
    pub parent_item_id: Option<BoardItemId>,
    pub ordinal: Option<i32>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BoardError {
    #[error("board not found")]
    BoardNotFound,

    #[error("item not found")]
    ItemNotFound,

    #[error("expected a group item")]
    NotAGroup,

    #[error("cannot move a group into its own descendant")]
    CycleDetected,

    #[error("parent item not found or not a group")]
    InvalidParent,

    #[error("card item already exists for this session")]
    DuplicateSession,
}

impl BoardLayout {
    pub fn default_board_id(&self) -> Result<&BoardId, BoardError> {
        self.boards
            .iter()
            .min_by_key(|b| (b.ordinal, b.id.as_str()))
            .map(|b| &b.id)
            .ok_or(BoardError::BoardNotFound)
    }

    pub fn find_card(&self, conn: &ConnectionId, session: &SessionId) -> Option<&BoardItem> {
        self.items.iter().find(|item| {
            matches!(
                &item.kind,
                BoardItemKind::Card { conn: c, session: s } if c == conn && s == session
            )
        })
    }

    pub fn item(&self, id: &BoardItemId) -> Option<&BoardItem> {
        self.items.iter().find(|i| &i.id == id)
    }

    pub fn item_mut(&mut self, id: &BoardItemId) -> Option<&mut BoardItem> {
        self.items.iter_mut().find(|i| &i.id == id)
    }

    fn siblings<'a>(
        &'a self,
        board_id: &BoardId,
        parent: Option<&BoardItemId>,
    ) -> Vec<&'a BoardItem> {
        let mut out: Vec<_> = self
            .items
            .iter()
            .filter(|i| {
                i.board_id == *board_id
                    && i.parent_item_id.as_ref() == parent
                    && i.parent_item_id.is_none() == parent.is_none()
            })
            .collect();
        out.sort_by_key(|i| (i.ordinal, i.id.as_str()));
        out
    }

    fn sibling_ids_sorted(
        &self,
        board_id: &BoardId,
        parent: Option<&BoardItemId>,
    ) -> Vec<BoardItemId> {
        let mut ids: Vec<BoardItemId> = self
            .items
            .iter()
            .filter(|i| {
                i.board_id == *board_id
                    && i.parent_item_id.as_ref() == parent
                    && i.parent_item_id.is_none() == parent.is_none()
            })
            .map(|i| i.id.clone())
            .collect();
        ids.sort_by(|a, b| {
            let oa = self.item(a).map(|i| i.ordinal).unwrap_or(0);
            let ob = self.item(b).map(|i| i.ordinal).unwrap_or(0);
            oa.cmp(&ob).then_with(|| a.as_str().cmp(b.as_str()))
        });
        ids
    }

    fn assign_sibling_ordinals(
        &mut self,
        board_id: &BoardId,
        parent: Option<BoardItemId>,
        ordered: Vec<BoardItemId>,
    ) {
        for (ord, id) in ordered.into_iter().enumerate() {
            if let Some(item) = self.item_mut(&id) {
                debug_assert_eq!(item.board_id, *board_id);
                debug_assert_eq!(item.parent_item_id, parent);
                item.ordinal = ord as i32;
            }
        }
    }

    fn insert_at_ordinal(
        &mut self,
        board_id: &BoardId,
        parent: Option<BoardItemId>,
        ordinal: i32,
        item: BoardItem,
    ) {
        let parent_ref = parent.as_ref();
        let mut order = self.sibling_ids_sorted(board_id, parent_ref);
        let idx = (ordinal.max(0) as usize).min(order.len());
        let id = item.id.clone();
        self.items.push(item);
        order.insert(idx, id);
        self.assign_sibling_ordinals(board_id, parent, order);
    }

    fn renumber_siblings(&mut self, board_id: &BoardId, parent: Option<BoardItemId>) {
        let order = self.sibling_ids_sorted(board_id, parent.as_ref());
        self.assign_sibling_ordinals(board_id, parent, order);
    }

    fn is_descendant(&self, ancestor: &BoardItemId, candidate: &BoardItemId) -> bool {
        if ancestor == candidate {
            return true;
        }
        let mut stack = vec![ancestor.clone()];
        let mut seen = HashMap::new();
        while let Some(id) = stack.pop() {
            if seen.insert(id.clone(), ()).is_some() {
                continue;
            }
            for child in self
                .items
                .iter()
                .filter(|i| i.parent_item_id.as_ref().map(|p| p.as_str()) == Some(id.as_str()))
            {
                if &child.id == candidate {
                    return true;
                }
                if matches!(child.kind, BoardItemKind::Group { .. }) {
                    stack.push(child.id.clone());
                }
            }
        }
        false
    }

    fn assert_group_parent(
        &self,
        board_id: &BoardId,
        parent: Option<&BoardItemId>,
    ) -> Result<(), BoardError> {
        if let Some(pid) = parent {
            let parent_item = self.item(pid).ok_or(BoardError::InvalidParent)?;
            if parent_item.board_id != *board_id {
                return Err(BoardError::InvalidParent);
            }
            if !matches!(parent_item.kind, BoardItemKind::Group { .. }) {
                return Err(BoardError::InvalidParent);
            }
        }
        Ok(())
    }

    pub fn create_group(
        &mut self,
        board_id: &BoardId,
        parent_item_id: Option<BoardItemId>,
        ordinal: i32,
        name: impl Into<String>,
        item_id: BoardItemId,
        created_at: i64,
    ) -> Result<(), BoardError> {
        if !self.boards.iter().any(|b| &b.id == board_id) {
            return Err(BoardError::BoardNotFound);
        }
        self.assert_group_parent(board_id, parent_item_id.as_ref())?;
        let parent = parent_item_id.clone();
        self.insert_at_ordinal(
            board_id,
            parent,
            ordinal,
            BoardItem {
                id: item_id,
                board_id: board_id.clone(),
                parent_item_id,
                ordinal,
                kind: BoardItemKind::Group {
                    name: name.into(),
                    color_token: None,
                    collapsed: false,
                    archived: false,
                },
                created_at,
            },
        );
        Ok(())
    }

    pub fn place_session(
        &mut self,
        conn: ConnectionId,
        session: SessionId,
        target: &PlacementTarget,
        item_id: BoardItemId,
        created_at: i64,
    ) -> Result<(), BoardError> {
        if self.find_card(&conn, &session).is_some() {
            return Ok(());
        }
        let board_id = target
            .board_id
            .clone()
            .or_else(|| self.default_board_id().ok().cloned())
            .ok_or(BoardError::BoardNotFound)?;
        if !self.boards.iter().any(|b| b.id == board_id) {
            return Err(BoardError::BoardNotFound);
        }
        let parent_item_id = target.parent_item_id.clone();
        self.assert_group_parent(&board_id, parent_item_id.as_ref())?;
        let ordinal = target.ordinal.unwrap_or_else(|| {
            self.sibling_ids_sorted(&board_id, parent_item_id.as_ref())
                .len() as i32
        });
        let parent = parent_item_id.clone();
        self.insert_at_ordinal(
            &board_id,
            parent,
            ordinal,
            BoardItem {
                id: item_id,
                board_id: board_id.clone(),
                parent_item_id,
                ordinal,
                kind: BoardItemKind::Card { conn, session },
                created_at,
            },
        );
        Ok(())
    }

    pub fn remove_session(
        &mut self,
        conn: &ConnectionId,
        session: &SessionId,
    ) -> Result<(), BoardError> {
        let Some(idx) = self.items.iter().position(|item| {
            matches!(
                &item.kind,
                BoardItemKind::Card { conn: c, session: s } if c == conn && s == session
            )
        }) else {
            return Ok(());
        };
        let (board_id, parent) = {
            let item = &self.items[idx];
            (item.board_id.clone(), item.parent_item_id.clone())
        };
        self.items.remove(idx);
        self.renumber_siblings(&board_id, parent);
        Ok(())
    }

    pub fn move_item(
        &mut self,
        item_id: &BoardItemId,
        new_board_id: &BoardId,
        new_parent: Option<BoardItemId>,
        new_ordinal: i32,
    ) -> Result<(), BoardError> {
        if !self.boards.iter().any(|b| &b.id == new_board_id) {
            return Err(BoardError::BoardNotFound);
        }
        self.assert_group_parent(new_board_id, new_parent.as_ref())?;
        let old_parent = self
            .item(item_id)
            .ok_or(BoardError::ItemNotFound)?
            .parent_item_id
            .clone();
        let old_board = self
            .item(item_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        if matches!(
            self.item(item_id).map(|i| &i.kind),
            Some(BoardItemKind::Group { .. })
        ) && let Some(ref parent) = new_parent
            && self.is_descendant(item_id, parent)
        {
            return Err(BoardError::CycleDetected);
        }
        let mut old_order = self.sibling_ids_sorted(&old_board, old_parent.as_ref());
        old_order.retain(|id| id != item_id);
        self.assign_sibling_ordinals(&old_board, old_parent.clone(), old_order);
        {
            let item = self.item_mut(item_id).ok_or(BoardError::ItemNotFound)?;
            item.board_id = new_board_id.clone();
            item.parent_item_id = new_parent.clone();
        }
        // Cross-board move: the moved node's descendants keep their parent links but
        // must follow it onto the new board, else the subtree is stranded on the old
        // board under a parent that no longer lives there.
        if old_board != *new_board_id {
            self.reassign_subtree_board(item_id, new_board_id);
        }
        let mut new_order = self.sibling_ids_sorted(new_board_id, new_parent.as_ref());
        new_order.retain(|id| id != item_id);
        let idx = (new_ordinal.max(0) as usize).min(new_order.len());
        new_order.insert(idx, item_id.clone());
        self.assign_sibling_ordinals(new_board_id, new_parent, new_order);
        Ok(())
    }

    /// Set `board_id` on every descendant of `root` (exclusive) to `new_board_id`.
    /// Parent links are unchanged — only board membership moves with the subtree.
    fn reassign_subtree_board(&mut self, root: &BoardItemId, new_board_id: &BoardId) {
        // `seen` guards against non-termination on a corrupt cyclic `parent_item_id`
        // graph. The mutation API can't create a cycle (`move_item` rejects them),
        // but rows loaded from a hand-edited/corrupt DB could form one, and the
        // store's contract is to never hang on bad data (cf. corrupt-row skip).
        let mut stack = vec![root.clone()];
        let mut seen: HashMap<String, ()> = HashMap::new();
        while let Some(id) = stack.pop() {
            if seen.insert(id.as_str().to_string(), ()).is_some() {
                continue;
            }
            let child_ids: Vec<BoardItemId> = self
                .items
                .iter()
                .filter(|i| i.parent_item_id.as_ref().map(|p| p.as_str()) == Some(id.as_str()))
                .map(|i| i.id.clone())
                .collect();
            for cid in child_ids {
                if let Some(child) = self.item_mut(&cid) {
                    child.board_id = new_board_id.clone();
                }
                stack.push(cid);
            }
        }
    }

    pub fn ungroup(&mut self, group_id: &BoardItemId) -> Result<(), BoardError> {
        let (board_id, parent) = {
            let group = self.item(group_id).ok_or(BoardError::ItemNotFound)?;
            if !matches!(group.kind, BoardItemKind::Group { .. }) {
                return Err(BoardError::NotAGroup);
            }
            (group.board_id.clone(), group.parent_item_id.clone())
        };
        // The group's children, in their current order.
        let mut children: Vec<BoardItemId> = self
            .items
            .iter()
            .filter(|i| i.parent_item_id.as_ref().map(|p| p.as_str()) == Some(group_id.as_str()))
            .map(|i| i.id.clone())
            .collect();
        children.sort_by(|a, b| {
            let oa = self.item(a).map(|i| i.ordinal).unwrap_or(0);
            let ob = self.item(b).map(|i| i.ordinal).unwrap_or(0);
            oa.cmp(&ob).then_with(|| a.as_str().cmp(b.as_str()))
        });
        // Build the parent's new sibling order EXPLICITLY: splice the children into
        // the group's slot position. Relying on `slot + offset` ordinals + an
        // `(ordinal, id)` re-sort interleaves a trailing sibling when the group has
        // >1 child (its ordinals collide with the siblings that followed the group).
        let new_order: Vec<BoardItemId> = self
            .sibling_ids_sorted(&board_id, parent.as_ref())
            .into_iter()
            .flat_map(|id| {
                if &id == group_id {
                    children.clone()
                } else {
                    vec![id]
                }
            })
            .collect();
        // Reparent the children to the group's parent, then drop the group.
        for child_id in &children {
            if let Some(child) = self.item_mut(child_id) {
                child.parent_item_id = parent.clone();
            }
        }
        self.items.retain(|i| &i.id != group_id);
        self.assign_sibling_ordinals(&board_id, parent, new_order);
        Ok(())
    }

    pub fn rename(
        &mut self,
        item_id: &BoardItemId,
        name: impl Into<String>,
    ) -> Result<(), BoardError> {
        let item = self.item_mut(item_id).ok_or(BoardError::ItemNotFound)?;
        match &mut item.kind {
            BoardItemKind::Group { name: n, .. } => *n = name.into(),
            BoardItemKind::Card { .. } => return Err(BoardError::NotAGroup),
        }
        Ok(())
    }

    pub fn set_collapsed(
        &mut self,
        group_id: &BoardItemId,
        collapsed: bool,
    ) -> Result<(), BoardError> {
        let item = self.item_mut(group_id).ok_or(BoardError::ItemNotFound)?;
        match &mut item.kind {
            BoardItemKind::Group { collapsed: c, .. } => *c = collapsed,
            BoardItemKind::Card { .. } => return Err(BoardError::NotAGroup),
        }
        Ok(())
    }

    pub fn set_color(
        &mut self,
        group_id: &BoardItemId,
        token: impl Into<String>,
    ) -> Result<(), BoardError> {
        let item = self.item_mut(group_id).ok_or(BoardError::ItemNotFound)?;
        match &mut item.kind {
            BoardItemKind::Group { color_token, .. } => *color_token = Some(token.into()),
            BoardItemKind::Card { .. } => return Err(BoardError::NotAGroup),
        }
        Ok(())
    }

    pub fn archive(&mut self, item_id: &BoardItemId) -> Result<(), BoardError> {
        let item = self.item_mut(item_id).ok_or(BoardError::ItemNotFound)?;
        match &mut item.kind {
            BoardItemKind::Group { archived, .. } => *archived = true,
            BoardItemKind::Card { .. } => return Err(BoardError::NotAGroup),
        }
        Ok(())
    }

    /// Ordered children at `parent` (`None` = board root), dense ordinals 0..n-1.
    pub fn children(&self, board_id: &BoardId, parent: Option<&BoardItemId>) -> Vec<&BoardItem> {
        let mut sibs = self.siblings(board_id, parent);
        sibs.sort_by_key(|i| i.ordinal);
        sibs
    }

    /// Ordered walk of a board's item forest — the packer input (§4). Top-level
    /// items in ordinal order; each group recurses into its children. **Archived
    /// groups (and their subtrees) are skipped** — they belong to the Archive
    /// surface (B-6). Depth-1 committed; deeper recursive-by-construction (§1).
    pub fn board_tree(&self, board_id: &BoardId) -> Result<Vec<BoardNode<'_>>, BoardError> {
        if !self.boards.iter().any(|b| &b.id == board_id) {
            return Err(BoardError::BoardNotFound);
        }
        Ok(self.nodes_under(board_id, None))
    }

    fn nodes_under(&self, board_id: &BoardId, parent: Option<&BoardItemId>) -> Vec<BoardNode<'_>> {
        self.children(board_id, parent)
            .into_iter()
            .filter_map(|item| match &item.kind {
                BoardItemKind::Card { .. } => Some(BoardNode::Card(item)),
                BoardItemKind::Group { archived: true, .. } => None, // → Archive (B-6)
                BoardItemKind::Group { .. } => Some(BoardNode::Group {
                    item,
                    members: self.nodes_under(board_id, Some(&item.id)),
                }),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};

    fn layout_with_default_board() -> BoardLayout {
        let now = 1_700_000_000_000_i64;
        BoardLayout {
            boards: vec![Board {
                id: BoardId::new(DEFAULT_BOARD_ID),
                name: DEFAULT_BOARD_NAME.into(),
                ordinal: 0,
                created_at: now,
                updated_at: now,
            }],
            items: vec![],
        }
    }

    fn card_id(n: &str) -> BoardItemId {
        BoardItemId::new(n)
    }

    fn sess(n: &str) -> SessionId {
        SessionId::new(n)
    }

    fn conn() -> ConnectionId {
        ConnectionId::new("conn_1")
    }

    fn group_item(id: &str, ordinal: i32, archived: bool) -> BoardItem {
        BoardItem {
            id: BoardItemId::new(id),
            board_id: BoardId::new(DEFAULT_BOARD_ID),
            parent_item_id: None,
            ordinal,
            kind: BoardItemKind::Group {
                name: id.into(),
                color_token: None,
                collapsed: false,
                archived,
            },
            created_at: 1_700_000_000_000,
        }
    }

    fn card_item(id: &str, session: &str, parent: Option<&str>, ordinal: i32) -> BoardItem {
        BoardItem {
            id: BoardItemId::new(id),
            board_id: BoardId::new(DEFAULT_BOARD_ID),
            parent_item_id: parent.map(BoardItemId::new),
            ordinal,
            kind: BoardItemKind::Card {
                conn: conn(),
                session: sess(session),
            },
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn board_tree_orders_loose_cards_by_ordinal() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            card_item("c2", "s2", None, 2),
            card_item("c1", "s1", None, 1),
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(sessions, vec![&sess("s1"), &sess("s2")]);
    }

    #[test]
    fn board_tree_nests_group_members() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            group_item("g1", 1, false),
            card_item("c1", "s1", Some("g1"), 1),
            card_item("c2", "s2", Some("g1"), 2),
            card_item("c3", "s3", None, 2), // loose after the group
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        assert_eq!(nodes.len(), 2); // group node + loose card
        match &nodes[0] {
            BoardNode::Group { members, .. } => {
                assert_eq!(members.len(), 2);
                assert_eq!(nodes[0].leaf_sessions(), vec![&sess("s1"), &sess("s2")]);
            }
            _ => panic!("first node must be the group"),
        }
        assert!(matches!(nodes[1], BoardNode::Card(_)));
    }

    #[test]
    fn board_tree_skips_archived_group() {
        let mut layout = layout_with_default_board();
        layout.items = vec![
            group_item("g_arch", 1, true),
            card_item("c1", "s1", Some("g_arch"), 1), // under archived group → skipped
            card_item("c2", "s2", None, 2),
        ];
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let nodes = layout.board_tree(&board).unwrap();
        let sessions: Vec<_> = nodes.iter().flat_map(|n| n.leaf_sessions()).collect();
        assert_eq!(sessions, vec![&sess("s2")]); // archived subtree absent
    }

    #[test]
    fn board_tree_unknown_board_errs() {
        let layout = layout_with_default_board();
        let err = layout.board_tree(&BoardId::new("nope")).unwrap_err();
        assert_eq!(err, BoardError::BoardNotFound);
    }

    #[test]
    fn place_session_appends_loose_card_with_dense_ordinals() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .place_session(
                conn(),
                sess("s1"),
                &PlacementTarget::default(),
                card_id("c1"),
                1,
            )
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("s2"),
                &PlacementTarget::default(),
                card_id("c2"),
                2,
            )
            .unwrap();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 2);
        assert_eq!(root[0].ordinal, 0);
        assert_eq!(root[1].ordinal, 1);
        assert!(matches!(
            &root[0].kind,
            BoardItemKind::Card { session, .. } if session.as_str() == "s1"
        ));
    }

    #[test]
    fn place_session_is_idempotent_for_same_session() {
        let mut layout = layout_with_default_board();
        layout
            .place_session(
                conn(),
                sess("s1"),
                &PlacementTarget::default(),
                card_id("c1"),
                1,
            )
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("s1"),
                &PlacementTarget::default(),
                card_id("c_dup"),
                2,
            )
            .unwrap();
        assert_eq!(layout.items.len(), 1);
        assert_eq!(layout.items[0].id.as_str(), "c1");
    }

    #[test]
    fn create_group_and_nested_cards_renumber() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .create_group(&board_id, None, 0, "auth work", card_id("g1"), 1)
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("s1"),
                &PlacementTarget {
                    board_id: Some(board_id.clone()),
                    parent_item_id: Some(card_id("g1")),
                    ordinal: None,
                },
                card_id("c1"),
                2,
            )
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("s2"),
                &PlacementTarget {
                    board_id: Some(board_id.clone()),
                    parent_item_id: Some(card_id("g1")),
                    ordinal: None,
                },
                card_id("c2"),
                3,
            )
            .unwrap();
        let kids = layout.children(&board_id, Some(&card_id("g1")));
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].ordinal, 0);
        assert_eq!(kids[1].ordinal, 1);
    }

    #[test]
    fn move_item_reparents_and_renumbers_both_sibling_sets() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        for (sid, cid) in [("s1", "c1"), ("s2", "c2"), ("s3", "c3")] {
            layout
                .place_session(
                    conn(),
                    sess(sid),
                    &PlacementTarget::default(),
                    card_id(cid),
                    1,
                )
                .unwrap();
        }
        layout
            .create_group(&board_id, None, 1, "grp", card_id("g1"), 1)
            .unwrap();
        layout
            .move_item(&card_id("c3"), &board_id, Some(card_id("g1")), 0)
            .unwrap();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 3);
        assert_eq!(root[0].ordinal, 0);
        assert_eq!(root[1].ordinal, 1);
        assert_eq!(root[2].ordinal, 2);
        let in_group = layout.children(&board_id, Some(&card_id("g1")));
        assert_eq!(in_group.len(), 1);
        assert_eq!(in_group[0].id.as_str(), "c3");
        assert_eq!(in_group[0].ordinal, 0);
    }

    #[test]
    fn move_group_into_descendant_is_rejected() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .create_group(&board_id, None, 0, "outer", card_id("g_outer"), 1)
            .unwrap();
        layout
            .create_group(
                &board_id,
                Some(card_id("g_outer")),
                0,
                "inner",
                card_id("g_inner"),
                2,
            )
            .unwrap();
        let err = layout
            .move_item(&card_id("g_outer"), &board_id, Some(card_id("g_inner")), 0)
            .unwrap_err();
        assert_eq!(err, BoardError::CycleDetected);
    }

    #[test]
    fn move_group_across_boards_carries_subtree() {
        let now = 1_700_000_000_000_i64;
        let ba = BoardId::new("bA");
        let bb = BoardId::new("bB");
        let mut layout = BoardLayout {
            boards: vec![
                Board {
                    id: ba.clone(),
                    name: "A".into(),
                    ordinal: 0,
                    created_at: now,
                    updated_at: now,
                },
                Board {
                    id: bb.clone(),
                    name: "B".into(),
                    ordinal: 1,
                    created_at: now,
                    updated_at: now,
                },
            ],
            items: vec![],
        };
        layout
            .create_group(&ba, None, 0, "g", card_id("g"), now)
            .unwrap();
        for c in ["x", "y"] {
            layout
                .place_session(
                    conn(),
                    sess(c),
                    &PlacementTarget {
                        board_id: Some(ba.clone()),
                        parent_item_id: Some(card_id("g")),
                        ordinal: None,
                    },
                    card_id(c),
                    now,
                )
                .unwrap();
        }
        // Move the group (with its subtree) to board B's root.
        layout.move_item(&card_id("g"), &bb, None, 0).unwrap();
        assert_eq!(layout.item(&card_id("g")).unwrap().board_id.as_str(), "bB");
        for c in ["x", "y"] {
            let item = layout.item(&card_id(c)).unwrap();
            assert_eq!(item.board_id.as_str(), "bB", "child {c} follows to board B");
            assert_eq!(item.parent_item_id.as_ref().unwrap().as_str(), "g");
        }
        assert_eq!(layout.children(&bb, None).len(), 1, "group on B");
        assert_eq!(
            layout.children(&bb, Some(&card_id("g"))).len(),
            2,
            "children visible under the group on B"
        );
        assert_eq!(layout.children(&ba, None).len(), 0, "board A emptied");
    }

    #[test]
    fn move_item_reorders_within_same_parent() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        for (sid, cid) in [("s1", "c1"), ("s2", "c2"), ("s3", "c3")] {
            layout
                .place_session(
                    conn(),
                    sess(sid),
                    &PlacementTarget::default(),
                    card_id(cid),
                    1,
                )
                .unwrap();
        }
        layout
            .move_item(&card_id("c1"), &board_id, None, 2)
            .unwrap();
        let order: Vec<&str> = layout
            .children(&board_id, None)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(order, vec!["c2", "c3", "c1"]);
        let ords: Vec<i32> = layout
            .children(&board_id, None)
            .iter()
            .map(|i| i.ordinal)
            .collect();
        assert_eq!(ords, vec![0, 1, 2]);
    }

    #[test]
    fn ungroup_reparents_children_to_group_slot() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .place_session(
                conn(),
                sess("before"),
                &PlacementTarget::default(),
                card_id("before"),
                1,
            )
            .unwrap();
        layout
            .create_group(&board_id, None, 1, "grp", card_id("g1"), 2)
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("in1"),
                &PlacementTarget {
                    board_id: Some(board_id.clone()),
                    parent_item_id: Some(card_id("g1")),
                    ordinal: None,
                },
                card_id("in1"),
                3,
            )
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("in2"),
                &PlacementTarget {
                    board_id: Some(board_id.clone()),
                    parent_item_id: Some(card_id("g1")),
                    ordinal: None,
                },
                card_id("in2"),
                4,
            )
            .unwrap();
        layout.ungroup(&card_id("g1")).unwrap();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 3);
        assert_eq!(root[0].id.as_str(), "before");
        assert_eq!(root[1].id.as_str(), "in1");
        assert_eq!(root[2].id.as_str(), "in2");
        assert_eq!(root[0].ordinal, 0);
        assert_eq!(root[1].ordinal, 1);
        assert_eq!(root[2].ordinal, 2);
        assert!(!layout.items.iter().any(|i| i.id.as_str() == "g1"));
    }

    #[test]
    fn ungroup_non_last_group_keeps_children_contiguous_before_trailing_siblings() {
        // Regression: a group that is NOT the last sibling and has >1 child. Its
        // children must land contiguously at the group's slot, with the trailing
        // sibling AFTER them — not interleaved. Layout: [A, grp{x,y}, B].
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .place_session(
                conn(),
                sess("A"),
                &PlacementTarget::default(),
                card_id("A"),
                1,
            )
            .unwrap();
        layout
            .create_group(&board_id, None, 1, "grp", card_id("g"), 1)
            .unwrap();
        layout
            .place_session(
                conn(),
                sess("B"),
                &PlacementTarget::default(),
                card_id("B"),
                1,
            )
            .unwrap();
        for c in ["x", "y"] {
            layout
                .place_session(
                    conn(),
                    sess(c),
                    &PlacementTarget {
                        board_id: Some(board_id.clone()),
                        parent_item_id: Some(card_id("g")),
                        ordinal: None,
                    },
                    card_id(c),
                    1,
                )
                .unwrap();
        }
        // Sanity: order is A, g, B at root before ungroup.
        let pre: Vec<&str> = layout
            .children(&board_id, None)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(pre, vec!["A", "g", "B"]);

        layout.ungroup(&card_id("g")).unwrap();

        let root: Vec<&str> = layout
            .children(&board_id, None)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(
            root,
            vec!["A", "x", "y", "B"],
            "children spliced at slot, B trails"
        );
        let ords: Vec<i32> = layout
            .children(&board_id, None)
            .iter()
            .map(|i| i.ordinal)
            .collect();
        assert_eq!(ords, vec![0, 1, 2, 3], "dense ordinals");
    }

    #[test]
    fn archive_keeps_group_row_and_sets_flag() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .create_group(&board_id, None, 0, "done", card_id("g1"), 1)
            .unwrap();
        layout.archive(&card_id("g1")).unwrap();
        let g = layout.item(&card_id("g1")).unwrap();
        assert!(matches!(
            g.kind,
            BoardItemKind::Group { archived: true, .. }
        ));
        assert_eq!(layout.items.len(), 1);
    }

    #[test]
    fn remove_session_drops_card_and_renumbers() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        for (sid, cid) in [("s1", "c1"), ("s2", "c2")] {
            layout
                .place_session(
                    conn(),
                    sess(sid),
                    &PlacementTarget::default(),
                    card_id(cid),
                    1,
                )
                .unwrap();
        }
        layout.remove_session(&conn(), &sess("s1")).unwrap();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].ordinal, 0);
        assert!(matches!(
            &root[0].kind,
            BoardItemKind::Card { session, .. } if session.as_str() == "s2"
        ));
    }

    #[test]
    fn rename_set_collapsed_set_color_on_group() {
        let mut layout = layout_with_default_board();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        layout
            .create_group(&board_id, None, 0, "old", card_id("g1"), 1)
            .unwrap();
        layout.rename(&card_id("g1"), "new").unwrap();
        layout.set_collapsed(&card_id("g1"), true).unwrap();
        layout.set_color(&card_id("g1"), "blue").unwrap();
        let g = layout.item(&card_id("g1")).unwrap();
        assert!(matches!(
            &g.kind,
            BoardItemKind::Group {
                name,
                color_token: Some(token),
                collapsed: true,
                archived: false,
            } if name == "new" && token == "blue"
        ));
    }
}
