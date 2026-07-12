//! D16 pending_user reconcile — pure precedence (1) pending_id (2) item_id (3) content.
//! D28 held-bubble three-way reconcile on in-actor reconnect.

use crate::domain::controls::PendingUserMessage;
use crate::domain::item::{Item, ItemKind};
use crate::domain::scalars::Role;
use lens_client::sessions::PendingInput;

/// A held send confirmed lost after reconnect catch-up (path 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostSend {
    pub lens_pending_id: String,
    pub content: String,
}

/// Concatenate `input_text`/`output_text` block texts in order (F10).
pub(crate) fn user_text(item: &Item) -> Option<String> {
    match &item.kind {
        ItemKind::Message {
            role: Role::User,
            content,
        } => {
            let text: String = content
                .iter()
                .filter(|b| b.kind == "input_text" || b.kind == "output_text")
                .filter_map(|b| b.text.as_deref())
                .collect();
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

fn is_held(bubble: &PendingUserMessage) -> bool {
    bubble.server_pending_id.is_none() && bubble.store_item_id.is_none()
}

fn count_held_with_content(bubbles: &[PendingUserMessage], content: &str) -> usize {
    bubbles
        .iter()
        .filter(|b| is_held(b) && b.content == content)
        .count()
}

/// D28: three-way held-bubble reconcile. Path 1 (snapshot `pending_inputs` → stamp+keep),
/// path 2 (catch-up NEW user rows → drop), path 3 (else → SendLost). Uniqueness-only:
/// ambiguity on either side for path 1 → keep unstamped; ambiguous path 2 → SendLost
/// (visible). Path order 1→2→3 (F8).
///
/// **Residual:** a rare unrelated identical message landing in the same disconnect window
/// can still be silently dropped on path 2 (unique content match without idempotency key).
/// Robust fix deferred to omnigent client-message-id.
pub fn reconcile_held_landed(
    pending: &mut Vec<PendingUserMessage>,
    snapshot_pending_inputs: &[PendingInput],
    catchup_new_user_contents: &[(String, i64)],
) -> Vec<LostSend> {
    // i64 is catch-up wall-clock, unused; omnigent /items has no per-item timestamp — temporal screen deferred to client-message-id.
    let mut consumed_inputs = vec![false; snapshot_pending_inputs.len()];
    let mut consumed_deltas = vec![false; catchup_new_user_contents.len()];
    let mut lost = Vec::new();

    let mut remaining = std::mem::take(pending);
    let mut output = Vec::with_capacity(remaining.len());

    while let Some(bubble) = remaining.first().cloned() {
        remaining.remove(0);

        if !is_held(&bubble) {
            output.push(bubble);
            continue;
        }

        let c = bubble.content.as_str();
        let held_count =
            count_held_with_content(&output, c) + count_held_with_content(&remaining, c) + 1;

        let avail_input_indices: Vec<usize> = snapshot_pending_inputs
            .iter()
            .enumerate()
            .filter(|(i, p)| !consumed_inputs[*i] && p.content.as_deref() == Some(c))
            .map(|(i, _)| i)
            .collect();

        // Path 1: two-sided unique match → stamp + keep.
        if held_count == 1 && avail_input_indices.len() == 1 {
            let idx = avail_input_indices[0];
            consumed_inputs[idx] = true;
            let mut stamped = bubble;
            stamped.server_pending_id = Some(snapshot_pending_inputs[idx].pending_id.clone());
            output.push(stamped);
            continue;
        }

        // Ambiguity on either side for path 1 → keep unstamped (re-evaluated next reconnect).
        if held_count > 1 || avail_input_indices.len() > 1 {
            output.push(bubble);
            continue;
        }

        let avail_delta_indices: Vec<usize> = catchup_new_user_contents
            .iter()
            .enumerate()
            .filter(|(i, (text, _))| !consumed_deltas[*i] && text == c)
            .map(|(i, _)| i)
            .collect();

        // Path 2: unique delta → silent drop (landed).
        if avail_delta_indices.len() == 1 {
            consumed_deltas[avail_delta_indices[0]] = true;
            continue;
        }

        // Path 3: lost (includes ambiguous path 2 or no match).
        lost.push(LostSend {
            lens_pending_id: bubble.pending_id.clone(),
            content: bubble.content.clone(),
        });
    }

    *pending = output;
    lost
}

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
        ReconcileSignal::Snapshot { pending_inputs } => reconcile_snapshot(pending, pending_inputs),
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
    // Path-3 (content) runs only for reserved enriched/replayed signals (content:Some);
    // live `session.input.consumed` passes content:None so this is inert. Semantics for
    // content:Some: id-matches across the whole vec take precedence over any content match
    // (a live id-drop preempts content); among both-None bubbles, drop the FIFO-oldest
    // equal-content bubble.
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

/// Signal B (§4 P3(b)): retain a bubble iff (a) both ids are None — a held/unacked
/// draft whose POST failed without stamping — or (b) its `server_pending_id` is still
/// listed in `pending_inputs`. Stamped bubbles whose server id is absent from the
/// snapshot drop (landed or confirmed-lost). Full landed-vs-lost dedup is deferred
/// to SendLost (P3-3); never silently drop typed send text.
fn reconcile_snapshot(
    pending: &mut Vec<PendingUserMessage>,
    pending_inputs: &[PendingInput],
) -> bool {
    let still_pending: std::collections::HashSet<&str> = pending_inputs
        .iter()
        .map(|p| p.pending_id.as_str())
        .collect();
    let before = pending.len();
    pending.retain(|bubble| {
        // Held/unacked draft (both server ids None): a Table B "keep" bubble whose POST
        // failed without stamping an id. NOT landed and NOT confirmed-lost — retain so the
        // text stays recoverable (future composer restores it). See design: never silently
        // drop typed send text. (Full landed-vs-lost dedup = deferred SendLost, P3-3.)
        let held = bubble.server_pending_id.is_none() && bubble.store_item_id.is_none();
        held || bubble
            .server_pending_id
            .as_deref()
            .is_some_and(|sid| still_pending.contains(sid))
    });
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

    fn input(id: &str, content: Option<&str>) -> PendingInput {
        PendingInput {
            pending_id: id.into(),
            content: content.map(str::to_string),
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
        let mut pending = vec![
            bubble("a", Some("pend_a"), None, "from server pending"),
            bubble("b", None, Some("msg_1"), "from store item"),
        ];
        assert!(consumed(&mut pending, Some("pend_a"), "msg_1", None));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pending_id, "b");
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
    fn snapshot_retains_held_both_ids_none_bubble() {
        let mut pending = vec![
            bubble("held", None, None, "typed but unacked"),
            bubble("stamped_gone", Some("pend_gone"), None, "landed"),
        ];
        let inputs: Vec<PendingInput> = vec![];
        assert!(reconcile_pending_user(
            &mut pending,
            ReconcileSignal::Snapshot {
                pending_inputs: &inputs,
            },
        ));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pending_id, "held");
        assert!(pending[0].server_pending_id.is_none());
        assert!(pending[0].store_item_id.is_none());
    }

    #[test]
    fn snapshot_pending_inputs_keeps_still_pending_bubbles() {
        let mut pending = vec![
            bubble("l1", Some("pend_keep"), None, "still pending"),
            bubble("l2", Some("pend_gone"), None, "landed"),
            bubble("l3", None, Some("msg_1"), "landed by store id"),
        ];
        let inputs = vec![input("pend_keep", None)];
        assert!(reconcile_pending_user(
            &mut pending,
            ReconcileSignal::Snapshot {
                pending_inputs: &inputs,
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

    #[test]
    fn held_matching_unique_pending_input_is_stamped_and_kept() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let inputs = vec![input("p9", Some("hi"))];
        let lost = reconcile_held_landed(&mut pending, &inputs, &[]);
        assert!(lost.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].server_pending_id.as_deref(), Some("p9"));
    }

    #[test]
    fn held_landed_in_catchup_delta_dropped() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let lost = reconcile_held_landed(&mut pending, &[], &[("hi".into(), 200)]);
        assert!(lost.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn held_absent_everywhere_is_lost() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let lost = reconcile_held_landed(&mut pending, &[], &[]);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].lens_pending_id, "l1");
        assert_eq!(lost[0].content, "hi");
        assert!(pending.is_empty());
    }

    #[test]
    fn path1_precedes_path2() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let inputs = vec![input("p9", Some("hi"))];
        let lost = reconcile_held_landed(&mut pending, &inputs, &[("hi".into(), 200)]);
        assert!(lost.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].server_pending_id.as_deref(), Some("p9"));
    }

    #[test]
    fn duplicate_pending_input_content_left_unstamped() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let inputs = vec![input("p1", Some("hi")), input("p2", Some("hi"))];
        let lost = reconcile_held_landed(&mut pending, &inputs, &[]);
        assert!(lost.is_empty());
        assert_eq!(pending.len(), 1);
        assert!(pending[0].server_pending_id.is_none());
    }

    #[test]
    fn ambiguous_catchup_delta_falls_to_send_lost() {
        let mut pending = vec![bubble("l1", None, None, "hi")];
        let lost =
            reconcile_held_landed(&mut pending, &[], &[("hi".into(), 100), ("hi".into(), 200)]);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].content, "hi");
        assert!(pending.is_empty());
    }

    #[test]
    fn stamped_bubble_untouched() {
        let mut pending = vec![bubble("l1", Some("pend_x"), None, "hi")];
        let lost = reconcile_held_landed(&mut pending, &[], &[]);
        assert!(lost.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].server_pending_id.as_deref(), Some("pend_x"));
    }

    #[test]
    fn duplicate_held_bubbles_one_unique_input_none_stamped() {
        let mut pending = vec![
            bubble("l1", None, None, "hi"),
            bubble("l2", None, None, "hi"),
        ];
        let inputs = vec![input("p9", Some("hi"))];
        let lost = reconcile_held_landed(&mut pending, &inputs, &[]);
        assert!(lost.is_empty());
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|b| b.server_pending_id.is_none()));
    }
}
