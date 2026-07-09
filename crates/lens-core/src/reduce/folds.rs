//! Session-field scalar folds + status/usage normalization (§4.1).

use crate::domain::{SandboxStatus, SessionState, SessionStatusValue, Todo, TodoStatus};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::event::TodoItemStatus;
use lens_client::stream::{SessionEvent, SessionStatusValue as WireStatus};
use smallvec::smallvec;

/// Map the 6-value wire status to the domain status (D-P1-8). Distinct types, same shape.
pub fn normalize_status(w: WireStatus) -> SessionStatusValue {
    match w {
        WireStatus::Idle => SessionStatusValue::Idle,
        WireStatus::Launching => SessionStatusValue::Launching,
        WireStatus::Running => SessionStatusValue::Running,
        WireStatus::Waiting => SessionStatusValue::Waiting,
        WireStatus::Failed => SessionStatusValue::Failed,
        WireStatus::Unknown => SessionStatusValue::Unknown,
    }
}

fn map_todo_status(w: TodoItemStatus) -> TodoStatus {
    match w {
        TodoItemStatus::Pending => TodoStatus::Pending,
        TodoItemStatus::InProgress => TodoStatus::InProgress,
        TodoItemStatus::Completed => TodoStatus::Completed,
        TodoItemStatus::Unknown => TodoStatus::Unknown, // REVIEW#8: preserve churn signal
    }
}

/// Live `session.usage` → canonical scalars (D-P1-9).
pub(crate) fn fold_usage(
    state: &mut SessionState,
    context_tokens: Option<i64>,
    context_window: Option<i64>,
    total_cost_usd: Option<f64>,
) -> Updates {
    if let Some(ct) = context_tokens {
        state.last_total_tokens = Some(ct.max(0) as u64);
    }
    if let Some(cw) = context_window {
        state.context_window = Some(cw.max(0) as u64);
    }
    if let Some(cost) = total_cost_usd {
        state.cumulative_cost.total_cost_usd = Some(cost);
    }
    smallvec![StreamUpdate::UsageChanged]
}

