//! The typed SSE event taxonomy, modeled from captured bytes
//! (docs/spikes/captures/2026-06-26-sse/). `parse_event` is total: an unknown
//! or unparseable event degrades to `Unknown` so the reader thread never panics
//! on dev0 contract churn (AGENTS.md: the UI never panics).

// Consumed by stream reader (Task 5); allow until then.
#![allow(dead_code)]

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

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {} // filled in Task 4

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
    fn from_frame(_frame: &SseFrame) -> Option<Self> {
        None
    } // Task 4 fills this
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
}
