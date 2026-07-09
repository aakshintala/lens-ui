//! Session-field scalar folds + status/usage normalization (§4.1).

use crate::domain::{
    Elicitation, ElicitationId, ElicitationParams as DomainElicParams, SandboxStatus, SessionState,
    SessionStatusValue, Todo, TodoStatus,
};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::event::TodoItemStatus;
use lens_client::stream::{ResponseEvent, SessionEvent, SessionStatusValue as WireStatus};
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

/// Session-field scalar/collection folds. Returns `None` only for arms routed elsewhere.
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
        SessionEvent::Presence { viewers } => {
            state.presence = viewers
                .iter()
                .filter_map(|v| {
                    v.user_id().map(|uid| crate::domain::PresenceViewer {
                        user_id: uid.to_string(),
                        joined_at: String::new(), // P1-DECISION D-P1-5: wrapper drops these
                        idle: false,
                    })
                })
                .collect();
            smallvec![StreamUpdate::PresenceChanged]
        }
        SessionEvent::AgentChanged {
            agent_id,
            agent_name,
        } => {
            state.agent_id = crate::domain::AgentId::new(agent_id.clone());
            state.agent_name = Some(agent_name.clone());
            state.stream.current_agent = Some(agent_name.clone());
            // Transcript marker pushed in Task 8 (needs push_item); scalar fold here.
            smallvec![StreamUpdate::AgentChanged]
        }
        SessionEvent::ChildSessionUpdated { .. } => smallvec![StreamUpdate::ChildSessionChanged],
    })
}

/// Response lifecycle markers + elicitation folds. Returns `None` for item-producing /
/// scratch-routing arms handled in `reduce` or later tasks.
pub(crate) fn fold_response_marker(
    state: &mut SessionState,
    ev: &ResponseEvent,
) -> Option<Updates> {
    Some(match ev {
        ResponseEvent::InProgress => smallvec![StreamUpdate::StatusChanged], // P1-DECISION: liveness marker
        ResponseEvent::Failed | ResponseEvent::Incomplete | ResponseEvent::Cancelled => {
            smallvec![]
        }
        ResponseEvent::CompactionInProgress | ResponseEvent::CompactionFailed => smallvec![],
        // REVIEW#4: fold response.error into the `last_task_error` scalar banner (ErrorInfo,
        // "present iff Failed"). NOT a transcript item — the byte-verified error-item path is
        // `OutputItemDone(Error)`; pushing from both would double-insert. This preserves the
        // external error data without that hazard.
        ResponseEvent::Error { code, message, .. } => {
            state.last_task_error = Some(crate::domain::ErrorInfo {
                code: code.clone(),
                message: message.clone(),
            });
            smallvec![StreamUpdate::StatusChanged]
        }
        ResponseEvent::ElicitationRequest {
            elicitation_id,
            params,
        } => {
            state.pending_elicitations.push(Elicitation {
                id: ElicitationId::new(elicitation_id.clone()),
                target_session_id: state.id.clone(),
                params: DomainElicParams {
                    mode: params.mode().to_string(),
                    message: params.message().to_string(),
                    url: params.url().map(str::to_string),
                    phase: params.phase().map(str::to_string),
                    policy_name: params.policy_name().map(str::to_string),
                    content_preview: params.content_preview().map(str::to_string),
                },
            });
            smallvec![StreamUpdate::ElicitationsChanged]
        }
        ResponseEvent::ElicitationResolved { elicitation_id } => {
            state
                .pending_elicitations
                .retain(|e| e.id.as_str() != elicitation_id);
            smallvec![StreamUpdate::ElicitationsChanged]
        }
        // item-producing / scratch-finalizing arms handled in Task 7/8:
        ResponseEvent::OutputItemDone { .. }
        | ResponseEvent::Completed
        | ResponseEvent::ReasoningClosed { .. }
        | ResponseEvent::CompactionCompleted { .. }
        | ResponseEvent::OutputTextDelta { .. }
        | ResponseEvent::ReasoningStarted
        | ResponseEvent::ReasoningTextDelta { .. }
        | ResponseEvent::ReasoningSummaryTextDelta { .. } => return None,
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

    #[test]
    fn presence_fills_user_id_only() {
        let mut s = st();
        // build via bytes so the private wrapper is populated
        let ev = parse_session("session.presence", r#"{"viewers":[{"user_id":"u_1"}]}"#);
        let u = reduce(&mut s, &ev, &clock());
        assert_eq!(s.presence.len(), 1);
        assert_eq!(s.presence[0].user_id, "u_1");
        assert_eq!(s.presence[0].joined_at, ""); // P1-DECISION: wrapper drops joined_at/idle
        assert!(!s.presence[0].idle);
        assert_eq!(&u[..], &[StreamUpdate::PresenceChanged]);
    }

    #[test]
    fn elicitation_request_then_resolved() {
        use crate::reduce::testutil::parse_response;
        use lens_client::stream::ResponseEvent;

        let mut s = st();
        let req = parse_response(
            "response.elicitation_request",
            r#"{"elicitation_id":"e1","params":{"mode":"url","message":"ok?","url":"/a"}}"#,
        );
        reduce(&mut s, &req, &clock());
        assert_eq!(s.pending_elicitations.len(), 1);
        assert_eq!(s.pending_elicitations[0].id.as_str(), "e1");
        let res = ServerStreamEvent::Response(ResponseEvent::ElicitationResolved {
            elicitation_id: "e1".into(),
        });
        let u = reduce(&mut s, &res, &clock());
        assert!(s.pending_elicitations.is_empty());
        assert_eq!(&u[..], &[StreamUpdate::ElicitationsChanged]);
    }

    #[test]
    fn agent_changed_updates_scalars() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::AgentChanged {
                agent_id: "ag_2".into(),
                agent_name: "debby".into(),
            }),
            &clock(),
        );
        assert_eq!(s.agent_id.as_str(), "ag_2");
        assert_eq!(s.agent_name.as_deref(), Some("debby"));
        assert_eq!(s.stream.current_agent.as_deref(), Some("debby"));
        assert!(u.contains(&StreamUpdate::AgentChanged));
    }
}
