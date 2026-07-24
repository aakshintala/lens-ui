//! Session-field scalar folds + status/usage normalization (§4.1).

use crate::clock::Clock;
use crate::domain::ids::TerminalId;
use crate::domain::{
    Elicitation, ElicitationId, ElicitationParams as DomainElicParams, ResponseId, SandboxStatus,
    SessionState, SessionStatusValue, Todo, TodoStatus,
};
use crate::reduce::items;
use crate::reduce::reconcile::{ReconcileSignal, reconcile_pending_user};
use crate::reduce::{StreamUpdate, Updates};
use lens_client::stream::event::TodoItemStatus;
use lens_client::stream::{ResponseEvent, SessionEvent, SessionStatusValue as WireStatus};
use smallvec::smallvec;
use std::sync::Arc;

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
    smallvec![
        StreamUpdate::UsageChanged(state.cumulative_cost.clone()),
        StreamUpdate::LastTokensChanged(state.last_total_tokens),
        StreamUpdate::ContextWindowChanged(state.context_window),
    ]
}

/// Session-field scalar/collection folds. Returns `None` only for arms routed elsewhere.
pub(crate) fn fold_session_field(
    state: &mut SessionState,
    ev: &SessionEvent,
    clock: &dyn Clock,
) -> Option<Updates> {
    Some(match ev {
        SessionEvent::Status { status, .. } => {
            let normalized = normalize_status(*status);
            state.status = normalized;
            let cleared_error = if normalized != SessionStatusValue::Failed {
                let had_error = state.last_task_error.is_some();
                state.last_task_error = None;
                had_error
            } else {
                false
            };
            if cleared_error {
                smallvec![
                    StreamUpdate::StatusChanged(normalized),
                    StreamUpdate::LastTaskErrorChanged(state.last_task_error.clone()),
                ]
            } else {
                smallvec![StreamUpdate::StatusChanged(normalized)]
            }
        }
        SessionEvent::Model { model } => {
            state.llm_model = Some(model.clone());
            smallvec![StreamUpdate::ModelChanged {
                llm_model: state.llm_model.clone(),
                model_override: state.model_override.clone(),
            }]
        }
        SessionEvent::ReasoningEffort { reasoning_effort } => {
            state.reasoning_effort = reasoning_effort.clone();
            smallvec![StreamUpdate::ReasoningEffortChanged(
                state.reasoning_effort.clone()
            )]
        }
        SessionEvent::ModelOptions => smallvec![StreamUpdate::ModelOptionsChanged(
            state.model_options.clone()
        )],
        SessionEvent::Todos { todos } => {
            state.todos = todos
                .iter()
                .map(|t| Todo {
                    content: t.content().to_string(),
                    status: map_todo_status(t.status()),
                    active_form: t.active_form().to_string(),
                })
                .collect();
            smallvec![StreamUpdate::TodosChanged(state.todos.clone())]
        }
        SessionEvent::Skills => {
            // P1-DECISION: lens-client `session.skills` wrapper is a unit variant (payload
            // dropped) — no names available. Mark changed; leave `state.skills` untouched.
            smallvec![StreamUpdate::SkillsChanged(state.skills.clone())]
        }
        SessionEvent::SandboxStatus { stage, error } => {
            state.sandbox_status = Some(SandboxStatus {
                stage: stage.clone(),
                detail: error.clone(),
            });
            smallvec![StreamUpdate::SandboxChanged(state.sandbox_status.clone())]
        }
        SessionEvent::TerminalPending { pending } => {
            state.terminal_pending = *pending;
            smallvec![StreamUpdate::TerminalPendingChanged(state.terminal_pending)]
        }
        // Marker-only (D-P1-19): no P1 field home / liveness only.
        // `McpStartup` is a 0.5.0 addition (per-MCP-server startup progress) with no
        // state-model field home yet — marker-only until the store designs a surface.
        SessionEvent::TerminalActivity { .. }
        | SessionEvent::ChangedFilesInvalidated { .. }
        | SessionEvent::Interrupted { .. }
        | SessionEvent::McpStartup { .. } => return Some(smallvec![]),
        SessionEvent::Superseded {
            target_conversation_id,
            reason,
            ..
        } => smallvec![StreamUpdate::Superseded {
            target_conversation_id: target_conversation_id.clone(),
            reason: reason.clone(),
        }],
        SessionEvent::InputConsumed {
            item_id,
            item_type: _,
            cleared_pending_id,
        } => {
            let mut pending = std::mem::take(&mut state.pending_user);
            let changed = reconcile_pending_user(
                &mut pending,
                ReconcileSignal::Consumed {
                    cleared_pending_id: cleared_pending_id.as_deref(),
                    item_id,
                    content: None, // live event payload not required for (1)/(2)
                },
            );
            state.pending_user = pending;
            if changed {
                smallvec![StreamUpdate::PendingUserChanged(state.pending_user.clone())]
            } else {
                smallvec![]
            }
        }
        // REVIEW#9: child spawn — D-P1-18 marker (no P1 field home; §9 owns child topology).
        SessionEvent::Created { .. } => smallvec![StreamUpdate::ChildSessionChanged],
        SessionEvent::ResourceCreated {
            resource_id,
            resource_type,
            terminal_name,
            session_key,
        } => {
            if resource_type == "terminal" {
                if let (Some(terminal_name), Some(session_key)) = (terminal_name, session_key) {
                    smallvec![StreamUpdate::TerminalResourceCreated {
                        terminal_id: TerminalId::new(resource_id),
                        terminal_name: terminal_name.clone(),
                        session_key: session_key.clone(),
                        session_id: state.id.clone(),
                    }]
                } else {
                    smallvec![StreamUpdate::ResourcesChanged]
                }
            } else {
                smallvec![StreamUpdate::ResourcesChanged]
            }
        }
        SessionEvent::ResourceDeleted {
            resource_id,
            resource_type,
            ..
        } => {
            if resource_type == "terminal" {
                smallvec![StreamUpdate::TerminalResourceDeleted {
                    terminal_id: TerminalId::new(resource_id),
                }]
            } else {
                smallvec![StreamUpdate::ResourcesChanged]
            }
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
            smallvec![StreamUpdate::PresenceChanged(state.presence.clone())]
        }
        SessionEvent::AgentChanged {
            agent_id,
            agent_name,
        } => {
            let prev_agent = state.stream.current_agent.clone();
            let from = state.agent_id.clone();
            let to = crate::domain::AgentId::new(agent_id.clone());
            state.agent_id = to.clone();
            state.agent_name = Some(agent_name.clone());
            state.stream.current_agent = Some(agent_name.clone());
            let mut u = items::push_agent_changed(state, from, to, clock);
            u.push(StreamUpdate::AgentChanged {
                agent_id: state.agent_id.clone(),
                agent_name: state.agent_name.clone(),
            });
            if state.stream.current_agent != prev_agent {
                u.push(StreamUpdate::ScratchChanged(Arc::new(state.stream.clone())));
            }
            u
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
        // T-0: liveness marker AND active-response tracking. The wire `response_id`
        // (nullable) becomes `state.active_response`; terminal completions clear it in
        // `reduce` (see the Completed/Incomplete/Cancelled/Failed arms).
        ResponseEvent::InProgress { response_id } => {
            let active = ResponseId::from_wire(response_id.as_deref());
            state.active_response = active.clone();
            smallvec![
                StreamUpdate::StatusChanged(state.status),
                StreamUpdate::ActiveResponseChanged(active),
            ]
        }
        ResponseEvent::CompactionInProgress | ResponseEvent::CompactionFailed => smallvec![],
        // 0.5.0 addition: a native policy DENY was surfaced. Observational, no state-model
        // field home yet — marker-only until the store designs a surface.
        ResponseEvent::PolicyDenied { .. } => smallvec![],
        // REVIEW#4: fold response.error into the `last_task_error` scalar banner (ErrorInfo,
        // "present iff Failed"). NOT a transcript item — the byte-verified error-item path is
        // `OutputItemDone(Error)`; pushing from both would double-insert. This preserves the
        // external error data without that hazard.
        ResponseEvent::Error { code, message, .. } => {
            state.last_task_error = Some(crate::domain::ErrorInfo {
                code: code.clone(),
                message: message.clone(),
            });
            smallvec![
                StreamUpdate::StatusChanged(state.status),
                StreamUpdate::LastTaskErrorChanged(state.last_task_error.clone()),
            ]
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
            smallvec![StreamUpdate::ElicitationsChanged(
                state.pending_elicitations.clone()
            )]
        }
        ResponseEvent::ElicitationResolved { elicitation_id } => {
            state
                .pending_elicitations
                .retain(|e| e.id.as_str() != elicitation_id);
            smallvec![StreamUpdate::ElicitationsChanged(
                state.pending_elicitations.clone()
            )]
        }
        // item-producing / scratch-routing arms handled in `reduce`. The terminal
        // completions discard open scratch there; Incomplete/Cancelled also bump the Ready
        // counter, Failed does not (surfaced via Wave::Failed) — see the match arms.
        ResponseEvent::OutputItemDone { .. }
        | ResponseEvent::Completed
        | ResponseEvent::Failed
        | ResponseEvent::Incomplete
        | ResponseEvent::Cancelled
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
    use crate::domain::ids::TerminalId;
    use crate::domain::{
        AgentId, ConnectionId, SessionId, SessionState, SessionStatusValue, TodoStatus,
    };
    use crate::reduce::testutil::parse_session;
    use crate::reduce::{RetireDisposition, StreamUpdate, reduce};
    use lens_client::stream::{
        ResponseEvent, ServerStreamEvent, SessionEvent, SessionStatusValue as WireStatus,
    };

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
    fn terminal_failure_discards_open_accumulators_and_clears_scratch() {
        for term in [
            ResponseEvent::Failed,
            ResponseEvent::Incomplete,
            ResponseEvent::Cancelled,
        ] {
            let mut s = st();
            reduce(
                &mut s,
                &ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
                    delta: "partial".into(),
                    message_id: None,
                    index: None,
                    last: None,
                }),
                &clock(),
            );
            let acc_id = s.stream.open_message.as_ref().unwrap().acc_id.clone();
            let u = reduce(&mut s, &ServerStreamEvent::Response(term.clone()), &clock());
            assert!(
                s.stream.open_message.is_none(),
                "{term:?} must clear scratch"
            );
            assert!(u.iter().any(|x| matches!(x,
                StreamUpdate::Retired { acc_id: a, disposition: RetireDisposition::Discarded } if *a == acc_id)));
        }
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
        assert_eq!(
            &u[..],
            &[StreamUpdate::StatusChanged(SessionStatusValue::Running)]
        );
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
        assert!(matches!(&u[..], [StreamUpdate::TodosChanged(todos)] if todos.len() == 1));
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
        assert!(matches!(
            &u[..],
            [
                StreamUpdate::UsageChanged(cost),
                StreamUpdate::LastTokensChanged(Some(1200)),
                StreamUpdate::ContextWindowChanged(Some(200_000)),
            ] if cost.total_cost_usd == Some(0.42)
        ));
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
        assert!(matches!(&u[..], [StreamUpdate::PresenceChanged(v)] if v.len() == 1));
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
        assert!(matches!(
            &u[..],
            [StreamUpdate::ElicitationsChanged(elicitations)] if elicitations.is_empty()
        ));
    }

    #[test]
    fn non_failed_status_clears_last_task_error() {
        let mut s = st();
        s.last_task_error = Some(crate::domain::ErrorInfo {
            code: "E1".into(),
            message: "boom".into(),
        });
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Status {
                status: WireStatus::Idle,
                response_id: None,
                background_task_count: None,
            }),
            &clock(),
        );
        assert_eq!(s.last_task_error, None);
        assert!(matches!(
            &u[..],
            [
                StreamUpdate::StatusChanged(SessionStatusValue::Idle),
                StreamUpdate::LastTaskErrorChanged(None),
            ]
        ));
    }

    #[test]
    fn response_error_emits_status_and_last_task_error_changed() {
        use crate::reduce::testutil::parse_response;

        let mut s = st();
        s.status = SessionStatusValue::Failed;
        let ev = parse_response(
            "response.error",
            r#"{"source":"llm","tool_name":null,"error":{"code":"timeout","message":"timed out"}}"#,
        );
        let u = reduce(&mut s, &ev, &clock());
        assert_eq!(
            s.last_task_error,
            Some(crate::domain::ErrorInfo {
                code: "timeout".into(),
                message: "timed out".into(),
            })
        );
        assert!(matches!(
            &u[..],
            [
                StreamUpdate::StatusChanged(SessionStatusValue::Failed),
                StreamUpdate::LastTaskErrorChanged(Some(err)),
            ] if err.code == "timeout" && err.message == "timed out"
        ));
    }

    #[test]
    fn failed_status_preserves_last_task_error() {
        let mut s = st();
        let err = crate::domain::ErrorInfo {
            code: "E1".into(),
            message: "boom".into(),
        };
        s.last_task_error = Some(err.clone());
        reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Status {
                status: WireStatus::Failed,
                response_id: None,
                background_task_count: None,
            }),
            &clock(),
        );
        assert_eq!(s.last_task_error, Some(err));
    }

    #[test]
    fn terminal_activity_is_marker_only_no_pending_change() {
        let mut s = st();
        assert!(!s.terminal_pending);
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::TerminalActivity {
                terminal_id: "term_1".into(),
            }),
            &clock(),
        );
        assert!(u.is_empty());
        assert!(!s.terminal_pending);
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
        assert!(
            u.iter()
                .any(|update| matches!(update, StreamUpdate::AgentChanged { .. }))
        );
    }

    #[test]
    fn agent_changed_emits_scratch_when_current_agent_updates() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::AgentChanged {
                agent_id: "ag_2".into(),
                agent_name: "debby".into(),
            }),
            &clock(),
        );
        let scratch = u.iter().find_map(|update| match update {
            StreamUpdate::ScratchChanged(scratch) => Some(std::sync::Arc::clone(scratch)),
            _ => None,
        });
        let scratch =
            scratch.expect("AgentChanged must emit ScratchChanged when current_agent updates");
        assert_eq!(scratch.current_agent, s.stream.current_agent);
    }

    #[test]
    fn input_consumed_clears_matching_bubble_by_store_item_id() {
        use crate::domain::controls::PendingUserMessage;

        let mut s = st();
        s.pending_user.push(PendingUserMessage {
            pending_id: "lens_pend_1".into(),
            server_pending_id: None,
            store_item_id: Some("msg_1".into()),
            content: "hello".into(),
            created_at: 1_700_000_000_000,
        });
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
                cleared_pending_id: None,
            }),
            &clock(),
        );
        assert!(s.pending_user.is_empty());
        assert!(
            u.iter().any(
                |update| matches!(update, StreamUpdate::PendingUserChanged(v) if v.is_empty())
            )
        );
    }

    #[test]
    fn superseded_emits_stream_update() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::Superseded {
                conversation_id: "conv_a".into(),
                target_conversation_id: "conv_b".into(),
                reason: "clear".into(),
            }),
            &clock(),
        );
        assert_eq!(
            &u[..],
            &[StreamUpdate::Superseded {
                target_conversation_id: "conv_b".into(),
                reason: "clear".into(),
            }]
        );
    }

    #[test]
    fn terminal_resource_created_emits_terminal_resource_update() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::ResourceCreated {
                resource_id: "terminal_tui_main".into(),
                resource_type: "terminal".into(),
                terminal_name: Some("tui".into()),
                session_key: Some("main".into()),
            }),
            &clock(),
        );
        assert_eq!(
            &u[..],
            &[StreamUpdate::TerminalResourceCreated {
                terminal_id: TerminalId::new("terminal_tui_main"),
                terminal_name: "tui".into(),
                session_key: "main".into(),
                session_id: SessionId::new("conv"),
            }]
        );
    }

    #[test]
    fn terminal_resource_deleted_emits_terminal_resource_update() {
        let mut s = st();
        let u = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::ResourceDeleted {
                resource_id: "terminal_tui_main".into(),
                resource_type: "terminal".into(),
                session_id: "conv".into(),
            }),
            &clock(),
        );
        assert_eq!(
            &u[..],
            &[StreamUpdate::TerminalResourceDeleted {
                terminal_id: TerminalId::new("terminal_tui_main"),
            }]
        );
    }

    #[test]
    fn non_terminal_resource_events_keep_resources_changed_marker() {
        let mut s = st();
        let created = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::ResourceCreated {
                resource_id: "file_abc".into(),
                resource_type: "file".into(),
                terminal_name: None,
                session_key: None,
            }),
            &clock(),
        );
        assert_eq!(&created[..], &[StreamUpdate::ResourcesChanged]);

        let deleted = reduce(
            &mut s,
            &ServerStreamEvent::Session(SessionEvent::ResourceDeleted {
                resource_id: "file_abc".into(),
                resource_type: "file".into(),
                session_id: "conv".into(),
            }),
            &clock(),
        );
        assert_eq!(&deleted[..], &[StreamUpdate::ResourcesChanged]);
    }
}
