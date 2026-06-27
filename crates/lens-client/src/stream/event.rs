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
    /// Crate-synthetic: a reconnect attempt is in flight (typed-client §7 step 2).
    Reconnecting {
        attempt: u32,
    },
    /// Crate-synthetic: stream re-opened. `gap` per §7 / plan decision 2:
    /// `Some(0)` = provably contiguous overlap; `None` = clear transient state.
    Reconnected {
        gap: Option<u64>,
    },
    /// Crate-synthetic: bucket-B chrome restore (decision A2, typed-client §7).
    /// Emitted after `Reconnected`, before replayed history. Boxed (large payload).
    SnapshotRestored(Box<crate::sessions::SessionSnapshot>),
    /// Crate-synthetic: terminal. Last event before the channel closes (§7 step 3).
    Disconnected {
        reason: DisconnectReason,
    },
    /// Forward-compat escape hatch for an event type this crate version does not
    /// model. Carries only the wire `type` (no `Value` to consumers); the raw
    /// payload is dropped. The contract test (Plan 3c) alarms when a live stream
    /// produces `Unknown`, signaling a needed crate bump.
    Unknown {
        event_type: String,
    },
}

/// Why the stream gave up (typed-client §7 stop-immediately table + retries-exhausted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Unauthorized,     // 401 — re-auth
    Forbidden,        // 403 — access denied, remove session
    NotFound,         // 404 — session deleted, remove
    SessionFailed,    // snapshot status == failed — surface, no retry
    RetriesExhausted, // backoff window elapsed
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
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ChildSessionUpdated {
        child_session_id: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    TerminalActivity {
        terminal_id: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    TerminalPending {
        pending: bool,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Model {
        model: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Todos {
        todos: Vec<TodoItem>,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ReasoningEffort {
        reasoning_effort: Option<String>,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ModelOptions,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    SandboxStatus {
        stage: String,
        error: Option<String>,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Skills,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoItemStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    content: String,
    status: TodoItemStatus,
    active_form: String,
}
impl TodoItem {
    pub fn content(&self) -> &str {
        &self.content
    }
    pub fn status(&self) -> TodoItemStatus {
        self.status
    }
    pub fn active_form(&self) -> &str {
        &self.active_form
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
#[derive(Deserialize)]
struct RawReasoningDelta {
    delta: String,
}
#[derive(Deserialize)]
struct RawCompactionCompleted {
    #[serde(default)]
    total_tokens: Option<i64>,
}
#[derive(Deserialize)]
struct RawStreamErrorDetail {
    code: String,
    message: String,
}
#[derive(Deserialize)]
struct RawStreamError {
    source: String,
    #[serde(default)]
    tool_name: Option<String>,
    error: RawStreamErrorDetail,
}
#[derive(Deserialize)]
struct RawElicitationParams {
    #[serde(rename = "message")]
    _message: String,
}
#[derive(Deserialize)]
struct RawElicitationRequest {
    elicitation_id: String,
    #[serde(rename = "params")]
    _params: RawElicitationParams,
}
#[derive(Deserialize)]
struct RawElicitationResolved {
    elicitation_id: String,
}
#[derive(Deserialize)]
struct RawChildSessionUpdated {
    child_session_id: String,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
    #[serde(rename = "child")]
    _child: serde_json::Map<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct RawTerminalActivity {
    terminal_id: String,
    #[serde(rename = "session_id")]
    _session_id: String,
}
#[derive(Deserialize)]
struct RawTerminalPending {
    pending: bool,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawSessionModel {
    model: String,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawSessionReasoningEffort {
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawTodoItem {
    content: String,
    status: TodoItemStatus,
    #[serde(rename = "activeForm")]
    active_form: String,
}
#[derive(Deserialize)]
struct RawSessionTodos {
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
    todos: Vec<RawTodoItem>,
}
#[derive(Deserialize)]
struct RawSessionConversationOnly {
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawSessionSandboxStatus {
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
    stage: String,
    #[serde(default)]
    error: Option<String>,
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
    /// SYNTHETIC (typed-client.md §7a) — emitted by `stream::normalize::Normalizer`,
    /// never by `parse_event`. The SSE stream has no reasoning-end frame; the crate
    /// closes the bracket on the first `OutputTextDelta`/`Completed` after
    /// `ReasoningStarted`. `full_text`/`summary_text` accumulate the reasoning deltas
    /// so the renderer need not re-accumulate.
    /// NOT BYTE-VERIFIED (claude-sdk folds reasoning into output_text — re-capture at config-time)
    ReasoningClosed {
        full_text: String,
        summary_text: String,
    },
    OutputItemDone {
        item: Item,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Failed,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Incomplete,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Cancelled,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ReasoningTextDelta {
        delta: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ReasoningSummaryTextDelta {
        delta: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    CompactionInProgress,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    CompactionCompleted {
        total_tokens: Option<i64>,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    CompactionFailed,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Error {
        source: String,
        tool_name: Option<String>,
        code: String,
        message: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ElicitationRequest {
        elicitation_id: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ElicitationResolved {
        elicitation_id: String,
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
    /// A persisted resource lifecycle item (`/items` only; the live stream carries
    /// these as `session.resource.*` SessionEvents instead). `resource_type` is
    /// e.g. `terminal`; `event_type` is the wire `session.resource.created` form.
    ResourceEvent {
        id: String,
        resource_id: String,
        resource_type: String,
        event_type: String,
    },
    /// Forward-compat for item types not yet modeled. Retains `id` so the state
    /// model can still reconcile it by `id` (typed-client §7 step 5).
    Other { item_type: String, id: String },
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

/// Wire `ServerStreamEvent` discriminators the crate MODELS — `parse_event`
/// dispatches each to a typed `SessionEvent`/`ResponseEvent` variant. A modeled
/// type arriving as `Unknown` (e.g. its payload shape drifted and dispatch
/// degraded) is a contract problem the live taxonomy test must catch.
/// `parse_event` (below) is the SSOT for this set.
pub const MODELED_EVENT_TYPES: &[&str] = &[
    "response.cancelled",
    "response.compaction.completed",
    "response.compaction.failed",
    "response.compaction.in_progress",
    "response.completed",
    "response.elicitation_request",
    "response.elicitation_resolved",
    "response.error",
    "response.failed",
    "response.in_progress",
    "response.incomplete",
    "response.output_item.done",
    "response.output_text.delta",
    "response.reasoning.started",
    "response.reasoning_summary_text.delta",
    "response.reasoning_text.delta",
    "session.changed_files.invalidated",
    "session.child_session.updated",
    "session.heartbeat",
    "session.input.consumed",
    "session.interrupted",
    "session.model",
    "session.model_options",
    "session.presence",
    "session.reasoning_effort",
    "session.resource.created",
    "session.sandbox_status",
    "session.skills",
    "session.status",
    "session.terminal.activity",
    "session.terminal_pending",
    "session.todos",
    "session.usage",
];

/// Wire discriminators the pinned contract declares but the crate knowingly
/// routes to `Unknown` (deferred — absent from the golden captures). A deferred
/// type arriving as `Unknown` is EXPECTED; only these may legitimately surface
/// as `Unknown` on the live stream.
pub const DEFERRED_EVENT_TYPES: &[&str] = &[
    "response.client_task.cancel",
    "response.created",
    "response.heartbeat",
    "response.output_file.done",
    "response.queued",
    "response.retry",
    "session.agent_changed",
    "session.collaboration_mode",
    "session.created",
    "session.resource.deleted",
    "turn.cancelled",
    "turn.completed",
    "turn.failed",
    "turn.started",
];

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
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.child_session.updated" => {
                let r: RawChildSessionUpdated = serde_json::from_str(d).ok()?;
                SessionEvent::ChildSessionUpdated {
                    child_session_id: r.child_session_id,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.terminal.activity" => {
                let r: RawTerminalActivity = serde_json::from_str(d).ok()?;
                SessionEvent::TerminalActivity {
                    terminal_id: r.terminal_id,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.terminal_pending" => {
                let r: RawTerminalPending = serde_json::from_str(d).ok()?;
                SessionEvent::TerminalPending { pending: r.pending }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.model" => {
                let r: RawSessionModel = serde_json::from_str(d).ok()?;
                SessionEvent::Model { model: r.model }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.todos" => {
                let r: RawSessionTodos = serde_json::from_str(d).ok()?;
                SessionEvent::Todos {
                    todos: r
                        .todos
                        .into_iter()
                        .map(|t| TodoItem {
                            content: t.content,
                            status: t.status,
                            active_form: t.active_form,
                        })
                        .collect(),
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.reasoning_effort" => {
                let r: RawSessionReasoningEffort = serde_json::from_str(d).ok()?;
                SessionEvent::ReasoningEffort {
                    reasoning_effort: r.reasoning_effort,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.model_options" => {
                let _: RawSessionConversationOnly = serde_json::from_str(d).ok()?;
                SessionEvent::ModelOptions
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.sandbox_status" => {
                let r: RawSessionSandboxStatus = serde_json::from_str(d).ok()?;
                SessionEvent::SandboxStatus {
                    stage: r.stage,
                    error: r.error,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.skills" => {
                let _: RawSessionConversationOnly = serde_json::from_str(d).ok()?;
                SessionEvent::Skills
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
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.failed" => ResponseEvent::Failed,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.incomplete" => ResponseEvent::Incomplete,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.cancelled" => ResponseEvent::Cancelled,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.reasoning_text.delta" => {
                let r: RawReasoningDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::ReasoningTextDelta { delta: r.delta }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.reasoning_summary_text.delta" => {
                let r: RawReasoningDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::ReasoningSummaryTextDelta { delta: r.delta }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.compaction.in_progress" => ResponseEvent::CompactionInProgress,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.compaction.completed" => {
                let r: RawCompactionCompleted = serde_json::from_str(d).ok()?;
                ResponseEvent::CompactionCompleted {
                    total_tokens: r.total_tokens,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.compaction.failed" => ResponseEvent::CompactionFailed,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.error" => {
                let r: RawStreamError = serde_json::from_str(d).ok()?;
                ResponseEvent::Error {
                    source: r.source,
                    tool_name: r.tool_name,
                    code: r.error.code,
                    message: r.error.message,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.elicitation_request" => {
                let r: RawElicitationRequest = serde_json::from_str(d).ok()?;
                ResponseEvent::ElicitationRequest {
                    elicitation_id: r.elicitation_id,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.elicitation_resolved" => {
                let r: RawElicitationResolved = serde_json::from_str(d).ok()?;
                ResponseEvent::ElicitationResolved {
                    elicitation_id: r.elicitation_id,
                }
            }
            _ => return None,
        })
    }
}

impl Item {
    /// The item's stable `id` — the reconcile key for `GET /items` merge
    /// (persisted items carry no `sequence_number`; typed-client §7 step 5).
    pub fn id(&self) -> &str {
        match self {
            Item::Message { id, .. }
            | Item::FunctionCall { id, .. }
            | Item::FunctionCallOutput { id, .. }
            | Item::Error { id, .. }
            | Item::ResourceEvent { id, .. }
            | Item::Other { id, .. } => id,
        }
    }

    /// Total over a wire item object; unmodeled `type`s map to `Other`.
    pub(crate) fn from_value(v: serde_json::Value) -> Self {
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
            "resource_event" => Item::ResourceEvent {
                id,
                resource_id: s("resource_id"),
                resource_type: s("resource_type"),
                event_type: s("event_type"),
            },
            other => Item::Other {
                item_type: other.to_string(),
                id,
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
    fn synthetic_lifecycle_variants_exist_and_compare() {
        let a = ServerStreamEvent::Reconnecting { attempt: 2 };
        let b = ServerStreamEvent::Reconnected { gap: None };
        let c = ServerStreamEvent::Disconnected {
            reason: DisconnectReason::NotFound,
        };
        assert_eq!(a, ServerStreamEvent::Reconnecting { attempt: 2 });
        assert_ne!(b, ServerStreamEvent::Reconnected { gap: Some(0) });
        assert_ne!(
            c,
            ServerStreamEvent::Disconnected {
                reason: DisconnectReason::Unauthorized
            }
        );
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
    fn resource_event_item_is_typed_with_id() {
        let item = Item::from_value(serde_json::json!({
            "id": "rse_1", "type": "resource_event", "status": "completed",
            "event_type": "session.resource.created",
            "resource_id": "terminal_tui_main", "resource_type": "terminal",
            "resource": {"id": "terminal_tui_main", "object": "resource"}
        }));
        assert_eq!(
            item,
            Item::ResourceEvent {
                id: "rse_1".into(),
                resource_id: "terminal_tui_main".into(),
                resource_type: "terminal".into(),
                event_type: "session.resource.created".into(),
            }
        );
        assert_eq!(item.id(), "rse_1");
    }

    #[test]
    fn other_item_retains_its_id_for_reconcile() {
        let item = Item::from_value(serde_json::json!({
            "id": "x_9", "type": "native_tool", "kind": "web_search_call"
        }));
        assert_eq!(
            item,
            Item::Other {
                item_type: "native_tool".into(),
                id: "x_9".into()
            }
        );
        assert_eq!(item.id(), "x_9"); // reconcile-by-id works even for unmodeled types
    }

    #[test]
    fn id_accessor_is_total_over_all_variants() {
        let msg = Item::from_value(
            serde_json::json!({"id":"m1","type":"message","role":"assistant","content":[]}),
        );
        let fc = Item::from_value(
            serde_json::json!({"id":"fc1","type":"function_call","call_id":"c","name":"n","arguments":"{}","status":"completed"}),
        );
        let fco = Item::from_value(
            serde_json::json!({"id":"fco1","type":"function_call_output","call_id":"c","output":"o"}),
        );
        assert_eq!(msg.id(), "m1");
        assert_eq!(fc.id(), "fc1");
        assert_eq!(fco.id(), "fco1");
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

    #[test]
    fn schema_reasoning_text_delta() {
        // SCHEMA-DERIVED: ReasoningTextDeltaEvent {delta, sequence_number, type}.
        let ev = parse_event(&frame(
            "response.reasoning_text.delta",
            r#"{"delta":"because","sequence_number":5}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
                delta: "because".into()
            })
        );
    }

    #[test]
    fn schema_reasoning_summary_text_delta() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.reasoning_summary_text.delta",
            r#"{"delta":"sum"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta {
                delta: "sum".into()
            })
        );
    }

    #[test]
    fn schema_response_failed_carries_status() {
        // SCHEMA-DERIVED: response.failed mirrors response.completed (response obj).
        let ev = parse_event(&frame(
            "response.failed",
            r#"{"response":{"status":"failed"}}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::Failed));
    }

    #[test]
    fn schema_response_incomplete() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.incomplete",
            r#"{"response":{"status":"incomplete"}}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::Incomplete));
    }

    #[test]
    fn schema_response_cancelled() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.cancelled",
            r#"{"response":{"status":"cancelled"}}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::Cancelled));
    }

    #[test]
    fn schema_compaction_in_progress() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame("response.compaction.in_progress", "{}"));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::CompactionInProgress)
        );
    }

    #[test]
    fn schema_compaction_completed() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.compaction.completed",
            r#"{"total_tokens":8421}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::CompactionCompleted {
                total_tokens: Some(8421)
            })
        );
    }

    #[test]
    fn schema_compaction_failed() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame("response.compaction.failed", "{}"));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::CompactionFailed)
        );
    }

    #[test]
    fn schema_response_error() {
        // SCHEMA-DERIVED: ErrorEvent {source, tool_name, error:{code, message}}.
        let ev = parse_event(&frame(
            "response.error",
            r#"{"source":"llm","tool_name":null,"error":{"code":"timeout","message":"timed out"}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::Error {
                source: "llm".into(),
                tool_name: None,
                code: "timeout".into(),
                message: "timed out".into(),
            })
        );
    }

    #[test]
    fn schema_elicitation_request() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.elicitation_request",
            r#"{"elicitation_id":"elicit_abc","params":{"message":"approve?"}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::ElicitationRequest {
                elicitation_id: "elicit_abc".into()
            })
        );
    }

    #[test]
    fn schema_elicitation_resolved() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "response.elicitation_resolved",
            r#"{"elicitation_id":"elicit_abc"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::ElicitationResolved {
                elicitation_id: "elicit_abc".into()
            })
        );
    }

    #[test]
    fn schema_child_session_updated() {
        // SCHEMA-DERIVED: session.child_session.updated — flat child_session_id per openapi.
        let ev = parse_event(&frame(
            "session.child_session.updated",
            r#"{"conversation_id":"conv_parent","child_session_id":"conv_child","child":{}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated {
                child_session_id: "conv_child".into()
            })
        );
    }

    #[test]
    fn schema_terminal_activity() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.terminal.activity",
            r#"{"session_id":"conv_abc","terminal_id":"terminal_zsh_s1"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::TerminalActivity {
                terminal_id: "terminal_zsh_s1".into()
            })
        );
    }

    #[test]
    fn schema_terminal_pending() {
        // SCHEMA-DERIVED: session.terminal_pending carries pending, not terminal_id.
        let ev = parse_event(&frame(
            "session.terminal_pending",
            r#"{"conversation_id":"conv_abc","pending":true}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::TerminalPending { pending: true })
        );
    }

    #[test]
    fn schema_session_model() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.model",
            r#"{"conversation_id":"conv_abc","model":"opus"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Model {
                model: "opus".into()
            })
        );
    }

    #[test]
    fn schema_session_todos() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.todos",
            r#"{"conversation_id":"conv_abc","todos":[{"content":"Fix the bug","status":"in_progress","activeForm":"Fixing the bug"}]}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Todos {
                todos: vec![TodoItem {
                    content: "Fix the bug".into(),
                    status: TodoItemStatus::InProgress,
                    active_form: "Fixing the bug".into(),
                }],
            })
        );
    }

    #[test]
    fn schema_reasoning_effort() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.reasoning_effort",
            r#"{"conversation_id":"conv_abc","reasoning_effort":"high"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ReasoningEffort {
                reasoning_effort: Some("high".into())
            })
        );
    }

    #[test]
    fn schema_model_options() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.model_options",
            r#"{"conversation_id":"conv_abc"}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Session(SessionEvent::ModelOptions));
    }

    #[test]
    fn schema_sandbox_status() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.sandbox_status",
            r#"{"conversation_id":"conv_abc","stage":"provisioning"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::SandboxStatus {
                stage: "provisioning".into(),
                error: None,
            })
        );
    }

    #[test]
    fn schema_skills() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame(
            "session.skills",
            r#"{"conversation_id":"conv_abc"}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Session(SessionEvent::Skills));
    }
}
