//! §4.3 pure render transforms over `&[Item]`.

use crate::domain::item::{Item, ItemKind};
use std::collections::BTreeMap;

pub fn hide_reasoning(items: &[Item]) -> Vec<&Item> {
    items
        .iter()
        .filter(|i| !matches!(i.kind, ItemKind::Reasoning { .. }))
        .collect()
}

pub fn with_agent_changed_markers(items: &[Item], keep: bool) -> Vec<&Item> {
    items
        .iter()
        .filter(|i| keep || !matches!(i.kind, ItemKind::AgentChanged { .. }))
        .collect()
}

pub fn only_agent<'a>(items: &'a [Item], agent: &str) -> Vec<&'a Item> {
    items
        .iter()
        .filter(|i| i.ctx.agent.as_deref() == Some(agent))
        .collect()
}

pub fn by_depth(items: &[Item]) -> BTreeMap<u32, Vec<&Item>> {
    let mut m: BTreeMap<u32, Vec<&Item>> = BTreeMap::new();
    for i in items {
        m.entry(i.ctx.depth).or_default().push(i);
    }
    m
}

/// Coalesce adjacent assistant `Message` items into one for display. Returns owned
/// clones (it synthesizes merged content) — the only transform that does.
pub fn merge_text_for_display(items: &[Item]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for it in items {
        if let (Some(prev), ItemKind::Message { role, content }) = (out.last_mut(), &it.kind)
            && let ItemKind::Message {
                role: prole,
                content: pcontent,
            } = &mut prev.kind
            && prole == role
        {
            pcontent.extend(content.iter().cloned());
            continue;
        }
        out.push(it.clone());
    }
    out
}

/// DEFERRED (D-P1: sub-agent topology is §9). Identity passthrough in P1.
pub fn flatten_sub_agents(items: &[Item]) -> Vec<&Item> {
    items.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, ItemId};
    use crate::domain::item::{BlockContext, ContentBlock};
    use crate::domain::scalars::Role;
    use serde_json::Value;

    fn ctx() -> BlockContext {
        BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        }
    }

    fn msg_item(id: &str, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx(),
            created_at: 0,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "output_text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        }
    }

    fn reasoning_item(id: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx(),
            created_at: 0,
            kind: ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: String::new(),
                encrypted: false,
            },
        }
    }

    fn agent_changed_item(id: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx(),
            created_at: 0,
            kind: ItemKind::AgentChanged {
                from: AgentId::new("a"),
                to: AgentId::new("b"),
                at: 0,
            },
        }
    }

    #[test]
    fn hide_reasoning_drops_reasoning_items() {
        let items = vec![
            msg_item("m1", "hi"),
            reasoning_item("r1"),
            msg_item("m2", "bye"),
        ];
        let out = hide_reasoning(&items);
        assert_eq!(out.len(), 2);
        assert!(
            out.iter()
                .all(|i| !matches!(i.kind, ItemKind::Reasoning { .. }))
        );
    }

    #[test]
    fn only_agent_filters_by_ctx() {
        let mut a = msg_item("m1", "x");
        a.ctx.agent = Some("coder".into());
        let mut b = msg_item("m2", "y");
        b.ctx.agent = Some("researcher".into());
        let items = vec![a, b];
        let out = only_agent(&items, "coder");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id.as_str(), "m1");
    }

    #[test]
    fn with_agent_changed_markers_can_drop_them() {
        let items = vec![
            msg_item("m1", "x"),
            agent_changed_item("ac1"),
            msg_item("m2", "y"),
        ];
        assert_eq!(with_agent_changed_markers(&items, true).len(), 3);
        assert_eq!(with_agent_changed_markers(&items, false).len(), 2);
    }
}
