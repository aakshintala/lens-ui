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
    /// Emitted on reconnect (after `Reconnected`) and on first-open bootstrap
    /// (without a preceding `Reconnected`), before replayed history. Boxed (large payload).
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
        background_task_count: Option<i64>,
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
    ResourceDeleted {
        resource_id: String,
        resource_type: String,
    },
    InputConsumed {
        item_id: String,
        item_type: String,
        cleared_pending_id: Option<String>,
    },
    ChangedFilesInvalidated {
        environment_id: String,
    },
    Interrupted {
        requested_at: Option<i64>,
    },
    /// This conversation was superseded by another (e.g. a Claude `/clear`
    /// rotation); an actively-viewing client should follow to
    /// `target_conversation_id`. Transient (SSE-only, no replay) — the durable
    /// counterpart is a persisted notice message on the old conversation.
    Superseded {
        conversation_id: String,
        target_conversation_id: String,
        /// Why it was superseded; currently always `"clear"`.
        reason: String,
    },
    ChildSessionUpdated {
        child_session_id: String,
        child: ChildSession,
    },
    TerminalActivity {
        terminal_id: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    TerminalPending {
        pending: bool,
    },
    Model {
        model: String,
    },
    Todos {
        todos: Vec<TodoItem>,
    },
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
    Skills,
    AgentChanged {
        agent_id: String,
        agent_name: String,
    },
    Created {
        child_session_id: String,
        agent_id: Option<String>,
        parent_session_id: Option<String>,
    },
    /// Per-MCP-server startup progress for a native harness session (0.5.0).
    /// Transient (SSE + snapshot cache); a client connecting mid-startup seeds
    /// from the snapshot's `mcp_startup` field and updates live off this event.
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    McpStartup {
        servers: Vec<McpServerStartup>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildTaskStatus {
    Launching,
    InProgress,
    Completed,
    /// Any status this crate version does not know (dev0 churn safety).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChildSession {
    id: Option<String>,
    title: Option<String>,
    tool: Option<String>,
    session_name: Option<String>,
    busy: Option<bool>,
    current_task_status: Option<ChildTaskStatus>,
}
impl ChildSession {
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    pub fn tool(&self) -> Option<&str> {
        self.tool.as_deref()
    }
    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }
    pub fn busy(&self) -> Option<bool> {
        self.busy
    }
    pub fn current_task_status(&self) -> Option<ChildTaskStatus> {
        self.current_task_status
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

/// One MCP server's startup state within a `session.mcp_startup` event (0.5.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerStartup {
    name: String,
    status: McpStartupStatus,
    error: Option<String>,
}
impl McpServerStartup {
    /// The configured server name (map key on the wire), e.g. `"safe"`.
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn status(&self) -> McpStartupStatus {
        self.status
    }
    /// Failure detail when `status == Failed`; `None` otherwise.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpStartupStatus {
    Starting,
    Ready,
    Failed,
    Cancelled,
    /// Any status this crate version does not know (dev0 churn safety).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationParams {
    mode: String,
    message: String,
    url: Option<String>,
    phase: Option<String>,
    policy_name: Option<String>,
    content_preview: Option<String>,
}
impl ElicitationParams {
    pub fn mode(&self) -> &str {
        &self.mode
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
    pub fn phase(&self) -> Option<&str> {
        self.phase.as_deref()
    }
    pub fn policy_name(&self) -> Option<&str> {
        self.policy_name.as_deref()
    }
    pub fn content_preview(&self) -> Option<&str> {
        self.content_preview.as_deref()
    }
}

// Internal raw shapes (private; never exposed) used only to deserialize.
#[derive(Deserialize)]
struct RawStatus {
    status: SessionStatusValue,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    background_task_count: Option<i64>,
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
struct RawResourceDeleted {
    resource_id: String,
    resource_type: String,
    #[serde(rename = "session_id")]
    _session_id: String,
}
#[derive(Deserialize)]
struct RawChangedFiles {
    environment_id: String,
}
// FLAT shape (not enveloped under `data`): `{conversation_id,
// target_conversation_id, reason?}`. `reason` defaults to "clear" server-side.
#[derive(Deserialize)]
struct RawSuperseded {
    conversation_id: String,
    target_conversation_id: String,
    #[serde(default)]
    reason: Option<String>,
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
    #[serde(default)]
    cleared_pending_id: Option<String>,
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
/// Contract default for `ElicitationRequestParams.mode` (generated.rs maps a
/// missing `mode` to `Mode::Form`); keep our `String` model faithful on omission.
fn default_elicitation_mode() -> String {
    "form".to_string()
}
#[derive(Deserialize)]
struct RawElicitationParams {
    #[serde(default = "default_elicitation_mode")]
    mode: String,
    message: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    policy_name: Option<String>,
    #[serde(default)]
    content_preview: Option<String>,
    #[serde(default, rename = "requestedSchema")]
    _requested_schema: serde_json::Value,
    #[serde(default, rename = "target_session_id")]
    _target_session_id: Option<String>,
}
#[derive(Deserialize)]
struct RawElicitationRequest {
    elicitation_id: String,
    params: RawElicitationParams,
}
#[derive(Deserialize)]
struct RawElicitationResolved {
    elicitation_id: String,
}
#[derive(Deserialize)]
struct RawChild {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    session_name: Option<String>,
    #[serde(default)]
    busy: Option<bool>,
    #[serde(default)]
    current_task_status: Option<ChildTaskStatus>,
}
#[derive(Deserialize)]
struct RawChildSessionUpdated {
    child_session_id: String,
    child: RawChild,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
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
#[derive(Deserialize)]
struct RawAgentChanged {
    agent_id: String,
    agent_name: String,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawSessionCreated {
    child_session_id: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
#[derive(Deserialize)]
struct RawMcpStartup {
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
    // BTreeMap: deterministic ordering when flattened to the exposed Vec.
    // `servers` is REQUIRED by the contract — NOT defaulted, so a frame missing it
    // fails deser and degrades to `Unknown` rather than fabricating an empty event.
    servers: std::collections::BTreeMap<String, RawMcpServerStartup>,
}
#[derive(Deserialize)]
struct RawMcpServerStartup {
    status: McpStartupStatus,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Deserialize)]
struct RawPolicyDenied {
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {
    InProgress {
        response_id: Option<String>,
    },
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
    Failed,
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    Incomplete,
    Cancelled,
    ReasoningTextDelta {
        delta: String,
    },
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    ReasoningSummaryTextDelta {
        delta: String,
    },
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
    ElicitationRequest {
        elicitation_id: String,
        params: ElicitationParams,
    },
    ElicitationResolved {
        elicitation_id: String,
    },
    /// A policy DENY was enforced on a native harness turn (0.5.0). Fire-and-forget
    /// and observational — it does not gate the turn (the vendor command-hook already
    /// did) and carries no correlation id; it surfaces the decision so observers can
    /// see a native DENY as a positive signal rather than infer it from an absence.
    // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
    PolicyDenied {
        phase: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Message {
        id: String,
        role: String,
        content: Vec<MessageContentBlock>,
        response_id: Option<String>,
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
        response_id: Option<String>,
    },
    FunctionCallOutput {
        id: String,
        call_id: String,
        output: String,
        response_id: Option<String>,
    },
    Error {
        id: String,
        source: Option<String>,
        code: Option<String>,
        message: Option<String>,
        response_id: Option<String>,
    },
    /// A persisted resource lifecycle item (`/items` only; the live stream carries
    /// these as `session.resource.*` SessionEvents instead). `resource_type` is
    /// e.g. `terminal`; `event_type` is the wire `session.resource.created` form.
    ResourceEvent {
        id: String,
        resource_id: String,
        resource_type: String,
        event_type: String,
        response_id: Option<String>,
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
    "response.policy_denied",
    "response.reasoning.started",
    "response.reasoning_summary_text.delta",
    "response.reasoning_text.delta",
    "session.agent_changed",
    "session.changed_files.invalidated",
    "session.child_session.updated",
    "session.created",
    "session.heartbeat",
    "session.input.consumed",
    "session.interrupted",
    "session.mcp_startup",
    "session.model",
    "session.model_options",
    "session.presence",
    "session.reasoning_effort",
    "session.resource.created",
    "session.resource.deleted",
    "session.sandbox_status",
    "session.skills",
    "session.status",
    "session.superseded",
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
    "session.collaboration_mode",
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
                    background_task_count: r.background_task_count,
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
            "session.resource.deleted" => {
                let r: RawResourceDeleted = serde_json::from_str(d).ok()?;
                SessionEvent::ResourceDeleted {
                    resource_id: r.resource_id,
                    resource_type: r.resource_type,
                }
            }
            "session.input.consumed" => {
                let r: RawInputConsumed = serde_json::from_str(d).ok()?;
                SessionEvent::InputConsumed {
                    item_id: r.data.item_id,
                    item_type: r.data.item_type,
                    cleared_pending_id: r.data.cleared_pending_id,
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
            "session.superseded" => {
                let r: RawSuperseded = serde_json::from_str(d).ok()?;
                SessionEvent::Superseded {
                    conversation_id: r.conversation_id,
                    target_conversation_id: r.target_conversation_id,
                    reason: r.reason.unwrap_or_else(|| "clear".to_string()),
                }
            }
            "session.child_session.updated" => {
                let r: RawChildSessionUpdated = serde_json::from_str(d).ok()?;
                SessionEvent::ChildSessionUpdated {
                    child_session_id: r.child_session_id,
                    child: ChildSession {
                        id: r.child.id,
                        title: r.child.title,
                        tool: r.child.tool,
                        session_name: r.child.session_name,
                        busy: r.child.busy,
                        current_task_status: r.child.current_task_status,
                    },
                }
            }
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
            "session.model" => {
                let r: RawSessionModel = serde_json::from_str(d).ok()?;
                SessionEvent::Model { model: r.model }
            }
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
            "session.skills" => {
                let _: RawSessionConversationOnly = serde_json::from_str(d).ok()?;
                SessionEvent::Skills
            }
            "session.agent_changed" => {
                let r: RawAgentChanged = serde_json::from_str(d).ok()?;
                SessionEvent::AgentChanged {
                    agent_id: r.agent_id,
                    agent_name: r.agent_name,
                }
            }
            "session.created" => {
                let r: RawSessionCreated = serde_json::from_str(d).ok()?;
                SessionEvent::Created {
                    child_session_id: r.child_session_id,
                    agent_id: r.agent_id,
                    parent_session_id: r.parent_session_id,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "session.mcp_startup" => {
                let r: RawMcpStartup = serde_json::from_str(d).ok()?;
                SessionEvent::McpStartup {
                    servers: r
                        .servers
                        .into_iter()
                        .map(|(name, s)| McpServerStartup {
                            name,
                            status: s.status,
                            error: s.error,
                        })
                        .collect(),
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
            "response.in_progress" => {
                let obj: serde_json::Value = serde_json::from_str(d).ok()?;
                ResponseEvent::InProgress {
                    response_id: obj
                        .pointer("/response/id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                }
            }
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
            "response.failed" => ResponseEvent::Failed,
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.incomplete" => ResponseEvent::Incomplete,
            "response.cancelled" => ResponseEvent::Cancelled,
            "response.reasoning_text.delta" => {
                let r: RawReasoningDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::ReasoningTextDelta { delta: r.delta }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.reasoning_summary_text.delta" => {
                let r: RawReasoningDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::ReasoningSummaryTextDelta { delta: r.delta }
            }
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
            "response.elicitation_request" => {
                let r: RawElicitationRequest = serde_json::from_str(d).ok()?;
                ResponseEvent::ElicitationRequest {
                    elicitation_id: r.elicitation_id,
                    params: ElicitationParams {
                        mode: r.params.mode,
                        message: r.params.message,
                        url: r.params.url,
                        phase: r.params.phase,
                        policy_name: r.params.policy_name,
                        content_preview: r.params.content_preview,
                    },
                }
            }
            "response.elicitation_resolved" => {
                let r: RawElicitationResolved = serde_json::from_str(d).ok()?;
                ResponseEvent::ElicitationResolved {
                    elicitation_id: r.elicitation_id,
                }
            }
            // SCHEMA-DERIVED (not byte-verified — re-capture at config-time)
            "response.policy_denied" => {
                let r: RawPolicyDenied = serde_json::from_str(d).ok()?;
                ResponseEvent::PolicyDenied {
                    phase: r.phase,
                    reason: r.reason,
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

    /// The server `response_id` this item belongs to, if the wire carried one.
    /// `None` for the `Other` catch-all and for pre-response_id wire rows.
    pub fn response_id(&self) -> Option<&str> {
        match self {
            Item::Message { response_id, .. }
            | Item::FunctionCall { response_id, .. }
            | Item::FunctionCallOutput { response_id, .. }
            | Item::Error { response_id, .. }
            | Item::ResourceEvent { response_id, .. } => response_id.as_deref(),
            Item::Other { .. } => None,
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
        let wire_response_id = || {
            v.get("response_id")
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
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
                    response_id: wire_response_id(),
                }
            }
            "function_call" => Item::FunctionCall {
                id,
                call_id: s("call_id"),
                name: s("name"),
                arguments: s("arguments"),
                status: s("status"),
                agent: so("agent"),
                response_id: wire_response_id(),
            },
            "function_call_output" => Item::FunctionCallOutput {
                id,
                call_id: s("call_id"),
                output: s("output"),
                response_id: wire_response_id(),
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
                    response_id: wire_response_id(),
                }
            }
            "resource_event" => Item::ResourceEvent {
                id,
                resource_id: s("resource_id"),
                resource_type: s("resource_type"),
                event_type: s("event_type"),
                response_id: wire_response_id(),
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
                background_task_count: None,
            })
        );
    }

    #[test]
    fn status_background_task_count_present_and_absent() {
        let with_count = parse_event(&frame(
            "session.status",
            r#"{"status":"idle","background_task_count":3}"#,
        ));
        assert_eq!(
            with_count,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Idle,
                response_id: None,
                background_task_count: Some(3),
            })
        );

        let without_count = parse_event(&frame("session.status", r#"{"status":"idle"}"#));
        assert_eq!(
            without_count,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Idle,
                response_id: None,
                background_task_count: None,
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
                background_task_count: None,
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
                cleared_pending_id: None,
            })
        );
    }

    #[test]
    fn input_consumed_carries_cleared_pending_id_when_present() {
        let ev = parse_event(&frame(
            "session.input.consumed",
            r#"{"data":{"item_id":"msg_1","type":"message","data":{},"cleared_pending_id":"pending_a1b2c3"}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
                cleared_pending_id: Some("pending_a1b2c3".into()),
            })
        );
    }

    #[test]
    fn input_consumed_cleared_pending_id_defaults_none() {
        let ev = parse_event(&frame(
            "session.input.consumed",
            r#"{"data":{"item_id":"msg_1","type":"message","data":{}}}"#,
        ));
        let ServerStreamEvent::Session(SessionEvent::InputConsumed {
            cleared_pending_id, ..
        }) = ev
        else {
            panic!("expected InputConsumed");
        };
        assert_eq!(cleared_pending_id, None);
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
                response_id: None,
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
    fn bytes_reasoning_text_delta() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/cursor-sdk-reasoning.sse
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
    fn bytes_response_failed_carries_status() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/pi-failed-model.sse
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
    fn bytes_response_cancelled() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/interrupt-cancelled.sse
        let ev = parse_event(&frame(
            "response.cancelled",
            r#"{"response":{"status":"cancelled"}}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::Cancelled));
    }

    #[test]
    fn bytes_compaction_in_progress() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/chrome-model-effort-compact.sse
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
        // SCHEMA-DERIVED.
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
    fn bytes_elicitation_request_params() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/elicitation-request.sse
        let ev = parse_event(&frame(
            "response.elicitation_request",
            r#"{"sequence_number":null,"type":"response.elicitation_request","elicitation_id":"elicit_17f","method":"elicitation/create","params":{"mode":"url","message":"approve_file_ops: Agent wants to call sys_os_write('/tmp/spike_elicit.txt'). Approve?","requestedSchema":{},"url":"/approve/conv_78/elicit_17f","phase":"tool_call","policy_name":"approve_file_ops","content_preview":"{\"path\": \"/tmp/spike_elicit.txt\"}","target_session_id":null}}"#,
        ));
        let ServerStreamEvent::Response(ResponseEvent::ElicitationRequest {
            elicitation_id,
            params,
        }) = ev
        else {
            panic!("expected ElicitationRequest, got {ev:?}");
        };
        assert_eq!(elicitation_id, "elicit_17f");
        assert_eq!(params.policy_name(), Some("approve_file_ops"));
        assert_eq!(params.phase(), Some("tool_call"));
        assert_eq!(params.mode(), "url");
        assert!(params.message().contains("Approve?"));
        assert!(
            params
                .content_preview()
                .is_some_and(|s| s.contains("spike_elicit.txt"))
        );
        assert_eq!(params.url(), Some("/approve/conv_78/elicit_17f"));
    }

    #[test]
    fn elicitation_request_nullable_params_stay_typed() {
        let ev = parse_event(&frame(
            "response.elicitation_request",
            r#"{"elicitation_id":"elicit_sparse","params":{"message":"approve?","url":null,"phase":null,"policy_name":null,"content_preview":null}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::ElicitationRequest {
                elicitation_id: "elicit_sparse".into(),
                params: ElicitationParams {
                    // `mode` omitted on the wire → contract default "form".
                    mode: "form".into(),
                    message: "approve?".into(),
                    url: None,
                    phase: None,
                    policy_name: None,
                    content_preview: None,
                },
            })
        );
    }

    #[test]
    fn bytes_elicitation_resolved() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/elicitation-resolved.sse
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
    fn bytes_child_session_updated() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/polly-child-session.sse
        let ev = parse_event(&frame(
            "session.child_session.updated",
            r#"{"sequence_number":null,"type":"session.child_session.updated","conversation_id":"conv_parent","child_session_id":"conv_child","child":{"id":"conv_child","title":"claude_code:spike-hello-file","tool":"claude_code","session_name":"spike-hello-file","busy":false,"current_task_status":"launching"}}"#,
        ));
        let ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated {
            child_session_id,
            child,
        }) = ev
        else {
            panic!("expected ChildSessionUpdated, got {ev:?}");
        };
        assert_eq!(child_session_id, "conv_child");
        assert_eq!(child.id(), Some("conv_child"));
        assert_eq!(child.title(), Some("claude_code:spike-hello-file"));
        assert_eq!(child.tool(), Some("claude_code"));
        assert_eq!(child.session_name(), Some("spike-hello-file"));
        assert_eq!(child.busy(), Some(false));
        assert_eq!(
            child.current_task_status(),
            Some(ChildTaskStatus::Launching)
        );
    }

    #[test]
    fn sparse_child_session_delta_stays_typed() {
        // Mixes MISSING fields (tool, session_name) with explicit `null`
        // (title, busy) — both must deserialize to `None`, not drop to Unknown.
        let ev = parse_event(&frame(
            "session.child_session.updated",
            r#"{"conversation_id":"conv_parent","child_session_id":"conv_child","child":{"id":"conv_child","title":null,"busy":null,"current_task_status":"in_progress"}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated {
                child_session_id: "conv_child".into(),
                child: ChildSession {
                    id: Some("conv_child".into()),
                    title: None,
                    tool: None,
                    session_name: None,
                    busy: None,
                    current_task_status: Some(ChildTaskStatus::InProgress),
                },
            })
        );
    }

    #[test]
    fn child_task_status_unknown_for_novel_value() {
        // dev0 churn safety: an unknown status string degrades, never panics.
        let ev = parse_event(&frame(
            "session.child_session.updated",
            r#"{"sequence_number":null,"type":"session.child_session.updated","conversation_id":"c","child_session_id":"cc","child":{"id":"cc","title":"t","tool":"claude_code","session_name":"n","busy":true,"current_task_status":"some_future_state"}}"#,
        ));
        let ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated { child, .. }) = ev else {
            panic!("expected ChildSessionUpdated");
        };
        assert_eq!(child.current_task_status(), Some(ChildTaskStatus::Unknown));
    }

    #[test]
    fn bytes_terminal_activity() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/claude-native-turn.sse
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
    fn bytes_session_model() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/chrome-model-effort-compact.sse
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
    fn bytes_session_todos() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/claude-native-todos.sse
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
    fn bytes_reasoning_effort() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/chrome-model-effort-compact.sse
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
    fn bytes_skills() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/interrupt-cancelled.sse
        let ev = parse_event(&frame(
            "session.skills",
            r#"{"conversation_id":"conv_abc"}"#,
        ));
        assert_eq!(ev, ServerStreamEvent::Session(SessionEvent::Skills));
    }

    #[test]
    fn bytes_session_agent_changed() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/agent-switched.sse
        let ev = parse_event(&frame(
            "session.agent_changed",
            r#"{"sequence_number":null,"type":"session.agent_changed","conversation_id":"conv_2a9","agent_id":"ag_2e9","agent_name":"debby"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::AgentChanged {
                agent_id: "ag_2e9".into(),
                agent_name: "debby".into(),
            })
        );
    }

    #[test]
    fn bytes_session_created_child() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/polly-child-session.sse
        let ev = parse_event(&frame(
            "session.created",
            r#"{"sequence_number":null,"type":"session.created","conversation_id":"conv_parent","child_session_id":"conv_child","agent_id":"ag_b","parent_session_id":"conv_parent"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Created {
                child_session_id: "conv_child".into(),
                agent_id: Some("ag_b".into()),
                parent_session_id: Some("conv_parent".into()),
            })
        );
    }

    #[test]
    fn session_created_nullable_ids_stay_typed() {
        let ev = parse_event(&frame(
            "session.created",
            r#"{"conversation_id":"conv_parent","child_session_id":"conv_child","agent_id":null,"parent_session_id":null}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Created {
                child_session_id: "conv_child".into(),
                agent_id: None,
                parent_session_id: None,
            })
        );
    }

    #[test]
    fn schema_mcp_startup_flattens_server_map() {
        // SCHEMA-DERIVED (0.5.0): map<name, {status, error?}> → deterministic Vec.
        let ev = parse_event(&frame(
            "session.mcp_startup",
            r#"{"conversation_id":"conv_abc","servers":{"safe":{"status":"starting","error":null},"git":{"status":"failed","error":"handshaking with MCP server failed"}}}"#,
        ));
        let ServerStreamEvent::Session(SessionEvent::McpStartup { servers }) = ev else {
            panic!("expected McpStartup, got {ev:?}");
        };
        // BTreeMap ordering: "git" before "safe".
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name(), "git");
        assert_eq!(servers[0].status(), McpStartupStatus::Failed);
        assert_eq!(
            servers[0].error(),
            Some("handshaking with MCP server failed")
        );
        assert_eq!(servers[1].name(), "safe");
        assert_eq!(servers[1].status(), McpStartupStatus::Starting);
        assert_eq!(servers[1].error(), None);
    }

    #[test]
    fn mcp_startup_unknown_status_degrades_not_panics() {
        // dev0 churn safety: a novel status string maps to Unknown.
        let ev = parse_event(&frame(
            "session.mcp_startup",
            r#"{"conversation_id":"c","servers":{"safe":{"status":"reticulating"}}}"#,
        ));
        let ServerStreamEvent::Session(SessionEvent::McpStartup { servers }) = ev else {
            panic!("expected McpStartup");
        };
        assert_eq!(servers[0].status(), McpStartupStatus::Unknown);
        assert_eq!(servers[0].error(), None);
    }

    #[test]
    fn mcp_startup_missing_required_servers_degrades_to_unknown() {
        // `servers` is contract-required; a frame lacking it must not fabricate an
        // empty McpStartup — it degrades to Unknown (the crate-bump alarm).
        let ev = parse_event(&frame("session.mcp_startup", r#"{"conversation_id":"c"}"#));
        assert_eq!(
            ev,
            ServerStreamEvent::Unknown {
                event_type: "session.mcp_startup".into()
            }
        );
    }

    #[test]
    fn schema_policy_denied() {
        // SCHEMA-DERIVED (0.5.0): a native DENY surfaced on the stream.
        let ev = parse_event(&frame(
            "response.policy_denied",
            r#"{"conversation_id":"conv_abc","phase":"tool_call","reason":"Blocked by policy."}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::PolicyDenied {
                phase: "tool_call".into(),
                reason: "Blocked by policy.".into(),
            })
        );
    }

    #[test]
    fn policy_denied_optional_fields_default_empty() {
        // phase/reason carry server-side "" defaults; omission must not drop to Unknown.
        let ev = parse_event(&frame(
            "response.policy_denied",
            r#"{"conversation_id":"conv_abc"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::PolicyDenied {
                phase: String::new(),
                reason: String::new(),
            })
        );
    }

    #[test]
    fn from_value_retains_response_id_on_message() {
        // Byte-verified: docs/spikes/captures/2026-07-21-t0-verify/turn2.stream.sse
        let v = serde_json::json!({
            "id": "msg_165176d0b88d46f5ba570e1ebfa73e3f",
            "response_id": "resp_bcb93365f7aa4a0c9177e142",
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Mercury\nVenus\nEarth\nMars"}],
            "model": "claude-sdk"
        });
        let item = Item::from_value(v);
        assert_eq!(item.response_id(), Some("resp_bcb93365f7aa4a0c9177e142"));
    }

    #[test]
    fn from_value_response_id_absent_is_none() {
        let v = serde_json::json!({
            "id": "item_abc",
            "type": "message",
            "role": "assistant",
            "content": []
        });
        let item = Item::from_value(v);
        assert_eq!(item.response_id(), None);
    }

    #[test]
    fn from_value_response_id_empty_string_is_none() {
        let v = serde_json::json!({
            "id": "item_abc",
            "type": "message",
            "role": "assistant",
            "content": [],
            "response_id": ""
        });
        let item = Item::from_value(v);
        assert_eq!(item.response_id(), None);
    }

    #[test]
    fn from_value_retains_response_id_on_resource_event() {
        // Byte-verified: docs/spikes/captures/2026-07-21-t0-verify/items_endpoint.no-created_at.json
        let v = serde_json::json!({
            "id": "rse_024ea4e882de4f6688d23712303e3278",
            "response_id": "conv_599b6d156fd44a8886c200d9d55c7758",
            "type": "resource_event",
            "status": "completed",
            "event_type": "session.resource.created",
            "resource_id": "terminal_tui_main",
            "resource_type": "terminal"
        });
        let item = Item::from_value(v);
        assert_eq!(
            item.response_id(),
            Some("conv_599b6d156fd44a8886c200d9d55c7758")
        );
    }

    #[test]
    fn in_progress_carries_response_id() {
        // Byte-verified: docs/spikes/captures/2026-07-21-t0-verify/interrupt-then-retry.stream.sse
        let ev = parse_event(&frame(
            "response.in_progress",
            r#"{"sequence_number": 1, "type": "response.in_progress", "response": {"id": "resp_37ba30e3a06240e4bc1de44a", "object": "response", "status": "in_progress", "model": "claude-sdk", "created_at": 1784660489, "completed_at": null, "output": [], "background": false, "store": true, "usage": null, "previous_response_id": null, "conversation": null, "instructions": null, "reasoning": null, "error": null, "incomplete_details": null}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::InProgress { response_id }) => {
                assert_eq!(
                    response_id.as_deref(),
                    Some("resp_37ba30e3a06240e4bc1de44a")
                );
            }
            other => panic!("expected InProgress, got {other:?}"),
        }
    }

    #[test]
    fn from_value_retains_response_id_on_function_call() {
        let v = serde_json::json!({
            "id": "fc_8a2f1c9e4b7d46a3b0e5f812c6d9047a",
            "response_id": "resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6",
            "type": "function_call",
            "call_id": "toolu_01K8X2Y3Z4A5B6C7D8E9F0G1H2",
            "name": "sys_os_shell",
            "arguments": "{\"command\":\"pwd\"}",
            "status": "completed"
        });
        let item = Item::from_value(v);
        assert_eq!(
            item.response_id(),
            Some("resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6")
        );
    }

    #[test]
    fn from_value_retains_response_id_on_function_call_output() {
        let v = serde_json::json!({
            "id": "fco_5d9e3b7a1c4f48e2a6b8d0c3e5f71234",
            "response_id": "resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6",
            "type": "function_call_output",
            "call_id": "toolu_01K8X2Y3Z4A5B6C7D8E9F0G1H2",
            "output": "/Users/dev/project"
        });
        let item = Item::from_value(v);
        assert_eq!(
            item.response_id(),
            Some("resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6")
        );
    }

    #[test]
    fn from_value_retains_response_id_on_error() {
        let v = serde_json::json!({
            "id": "err_2c4f6a8b0d1e3f5a7b9c1d3e5f7a9b1c",
            "response_id": "resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6",
            "type": "error",
            "status": "completed",
            "data": {
                "source": "execution",
                "code": "RuntimeError",
                "message": "command failed"
            }
        });
        let item = Item::from_value(v);
        assert_eq!(
            item.response_id(),
            Some("resp_3f7e2a91c8b54d6e0a1f4325b9c8d7e6")
        );
    }

    #[test]
    fn in_progress_response_id_absent_is_none() {
        let ev = parse_event(&frame(
            "response.in_progress",
            r#"{"sequence_number": 1, "type": "response.in_progress", "response": {"object": "response", "status": "in_progress", "model": "claude-sdk", "created_at": 1784660489, "completed_at": null, "output": [], "background": false, "store": true, "usage": null, "previous_response_id": null, "conversation": null, "instructions": null, "reasoning": null, "error": null, "incomplete_details": null}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::InProgress { response_id }) => {
                assert!(response_id.is_none());
            }
            other => panic!("expected InProgress, got {other:?}"),
        }
    }

    #[test]
    fn in_progress_response_id_empty_string_is_none() {
        let ev = parse_event(&frame(
            "response.in_progress",
            r#"{"sequence_number": 1, "type": "response.in_progress", "response": {"id": "", "object": "response", "status": "in_progress", "model": "claude-sdk", "created_at": 1784660489, "completed_at": null, "output": [], "background": false, "store": true, "usage": null, "previous_response_id": null, "conversation": null, "instructions": null, "reasoning": null, "error": null, "incomplete_details": null}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::InProgress { response_id }) => {
                assert!(response_id.is_none());
            }
            other => panic!("expected InProgress, got {other:?}"),
        }
    }

    #[test]
    fn bytes_session_resource_deleted() {
        // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/agent-switched.sse
        let ev = parse_event(&frame(
            "session.resource.deleted",
            r#"{"sequence_number":null,"type":"session.resource.deleted","resource_id":"terminal_tui_main","resource_type":"terminal","session_id":"conv_2a9"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ResourceDeleted {
                resource_id: "terminal_tui_main".into(),
                resource_type: "terminal".into(),
            })
        );
    }
}
