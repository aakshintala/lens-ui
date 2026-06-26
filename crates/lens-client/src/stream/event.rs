//! The typed SSE event taxonomy, modeled from captured bytes
//! (docs/spikes/captures/2026-06-26-sse/). `parse_event` is total: an unknown
//! or unparseable event degrades to `Unknown` so the reader thread never panics
//! on dev0 contract churn (AGENTS.md: the UI never panics).

use super::sse::SseFrame;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStreamEvent {
    Session(SessionEvent),
    Response(ResponseEvent),
    /// Forward-compat escape hatch for an event type this crate version does not
    /// model. Carries only the wire `type` (no `Value` to consumers); the raw
    /// payload is dropped. The contract test (Plan 3c) alarms when a live stream
    /// produces `Unknown`, signaling a needed crate bump.
    Unknown {
        event_type: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Status {
        status: SessionStatusValue,
        response_id: Option<String>,
    },
    Usage {
        context_tokens: Option<i64>,
        context_window: Option<i64>,
        total_cost_usd: Option<f64>,
    },
    Presence {
        viewers: Vec<PresenceViewer>,
    },
    Heartbeat {
        sequence_number: Option<i64>,
        server_time: Option<String>,
    },
    ResourceCreated,
    InputConsumed {
        item_id: String,
        item_type: String,
    },
    ChangedFilesInvalidated {
        environment_id: String,
    },
    Interrupted {
        requested_at: Option<i64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatusValue {
    Idle,
    Launching,
    Running,
    Waiting,
    Failed,
    /// Any status literal this crate version does not know (dev0 churn safety).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceViewer {
    user_id: Option<String>,
}
impl PresenceViewer {
    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }
}

// Internal raw shapes (private; never exposed) used only to deserialize.
#[derive(Deserialize)]
struct RawStatus {
    status: SessionStatusValue,
    #[serde(default)]
    response_id: Option<String>,
}
#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    context_tokens: Option<i64>,
    #[serde(default)]
    context_window: Option<i64>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
}
#[derive(Deserialize)]
struct RawPresence {
    #[serde(default)]
    viewers: Vec<RawViewer>,
}
#[derive(Deserialize)]
struct RawViewer {
    #[serde(default)]
    user_id: Option<String>,
}
#[derive(Deserialize)]
struct RawHeartbeat {
    #[serde(default)]
    sequence_number: Option<i64>,
    #[serde(default)]
    server_time: Option<String>,
}
#[derive(Deserialize)]
struct RawChangedFiles {
    environment_id: String,
}
#[derive(Deserialize)]
struct RawInputConsumed {
    data: RawInputConsumedData,
}
#[derive(Deserialize)]
struct RawInputConsumedData {
    item_id: String,
    #[serde(rename = "type")]
    item_type: String,
}
#[derive(Deserialize)]
struct RawInterrupted {
    #[serde(default)]
    data: Option<RawInterruptedData>,
}
#[derive(Deserialize)]
struct RawInterruptedData {
    #[serde(default)]
    requested_at: Option<i64>,
}
#[derive(Deserialize)]
struct RawTextDelta {
    delta: String,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default, rename = "final")]
    last: Option<bool>,
}
#[derive(Deserialize)]
struct RawItemEnvelope {
    item: serde_json::Value,
}
#[derive(Deserialize)]
struct RawContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}
#[derive(Deserialize)]
struct RawErrorData {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {
    InProgress,
    Completed,
    OutputTextDelta {
        delta: String,
        message_id: Option<String>,
        index: Option<usize>,
        last: Option<bool>,
    },
    ReasoningStarted,
    OutputItemDone {
        item: Item,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Message {
        id: String,
        role: String,
        content: Vec<MessageContentBlock>,
    },
    /// `arguments` is the raw JSON string as it arrives on the wire (unparsed —
    /// the state model owns parsing). `agent` is a wire wart: it is the
    /// `resp_…` response id while `status == "in_progress"`, and the agent name
    /// once `completed`. Exposed verbatim; consumers must not assume a name.
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        status: String,
        agent: Option<String>,
    },
    FunctionCallOutput {
        id: String,
        call_id: String,
        output: String,
    },
    Error {
        id: String,
        source: Option<String>,
        code: Option<String>,
        message: Option<String>,
    },
    /// Forward-compat for item types not yet modeled (native_tool, reasoning,
    /// compaction, slash_command, terminal_command, resource_event) — added in
    /// Task 6 / at config-time capture.
    Other { item_type: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageContentBlock {
    block_type: String,
    text: Option<String>,
}
impl MessageContentBlock {
    pub fn block_type(&self) -> &str {
        &self.block_type
    }
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

/// Total: maps a raw frame to a typed event, degrading to `Unknown` on any
/// unmodeled type or deserialization failure. Modeled-family dispatch is added
/// by Tasks 3–4 (each returns `Some(event)` or `None` → fall through to Unknown).
pub(crate) fn parse_event(frame: &SseFrame) -> ServerStreamEvent {
    if let Some(ev) = SessionEvent::from_frame(frame) {
        return ServerStreamEvent::Session(ev);
    }
    if let Some(ev) = ResponseEvent::from_frame(frame) {
        return ServerStreamEvent::Response(ev);
    }
    ServerStreamEvent::Unknown {
        event_type: frame.event.clone(),
    }
}

impl SessionEvent {
    fn from_frame(frame: &SseFrame) -> Option<Self> {
        // Returns None on a non-session.* type → parse_event falls through.
        // A modeled type that fails to deserialize maps to Unknown at the
        // parse_event layer is NOT what we want here; instead we surface a safe
        // default so the chrome event is not silently dropped. We do that by
        // returning Some with best-effort fields, falling back to Unknown status
        // / empty collections (serde `default`). A hard parse failure on a
        // session.* type returns None (→ Unknown) — acceptable, it is logged.
        let d = &frame.data;
        Some(match frame.event.as_str() {
            "session.status" => {
                let r: RawStatus = serde_json::from_str(d).ok()?;
                SessionEvent::Status {
                    status: r.status,
                    response_id: r.response_id,
                }
            }
            "session.usage" => {
                let r: RawUsage = serde_json::from_str(d).ok()?;
                SessionEvent::Usage {
                    context_tokens: r.context_tokens,
                    context_window: r.context_window,
                    total_cost_usd: r.total_cost_usd,
                }
            }
            "session.presence" => {
                let r: RawPresence = serde_json::from_str(d).ok()?;
                SessionEvent::Presence {
                    viewers: r
                        .viewers
                        .into_iter()
                        .map(|v| PresenceViewer { user_id: v.user_id })
                        .collect(),
                }
            }
            "session.heartbeat" => {
                let r: RawHeartbeat = serde_json::from_str(d).ok()?;
                SessionEvent::Heartbeat {
                    sequence_number: r.sequence_number,
                    server_time: r.server_time,
                }
            }
            "session.resource.created" => SessionEvent::ResourceCreated,
            "session.input.consumed" => {
                let r: RawInputConsumed = serde_json::from_str(d).ok()?;
                SessionEvent::InputConsumed {
                    item_id: r.data.item_id,
                    item_type: r.data.item_type,
                }
            }
            "session.changed_files.invalidated" => {
                let r: RawChangedFiles = serde_json::from_str(d).ok()?;
                SessionEvent::ChangedFilesInvalidated {
                    environment_id: r.environment_id,
                }
            }
            "session.interrupted" => {
                let r: RawInterrupted = serde_json::from_str(d).ok()?;
                SessionEvent::Interrupted {
                    requested_at: r.data.and_then(|x| x.requested_at),
                }
            }
            _ => return None,
        })
    }
}
impl ResponseEvent {
    fn from_frame(frame: &SseFrame) -> Option<Self> {
        let d = &frame.data;
        Some(match frame.event.as_str() {
            "response.in_progress" => ResponseEvent::InProgress,
            "response.completed" => ResponseEvent::Completed,
            "response.reasoning.started" => ResponseEvent::ReasoningStarted,
            "response.output_text.delta" => {
                let r: RawTextDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::OutputTextDelta {
                    delta: r.delta,
                    message_id: r.message_id,
                    index: r.index,
                    last: r.last,
                }
            }
            "response.output_item.done" => {
                let env: RawItemEnvelope = serde_json::from_str(d).ok()?;
                ResponseEvent::OutputItemDone {
                    item: Item::from_value(env.item),
                }
            }
            _ => return None,
        })
    }
}

impl Item {
    /// Total over a wire item object; unmodeled `type`s map to `Other`.
    fn from_value(v: serde_json::Value) -> Self {
        let id = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let item_type = v
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let so = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        match item_type.as_str() {
            "message" => {
                let content = v
                    .get("content")
                    .and_then(|c| serde_json::from_value::<Vec<RawContentBlock>>(c.clone()).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|b| MessageContentBlock {
                        block_type: b.block_type,
                        text: b.text,
                    })
                    .collect();
                Item::Message {
                    id,
                    role: s("role"),
                    content,
                }
            }
            "function_call" => Item::FunctionCall {
                id,
                call_id: s("call_id"),
                name: s("name"),
                arguments: s("arguments"),
                status: s("status"),
                agent: so("agent"),
            },
            "function_call_output" => Item::FunctionCallOutput {
                id,
                call_id: s("call_id"),
                output: s("output"),
            },
            "error" => {
                let data = v
                    .get("data")
                    .and_then(|x| serde_json::from_value::<RawErrorData>(x.clone()).ok())
                    .unwrap_or(RawErrorData {
                        source: None,
                        code: None,
                        message: None,
                    });
                Item::Error {
                    id,
                    source: data.source,
                    code: data.code,
                    message: data.message,
                }
            }
            other => Item::Other {
                item_type: other.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame {
            event: event.into(),
            data: data.into(),
        }
    }

    #[test]
    fn unmodeled_event_type_degrades_to_unknown() {
        let ev = parse_event(&frame("session.brand_new_2027", "{}"));
        assert_eq!(
            ev,
            ServerStreamEvent::Unknown {
                event_type: "session.brand_new_2027".into()
            }
        );
    }

    #[test]
    fn garbage_data_on_unknown_type_still_does_not_panic() {
        let ev = parse_event(&frame("totally.unknown", "not json{{"));
        assert!(matches!(ev, ServerStreamEvent::Unknown { .. }));
    }

    #[test]
    fn status_running_from_bytes() {
        let ev = parse_event(&frame(
            "session.status",
            r#"{"conversation_id":"c","status":"running","response_id":null,"error":null}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Running,
                response_id: None,
            })
        );
    }

    #[test]
    fn unknown_status_string_is_not_a_panic() {
        let ev = parse_event(&frame("session.status", r#"{"status":"hibernating"}"#));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Unknown,
                response_id: None,
            })
        );
    }

    #[test]
    fn changed_files_invalidated_has_no_paths_field() {
        // Byte-verified: payload is {session_id, environment_id}; the design's
        // `paths` field does not exist on the wire.
        let ev = parse_event(&frame(
            "session.changed_files.invalidated",
            r#"{"sequence_number":null,"session_id":"c","environment_id":"default"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ChangedFilesInvalidated {
                environment_id: "default".into(),
            })
        );
    }

    #[test]
    fn input_consumed_reads_nested_data() {
        let ev = parse_event(&frame(
            "session.input.consumed",
            r#"{"data":{"item_id":"msg_1","type":"message","data":{}}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
            })
        );
    }

    #[test]
    fn interrupted_carries_requested_at() {
        let ev = parse_event(&frame(
            "session.interrupted",
            r#"{"data":{"requested_at":1782502914,"response_id":null}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Interrupted {
                requested_at: Some(1782502914)
            })
        );
    }