/// The non-item, non-usage, non-presence, non-child session-field arms. Returns
/// `None` for arms handled elsewhere so `reduce` can route them.
pub(crate) fn fold_session_field(state: &mut SessionState, ev: &SessionEvent) -> Option<Updates> {
    Some(match ev {
        SessionEvent::Status { status, .. } => {
            state.status = normalize_status(*status);
            smallvec![StreamUpdate::StatusChanged]
        }
        SessionEvent::Model { model } => {
            state.llm_model = Some(model.clone());
            smallvec![StreamUpdate::ModelChanged]
        }
        SessionEvent::ReasoningEffort { reasoning_effort } => {
            state.reasoning_effort = reasoning_effort.clone();
            smallvec![StreamUpdate::ReasoningEffortChanged]
        }
        SessionEvent::ModelOptions => smallvec![StreamUpdate::ModelOptionsChanged],
        SessionEvent::Todos { todos } => {
            state.todos = todos
                .iter()
                .map(|t| Todo {
                    content: t.content().to_string(),
                    status: map_todo_status(t.status()),
                    active_form: t.active_form().to_string(),
                })
                .collect();
            smallvec![StreamUpdate::TodosChanged]
        }
        SessionEvent::Skills => {
            // P1-DECISION: lens-client `session.skills` wrapper is a unit variant (payload
            // dropped) — no names available. Mark changed; leave `state.skills` untouched.
            smallvec![StreamUpdate::SkillsChanged]
        }
        SessionEvent::SandboxStatus { stage, error } => {
            state.sandbox_status = Some(SandboxStatus {
                stage: stage.clone(),
                detail: error.clone(),
            });
            smallvec![StreamUpdate::SandboxChanged]
        }
        SessionEvent::TerminalPending { pending } => {
            state.terminal_pending = *pending;
            smallvec![StreamUpdate::TerminalPendingChanged]
        }
        // Marker-only (D-P1-19): no P1 field home / liveness only.
        SessionEvent::TerminalActivity { .. } => smallvec![StreamUpdate::TerminalPendingChanged],
        SessionEvent::ChangedFilesInvalidated { .. }
        | SessionEvent::Interrupted { .. }
        | SessionEvent::Superseded { .. }
        | SessionEvent::InputConsumed { .. } => return Some(smallvec![]),
        // REVIEW#9: child spawn — D-P1-18 marker (no P1 field home; §9 owns child topology).
        SessionEvent::Created { .. } => smallvec![StreamUpdate::ChildSessionChanged],
        SessionEvent::ResourceCreated | SessionEvent::ResourceDeleted { .. } => {
            smallvec![StreamUpdate::ResourcesChanged] // D-P1-4
        }
        SessionEvent::Heartbeat { .. } => return Some(smallvec![]),
        SessionEvent::Usage {
            context_tokens,
            context_window,
            total_cost_usd,
        } => fold_usage(state, *context_tokens, *context_window, *total_cost_usd),
        // Handled elsewhere:
        SessionEvent::Presence { .. }
        | SessionEvent::ChildSessionUpdated { .. }
        | SessionEvent::AgentChanged { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::clock::ManualClock;
    use crate::domain::{
        AgentId, ConnectionId, SessionId, SessionState, SessionStatusValue, TodoStatus,
    };
    use crate::reduce::testutil::parse_session;
    use crate::reduce::{StreamUpdate, reduce};
    use lens_client::stream::{ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus};

    fn st() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }
    fn clock() -> ManualClock {
        ManualClock::new(1_700_000_000_000)
    }

    #[test]
    fn status_running_folds_and_marks() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Status {
                status: WireStatus::Running,
                response_id: None,
                background_task_count: None,
            }),
            &clock(),
        );
        assert_eq!(s.status, SessionStatusValue::Running);
        assert_eq!(&u[..], &[StreamUpdate::StatusChanged]);
    }

    #[test]
    fn model_and_effort_fold() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Model {
                model: "opus".into(),
            }),
            &clock(),
        );
        assert_eq!(s.llm_model.as_deref(), Some("opus"));
        reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::ReasoningEffort {
                reasoning_effort: Some("high".into()),
            }),
            &clock(),
        );
        assert_eq!(s.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn todos_replace_wholesale() {
        let mut s = st();
        // REVIEW#10: `TodoItem` has private fields — build the event from bytes via the
        // `parse_session` shared helper (decode_all seam), not a hand-built wrapper.
        let ev = parse_session(
            "session.todos",
            r#"{"conversation_id":"c","todos":[{"content":"Fix bug","status":"in_progress","activeForm":"Fixing bug"}]}"#,
        );
        let u = reduce(&mut s, &ev, &clock());
        assert_eq!(s.todos.len(), 1);
        assert_eq!(s.todos[0].content, "Fix bug");
        assert_eq!(s.todos[0].status, TodoStatus::InProgress);
        assert_eq!(&u[..], &[StreamUpdate::TodosChanged]);
    }

    #[test]
    fn terminal_pending_folds() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::TerminalPending { pending: true }),
            &clock(),
        );
        assert!(s.terminal_pending);
    }

    #[test]
    fn usage_folds_into_canonical_cost() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Usage {
                context_tokens: Some(1200),
                context_window: Some(200_000),
                total_cost_usd: Some(0.42),
            }),
            &clock(),
        );
        assert_eq!(s.last_total_tokens, Some(1200));
        assert_eq!(s.context_window, Some(200_000));
        assert_eq!(s.cumulative_cost.total_cost_usd, Some(0.42));
        assert_eq!(&u[..], &[StreamUpdate::UsageChanged]);
    }

    #[test]
    fn usage_negative_wire_ints_never_panic() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Usage {
                context_tokens: Some(-5),
                context_window: None,
                total_cost_usd: None,
            }),
            &clock(),
        );
        assert_eq!(s.last_total_tokens, Some(0)); // clamped, total
    }
}
