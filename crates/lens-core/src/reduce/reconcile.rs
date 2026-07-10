//! D16 pending_user reconcile — pure precedence (1) pending_id (2) item_id (3) content.

use crate::domain::controls::PendingUserMessage;
use lens_client::sessions::PendingInput;

/// Drop at most one matching bubble (Consumed) / keep-or-drop each (Snapshot).
/// Returns true iff `pending` changed. PURE — no I/O, deterministic.
pub fn reconcile_pending_user(
    pending: &mut Vec<PendingUserMessage>,
    signal: ReconcileSignal<'_>,
) -> bool {
    match signal {
        ReconcileSignal::Consumed {
            cleared_pending_id,
            item_id,
            content,
        } => reconcile_consumed(pending, cleared_pending_id, item_id, content),
        ReconcileSignal::Snapshot {
            pending_inputs,
            trailing_user_item_ids_and_text,
        } => reconcile_snapshot(pending, pending_inputs, trailing_user_item_ids_and_text),
    }
}

pub enum ReconcileSignal<'a> {
    Consumed {
        cleared_pending_id: Option<&'a str>,
        item_id: &'a str,
        content: Option<&'a str>,
    },
    Snapshot {
        pending_inputs: &'a [PendingInput],
        trailing_user_item_ids_and_text: &'a [(String, String)],
    },
}

/// Per bubble walk id-keys 1→2→3; never branch on harness/native — a
/// native-terminal-down send has `store_item_id` set + `server_pending_id` None
/// and is handled by path 2 with no `is_native` check.
fn reconcile_consumed(
    pending: &mut Vec<PendingUserMessage>,
    cleared_pending_id: Option<&str>,
    item_id: &str,
    content: Option<&str>,
) -> bool {
    for (i, bubble) in pending.iter().enumerate() {
        if bubble.server_pending_id.is_some() {
            if bubble.server_pending_id.as_deref() == cleared_pending_id {
                pending.remove(i);
                return true;
            }
            continue;
        }
        if bubble.store_item_id.is_some() {
            if bubble.store_item_id.as_deref() == Some(item_id) {
                pending.remove(i);
                return true;
            }
            continue;
        }
    }
    if let Some(c) = content
        && let Some(i) = pending.iter().position(|b| {
            b.server_pending_id.is_none() && b.store_item_id.is_none() && b.content == c
        })
    {
        pending.remove(i);
        return true;
    }
    false
}

/// Signal B (§4 P3(b)): retain a bubble iff its `server_pending_id` is still listed
/// in `pending_inputs`. All other rows in the decision table drop.
fn reconcile_snapshot(
    pending: &mut Vec<PendingUserMessage>,
    pending_inputs: &[PendingInput],
    trailing_user_item_ids_and_text: &[(String, String)],
) -> bool {
    let still_pending: std::collections::HashSet<&str> = pending_inputs
        .iter()
        .map(|p| p.pending_id.as_str())
        .collect();
    let before = pending.len();
    pending.retain(|bubble| {
        bubble
            .server_pending_id
            .as_deref()
            .is_some_and(|sid| still_pending.contains(sid))
    });
    // trailing ids/text inform the spec's drop rows (2)/(3); with retain-only KEEP
    // semantics they are redundant — a landed bubble's server_pending_id is absent
    // from `pending_inputs`, so it is already dropped above.
    let _ = trailing_user_item_ids_and_text;
    pending.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bubble(
        pending_id: &str,
        server_pending_id: Option<&str>,
        store_item_id: Option<&str>,
        content: &str,
    ) -> PendingUserMessage {
        PendingUserMessage {
            pending_id: pending_id.into(),
            server_pending_id: server_pending_id.map(str::to_string),
            store_item_id: store_item_id.map(str::to_string),
            content: content.into(),
            created_at: 0,
        }
    }

    fn consumed(
        pending: &mut Vec<PendingUserMessage>,
        cleared_pending_id: Option<&str>,
        item_id: &str,
        content: Option<&str>,
    ) -> bool {
        reconcile_pending_user(
            pending,
            ReconcileSignal::Consumed {
                cleared_pending_id,
                item_id,
                content,
            },
        )
    }

    #[test]
    fn precedence_server_pending_id_wins() {
        let mut pending = vec![bubble("l1", Some("pend_a"), Some("msg_other"), "hi")];
        assert!(consumed(&mut pending, Some("pend_a"), "msg_ignored", None));
        assert!(pending.is_empty());
    }

    #[test]
    fn precedence_store_item_id_when_no_server_pending() {
        let mut pending = vec![bubble("l1", None, Some("msg_1"), "hi")];
        assert!(consumed(&mut pending, None, "msg_1", None));
        assert!(pending.is_empty());
    }

    #[test]
    fn precedence_content_match_is_defensive_floor() {
        let mut pending = vec![bubble("l1", None, None, "hello")];
        assert!(consumed(&mut pending, None, "msg_x", Some("hello")));
        assert!(pending.is_empty());
    }

    #[test]
    fn native_down_uses_item_id_not_harness_flag() {
        let mut pending = vec![bubble("l1", None, Some("msg_native"), "hey")];
        assert!(consumed(&mut pending, None, "msg_native", None));
        assert!(pending.is_empty());
    }

    #[test]
    fn snapshot_pending_inputs_keeps_still_pending_bubbles() {
        let mut pending = vec![
            bubble("l1", Some("pend_keep"), None, "still pending"),
            bubble("l2", Some("pend_gone"), None, "landed"),
            bubble("l3", None, Some("msg_1"), "landed by store id"),
        ];
        let inputs = vec![PendingInput {
            pending_id: "pend_keep".into(),
        }];
        let trailing = vec![("msg_1".into(), "landed by store id".into())];
        assert!(reconcile_pending_user(
            &mut pending,
            ReconcileSignal::Snapshot {
                pending_inputs: &inputs,
                trailing_user_item_ids_and_text: &trailing,
            },
        ));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pending_id, "l1");
        assert_eq!(pending[0].server_pending_id.as_deref(), Some("pend_keep"));
    }

    #[test]
    fn consumed_drops_at_most_one_bubble() {
        let mut pending = vec![
            bubble("l1", None, Some("msg_1"), "a"),
            bubble("l2", None, Some("msg_1"), "b"),
        ];
        assert!(consumed(&mut pending, None, "msg_1", None));
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn consumed_content_path_drops_fifo_oldest() {
        let mut pending = vec![
            bubble("old", None, None, "same"),
            bubble("new", None, None, "same"),
        ];
        assert!(consumed(&mut pending, None, "x", Some("same")));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pending_id, "new");
    }
}