    #[test]
    fn interrupt_fixture_yields_a_session_interrupted_event() {
        let bytes = include_bytes!("../../tests/fixtures/sse/interrupt.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        assert!(frames.iter().map(parse_event).any(|e| matches!(
            e,
            ServerStreamEvent::Session(SessionEvent::Interrupted { .. })
        )));
    }

    #[test]
    fn output_text_delta_from_bytes() {
        let ev = parse_event(&frame(
            "response.output_text.delta",
            r#"{"sequence_number":4,"delta":"Hello","message_id":null,"index":null,"final":null}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
                delta: "Hello".into(),
                message_id: None,
                index: None,
                last: None,
            })
        );
    }

    #[test]
    fn output_item_done_function_call_keeps_arguments_as_string() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"sys_os_shell","arguments":"{\"command\":\"pwd\"}","call_id":"toolu_1","agent":"claude-sdk"}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item:
                    Item::FunctionCall {
                        name,
                        arguments,
                        call_id,
                        agent,
                        ..
                    },
            }) => {
                assert_eq!(name, "sys_os_shell");
                assert_eq!(arguments, r#"{"command":"pwd"}"#); // raw JSON string, unparsed
                assert_eq!(call_id, "toolu_1");
                assert_eq!(agent.as_deref(), Some("claude-sdk"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn output_item_done_message_and_output() {
        let m = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"msg_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hi"}]}}"#,
        ));
        assert!(matches!(
            m,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item: Item::Message { .. }
            })
        ));
        let o = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"fco_1","type":"function_call_output","call_id":"toolu_1","output":"/work"}}"#,
        ));
        assert!(matches!(
            o,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item: Item::FunctionCallOutput { .. }
            })
        ));
    }

    #[test]
    fn error_item_from_bytes() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"err_1","type":"error","status":"completed","data":{"source":"execution","code":"RuntimeError","message":"boom"}}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item:
                    Item::Error {
                        code,
                        message,
                        source,
                        ..
                    },
            }) => {
                assert_eq!(code.as_deref(), Some("RuntimeError"));
                assert_eq!(message.as_deref(), Some("boom"));
                assert_eq!(source.as_deref(), Some("execution"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn unmodeled_item_type_becomes_other_not_panic() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"x","type":"native_tool","kind":"web_search_call"}}"#,
        ));
        assert!(matches!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item: Item::Other { .. }
            })
        ));
    }

    #[test]
    fn happy_path_fixture_full_event_coverage() {
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let events: Vec<_> = frames.iter().map(parse_event).collect();
        // No event in the captured happy-path turn falls through to Unknown.
        let unknowns: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ServerStreamEvent::Unknown { event_type } => Some(event_type.clone()),
                _ => None,
            })
            .collect();
        assert!(
            unknowns.is_empty(),
            "unmodeled captured events: {unknowns:?}"
        );
        // The item union is exercised: function_call, message, function_call_output all present.
        let has = |pred: fn(&Item) -> bool| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item })
                        if pred(item)
                )
            })
        };
        assert!(has(|i| matches!(i, Item::FunctionCall { .. })));
        assert!(has(|i| matches!(i, Item::Message { .. })));
        assert!(has(|i| matches!(i, Item::FunctionCallOutput { .. })));
    }
}
