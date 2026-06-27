//! The `Sessions` subservice and the generalized `/events` write path.
//!
//! `SessionEventInput` here is the **hand-written** typed enum for the subset of
//! events Lens sends — distinct from `crate::generated::SessionEventInput`, which
//! is the raw `{type, data, model_override, tools}` wire container. Discriminators
//! and payload shapes are pinned to omnigent 0.3.0.dev0 source
//! (`server/routes/sessions.py`, `entities/conversation.py`); never guess them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::client::Client;
use crate::error::Result;
use crate::http::decode_json;
use std::collections::BTreeMap;

use crate::ids::{ElicitationId, FileId, SessionId};

/// Session status as reported by the REST surface (snapshot + list). Only three
/// values reach REST; the server collapses `waiting`→`running` and never emits
/// `launching` on parents (`sessions.py:1792-1811`). The richer 5-value SSE
/// status (`SessionStatusEvent`) is modeled separately in the streaming plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Running,
    Failed,
}

/// This server emits explicit `null` (not `[]`/`{}`) for empty collections on
/// some snapshots; `#[serde(default)]` covers a *missing* key but NOT a present
/// `null`, so map `null` → `Default` for the collection fields.
fn de_null_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

fn de_items<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<crate::stream::Item>, D::Error> {
    let raw: Vec<serde_json::Value> =
        Option::<Vec<serde_json::Value>>::deserialize(d)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .map(hoist_embedded_item_payload)
        .map(crate::stream::Item::from_value)
        .collect())
}

/// The embedded `items` on a `SessionSnapshot` nest their payload under a `data`
/// envelope (`{id, type, status, data: {role, content, …}}`), whereas the
/// standalone `GET /items` corpus and the live stream carry those payload fields
/// at the top level — the shape `Item::from_value` reads. Hoist `data`'s fields
/// up so embedded items type the same as the flat form; `data` wins on the
/// payload keys, while `id`/`type`/`status` are preserved from the top level
/// (they never appear inside `data`). Non-object or `data`-less values pass
/// through unchanged.
fn hoist_embedded_item_payload(v: Value) -> Value {
    match v {
        Value::Object(mut top) => {
            if let Some(Value::Object(data)) = top.remove("data") {
                top.extend(data);
            }
            Value::Object(top)
        }
        other => other,
    }
}

/// A session snapshot (`GET /v1/sessions/{id}`). Mirrors the CORE fields of
/// omnigent's `SessionResponse` (`schemas.py:1601-1642`); unmodeled fields are
/// ignored. Fields are private — access is via the typed getters, so the wire
/// shape stays an lens-client implementation detail (single edit site for drift).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct SessionSnapshot {
    id: SessionId,
    status: SessionStatus,
    agent_id: String,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    archived: bool,
    created_at: i64,
    #[serde(default, deserialize_with = "de_null_default")]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    runner_online: Option<bool>,
    #[serde(default)]
    host_online: Option<bool>,
    #[serde(default)]
    host_resumable: bool,
    #[serde(default)]
    harness: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    runner_id: Option<String>,
    #[serde(default)]
    host_id: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    context_window: Option<i64>,
    #[serde(default)]
    last_total_tokens: Option<i64>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    permission_level: Option<i64>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    git_branch: Option<String>,
    #[serde(default)]
    root_conversation_id: Option<String>,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    sub_agent_name: Option<String>,
    // ⚠ DEFERRED: last_task_error — null in the capture, but the sibling
    //   ChildSessionSummary models it as Option<BTreeMap<String,String>>
    //   (sessions.rs:309). Its non-string live shape would fail the whole
    //   snapshot deser, so it is left out (serde skips the unknown wire field).
    //   Model when a non-null shape is captured.
    #[serde(default, deserialize_with = "de_null_default")]
    usage_by_model: std::collections::BTreeMap<String, ModelUsage>,
    #[serde(default, deserialize_with = "de_null_default")]
    skills: Vec<SkillRef>,
    #[serde(default, deserialize_with = "de_items")]
    items: Vec<crate::stream::Item>,
    // ⚠ DEFERRED (empty/null in the only capture — model when non-empty, Plan 3b-2b/
    //   config-time): todos (TodoItem is not Deserialize; wire key `activeForm`),
    //   pending_elicitations (likely objects, not id strings), model_options,
    //   sandbox_status. Left out of the struct: serde skips unknown wire fields,
    //   so the snapshot still parses with them present-but-empty.
}

impl SessionSnapshot {
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn status(&self) -> SessionStatus {
        self.status
    }
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn agent_name(&self) -> Option<&str> {
        self.agent_name.as_deref()
    }
    pub fn archived(&self) -> bool {
        self.archived
    }
    /// Creation time, epoch **seconds**.
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }
    /// `Some` only when the snapshot was fetched with `include_liveness` (default true).
    pub fn runner_online(&self) -> Option<bool> {
        self.runner_online
    }
    pub fn host_online(&self) -> Option<bool> {
        self.host_online
    }
    pub fn host_resumable(&self) -> bool {
        self.host_resumable
    }
    pub fn harness(&self) -> &str {
        &self.harness
    }
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    pub fn runner_id(&self) -> Option<&str> {
        self.runner_id.as_deref()
    }
    pub fn host_id(&self) -> Option<&str> {
        self.host_id.as_deref()
    }
    pub fn llm_model(&self) -> Option<&str> {
        self.llm_model.as_deref()
    }
    pub fn model_override(&self) -> Option<&str> {
        self.model_override.as_deref()
    }
    pub fn reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }
    pub fn context_window(&self) -> Option<i64> {
        self.context_window
    }
    pub fn last_total_tokens(&self) -> Option<i64> {
        self.last_total_tokens
    }
    pub fn total_cost_usd(&self) -> Option<f64> {
        self.total_cost_usd
    }
    pub fn permission_level(&self) -> Option<i64> {
        self.permission_level
    }
    pub fn workspace(&self) -> Option<&str> {
        self.workspace.as_deref()
    }
    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }
    pub fn root_conversation_id(&self) -> Option<&str> {
        self.root_conversation_id.as_deref()
    }
    pub fn parent_session_id(&self) -> Option<&str> {
        self.parent_session_id.as_deref()
    }
    pub fn sub_agent_name(&self) -> Option<&str> {
        self.sub_agent_name.as_deref()
    }
    pub fn usage_by_model(&self) -> &std::collections::BTreeMap<String, ModelUsage> {
        &self.usage_by_model
    }
    pub fn skills(&self) -> &[SkillRef] {
        &self.skills
    }
    /// The transcript items embedded in the snapshot — non-empty only when fetched
    /// with `GetOpts { include_items: true }`. The standalone paginated read is
    /// `Sessions::items()`.
    pub fn items(&self) -> &[crate::stream::Item] {
        &self.items
    }
}

/// Per-model token+cost usage from `usage_by_model` on the session snapshot.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct ModelUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    total_cost_usd: f64,
}

impl ModelUsage {
    pub fn input_tokens(&self) -> i64 {
        self.input_tokens
    }
    pub fn output_tokens(&self) -> i64 {
        self.output_tokens
    }
    pub fn total_tokens(&self) -> i64 {
        self.total_tokens
    }
    pub fn cache_read_input_tokens(&self) -> i64 {
        self.cache_read_input_tokens
    }
    pub fn cache_creation_input_tokens(&self) -> i64 {
        self.cache_creation_input_tokens
    }
    pub fn total_cost_usd(&self) -> f64 {
        self.total_cost_usd
    }
}

/// An attached skill summary from `skills` on the session snapshot.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct SkillRef {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

impl SkillRef {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Options for `Sessions::get`. Defaults: liveness on, items off, no refresh.
#[derive(Clone, Copy, Debug)]
pub struct GetOpts {
    pub include_items: bool,
    pub include_liveness: bool,
    pub refresh_state: bool,
}

impl Default for GetOpts {
    fn default() -> Self {
        Self {
            include_items: false,
            include_liveness: true,
            refresh_state: false,
        }
    }
}

impl GetOpts {
    pub(crate) fn to_query(self) -> Vec<(&'static str, String)> {
        vec![
            ("include_items", self.include_items.to_string()),
            ("include_liveness", self.include_liveness.to_string()),
            ("refresh_state", self.refresh_state.to_string()),
        ]
    }
}

/// One element of `GET /v1/sessions` (omnigent `SessionListItem`, `schemas.py:1866-1885`).
/// Like a snapshot minus liveness, plus `updated_at`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionSummary {
    id: SessionId,
    status: SessionStatus,
    agent_id: String,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    archived: bool,
    created_at: i64,
    updated_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

impl SessionSummary {
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn status(&self) -> SessionStatus {
        self.status
    }
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn agent_name(&self) -> Option<&str> {
        self.agent_name.as_deref()
    }
    pub fn archived(&self) -> bool {
        self.archived
    }
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }
    pub fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }
}

/// `GET /v1/sessions` — a `PaginatedList` of summaries.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionList {
    pub data: Vec<SessionSummary>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

/// `GET /v1/sessions/{id}/items` — the durable, paginated transcript. Persisted
/// items carry no `sequence_number`; reconcile by `Item::id()` (typed-client §7).
#[derive(Debug)]
pub struct ItemList {
    items: Vec<crate::stream::Item>,
    has_more: bool,
    first_id: Option<String>,
    last_id: Option<String>,
}

impl ItemList {
    pub fn items(&self) -> &[crate::stream::Item] {
        &self.items
    }
    pub(crate) fn into_items(self) -> Vec<crate::stream::Item> {
        self.items
    }
    pub fn has_more(&self) -> bool {
        self.has_more
    }
    pub fn first_id(&self) -> Option<&str> {
        self.first_id.as_deref()
    }
    pub fn last_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }
}

// Internal wire envelope (private; `data` is Value only to feed Item::from_value).
#[derive(serde::Deserialize)]
struct RawItemList {
    #[serde(default)]
    data: Vec<serde_json::Value>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    first_id: Option<String>,
    #[serde(default)]
    last_id: Option<String>,
}

impl<'de> serde::Deserialize<'de> for ItemList {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = RawItemList::deserialize(d)?;
        Ok(ItemList {
            items: raw
                .data
                .into_iter()
                .map(crate::stream::Item::from_value)
                .collect(),
            has_more: raw.has_more,
            first_id: raw.first_id,
            last_id: raw.last_id,
        })
    }
}

/// Pagination for `Sessions::items` (openapi `/v1/sessions/{id}/items`: limit,
/// after, before, order). All optional; `None` fields are omitted from the query.
#[derive(Debug, Default, Clone)]
pub struct ItemsPage {
    pub limit: Option<u32>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub order: Option<String>,
}

impl ItemsPage {
    pub(crate) fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(n) = self.limit {
            q.push(("limit", n.to_string()));
        }
        if let Some(a) = &self.after {
            q.push(("after", a.clone()));
        }
        if let Some(b) = &self.before {
            q.push(("before", b.clone()));
        }
        if let Some(o) = &self.order {
            q.push(("order", o.clone()));
        }
        q
    }
}

/// `kind` filter for the fleet poll.
#[derive(Clone, Copy, Debug)]
pub enum SessionKind {
    Default,
    SubAgent,
    Any,
}

impl SessionKind {
    fn as_str(self) -> &'static str {
        match self {
            SessionKind::Default => "default",
            SessionKind::SubAgent => "sub_agent",
            SessionKind::Any => "any",
        }
    }
}

/// Query filter for `Sessions::list`. All fields optional; unset → omitted.
#[derive(Clone, Debug, Default)]
pub struct SessionFilter {
    limit: Option<u32>,
    after: Option<String>,
    before: Option<String>,
    agent_id: Option<String>,
    agent_name: Option<String>,
    search_query: Option<String>,
    include_archived: Option<bool>,
    kind: Option<SessionKind>,
}

impl SessionFilter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }
    pub fn after(mut self, c: impl Into<String>) -> Self {
        self.after = Some(c.into());
        self
    }
    pub fn before(mut self, c: impl Into<String>) -> Self {
        self.before = Some(c.into());
        self
    }
    pub fn agent_id(mut self, v: impl Into<String>) -> Self {
        self.agent_id = Some(v.into());
        self
    }
    pub fn agent_name(mut self, v: impl Into<String>) -> Self {
        self.agent_name = Some(v.into());
        self
    }
    pub fn search(mut self, v: impl Into<String>) -> Self {
        self.search_query = Some(v.into());
        self
    }
    pub fn include_archived(mut self, v: bool) -> Self {
        self.include_archived = Some(v);
        self
    }
    pub fn kind(mut self, k: SessionKind) -> Self {
        self.kind = Some(k);
        self
    }

    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(n) = self.limit {
            q.push(("limit", n.to_string()));
        }
        if let Some(v) = &self.after {
            q.push(("after", v.clone()));
        }
        if let Some(v) = &self.before {
            q.push(("before", v.clone()));
        }
        if let Some(v) = &self.agent_id {
            q.push(("agent_id", v.clone()));
        }
        if let Some(v) = &self.agent_name {
            q.push(("agent_name", v.clone()));
        }
        if let Some(v) = &self.search_query {
            q.push(("search_query", v.clone()));
        }
        if let Some(v) = self.include_archived {
            q.push(("include_archived", v.to_string()));
        }
        if let Some(k) = self.kind {
            q.push(("kind", k.as_str().to_string()));
        }
        q
    }
}

/// Mirror of omnigent `ChildSessionSummary` (`schemas.py:558-664`). Not in
/// `openapi.json` `components` — hand-written from source and contract-tested.
/// The live `session.child_session.updated` event carries a PARTIAL of this
/// shape, so fields the event may omit default rather than error (the state
/// model merges present fields over the cached child row).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ChildSessionSummary {
    id: SessionId,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    parent_session_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    session_name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    current_task_id: Option<String>,
    #[serde(default)]
    current_task_status: Option<String>,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    last_task_error: Option<BTreeMap<String, String>>,
    #[serde(default)]
    last_message_preview: Option<String>,
    #[serde(default)]
    pending_elicitations_count: i64,
}

impl ChildSessionSummary {
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn object(&self) -> Option<&str> {
        self.object.as_deref()
    }
    pub fn parent_session_id(&self) -> &str {
        &self.parent_session_id
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
    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }
    pub fn agent_name(&self) -> Option<&str> {
        self.agent_name.as_deref()
    }
    pub fn current_task_id(&self) -> Option<&str> {
        self.current_task_id.as_deref()
    }
    pub fn current_task_status(&self) -> Option<&str> {
        self.current_task_status.as_deref()
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }
    pub fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }
    pub fn last_task_error(&self) -> Option<&BTreeMap<String, String>> {
        self.last_task_error.as_ref()
    }
    pub fn last_message_preview(&self) -> Option<&str> {
        self.last_message_preview.as_deref()
    }
    pub fn pending_elicitations_count(&self) -> i64 {
        self.pending_elicitations_count
    }
}

/// `GET /v1/sessions/{id}/child_sessions` — paginated child summaries.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ChildSessionList {
    pub data: Vec<ChildSessionSummary>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

/// Ack for `POST /v1/sessions/{id}/events` (HTTP 202). The openapi declares an
/// empty body, but the route always returns a small JSON ack — model it with
/// defaults so an empty or future-extended body still deserializes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendEventAck {
    /// Whether the event was queued to the runner. Control events report `false`.
    #[serde(default)]
    pub queued: bool,
    /// Store item id for persisted item events (`message`, …). For
    /// `function_call_output` this echoes the `call_id`.
    #[serde(default)]
    pub item_id: Option<String>,
    /// Pending id for the native-terminal `message` bypass path.
    #[serde(default)]
    pub pending_id: Option<String>,
    /// Set when a policy denied a user `message`.
    #[serde(default)]
    pub denied: bool,
    /// Human-readable denial reason (paired with `denied`).
    #[serde(default)]
    pub reason: Option<String>,
    /// Elicitation id for the `mcp_elicitation` path.
    #[serde(default)]
    pub elicitation_id: Option<String>,
}

/// Consumer reply action for an `approval` event (MCP `ElicitResult` semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

impl ElicitationAction {
    fn as_str(self) -> &'static str {
        match self {
            ElicitationAction::Accept => "accept",
            ElicitationAction::Decline => "decline",
            ElicitationAction::Cancel => "cancel",
        }
    }
}

/// A client-submitted session event. Only the subset Lens sends is modeled;
/// the server accepts a larger dispatch table (pinned in `ALLOWED_EVENT_TYPES`).
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEventInput {
    /// A user message. `content` is a list of open content blocks, e.g.
    /// `{"type":"input_text","text":"…"}`. `role` is always `"user"` on send.
    Message {
        content: Vec<Value>,
        model_override: Option<String>,
        tools: Option<Vec<Value>>,
    },
    /// A client-side tool result.
    FunctionCallOutput { call_id: String, output: String },
    /// Reply to an outstanding elicitation. Wire `data` is flat:
    /// `{elicitation_id, action, content?}`.
    Approval {
        elicitation_id: ElicitationId,
        action: ElicitationAction,
        content: Option<Map<String, Value>>,
    },
    /// Interrupt the active turn.
    Interrupt,
    /// Request context compaction (control event `"compact"`, not the
    /// `"compaction"` item type).
    Compact,
    /// Terminate the live session (reclaim the runner). Owner-gated server-side.
    StopSession,
}

impl SessionEventInput {
    /// The wire `type` discriminator.
    pub fn discriminator(&self) -> &'static str {
        match self {
            SessionEventInput::Message { .. } => "message",
            SessionEventInput::FunctionCallOutput { .. } => "function_call_output",
            SessionEventInput::Approval { .. } => "approval",
            SessionEventInput::Interrupt => "interrupt",
            SessionEventInput::Compact => "compact",
            SessionEventInput::StopSession => "stop_session",
        }
    }

    /// Serialize into the wire envelope: `{ "type": <discrim>, "data": <payload>, .. }`.
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("type".into(), json!(self.discriminator()));

        let data: Value = match self {
            SessionEventInput::Message { content, .. } => {
                json!({ "role": "user", "content": content })
            }
            SessionEventInput::FunctionCallOutput { call_id, output } => {
                json!({ "call_id": call_id, "output": output })
            }
            SessionEventInput::Approval {
                elicitation_id,
                action,
                content,
            } => {
                let mut d = Map::new();
                d.insert("elicitation_id".into(), json!(elicitation_id.as_str()));
                d.insert("action".into(), json!(action.as_str()));
                if let Some(c) = content {
                    d.insert("content".into(), Value::Object(c.clone()));
                }
                Value::Object(d)
            }
            SessionEventInput::Interrupt
            | SessionEventInput::Compact
            | SessionEventInput::StopSession => json!({}),
        };
        obj.insert("data".into(), data);

        // `model_override` / `tools` are envelope-level and only meaningful for `message`.
        if let SessionEventInput::Message {
            model_override,
            tools,
            ..
        } = self
        {
            if let Some(m) = model_override {
                obj.insert("model_override".into(), json!(m));
            }
            if let Some(t) = tools {
                obj.insert("tools".into(), json!(t));
            }
        }

        Value::Object(obj)
    }
}

/// The full set of `type` discriminators the server's `POST /events` route
/// accepts (`_ALLOWED_EVENT_TYPES`, omnigent 0.3.0.dev0). Lens only *sends* the
/// six modeled by `SessionEventInput`, but the contract test pins the whole set
/// so a re-vendor that adds/removes a type is a conscious change. Kept sorted.
pub const ALLOWED_EVENT_TYPES: [&str; 30] = [
    "approval",
    "compact",
    "compaction",
    "error",
    "external_assistant_message",
    "external_codex_collaboration_mode_change",
    "external_codex_subagent_start",
    "external_compaction_status",
    "external_conversation_item",
    "external_elicitation_resolved",
    "external_model_change",
    "external_output_reasoning_delta",
    "external_output_text_delta",
    "external_reasoning_effort_change",
    "external_session_interrupted",
    "external_session_status",
    "external_session_todos",
    "external_session_usage",
    "external_subagent_start",
    "function_call",
    "function_call_output",
    "interrupt",
    "mcp_elicitation",
    "message",
    "native_tool",
    "reasoning",
    "resource_event",
    "slash_command",
    "stop_session",
    "terminal_command",
];

/// Host placement for a new session.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostType {
    External,
    Managed,
}

/// JSON body for `POST /v1/sessions` (omnigent `SessionCreateRequest`,
/// `schemas.py:1038-1155`). Not in `openapi.json` components — hand-written.
/// Only `agent_id` is required; unset fields are omitted (server defaults apply).
#[derive(Clone, Debug, serde::Serialize)]
pub struct CreateSessionRequest {
    agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_type: Option<HostType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_items: Option<Vec<serde_json::Value>>,
}

impl CreateSessionRequest {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            host_type: None,
            host_id: None,
            workspace: None,
            git: None,
            initial_items: None,
        }
    }
    pub fn host_type(mut self, h: HostType) -> Self {
        self.host_type = Some(h);
        self
    }
    pub fn host_id(mut self, v: impl Into<String>) -> Self {
        self.host_id = Some(v.into());
        self
    }
    pub fn workspace(mut self, v: impl Into<String>) -> Self {
        self.workspace = Some(v.into());
        self
    }
    pub fn git(mut self, branch_name: impl Into<String>, base_branch: Option<&str>) -> Self {
        let mut g = serde_json::Map::new();
        g.insert("branch_name".into(), serde_json::json!(branch_name.into()));
        if let Some(b) = base_branch {
            g.insert("base_branch".into(), serde_json::json!(b));
        }
        self.git = Some(serde_json::Value::Object(g));
        self
    }
    /// `initial_items` are `SessionEventInput`-shaped; build via `SessionEventInput::to_json`.
    pub fn initial_items(mut self, items: Vec<serde_json::Value>) -> Self {
        self.initial_items = Some(items);
        self
    }
}

/// Response of multipart `POST /v1/sessions` (omnigent `CreatedSessionResponse`,
/// `schemas.py:1289-1291`). Lighter than a snapshot.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreatedSessionResponse {
    session_id: SessionId,
    agent_id: String,
    agent_name: String,
}
impl CreatedSessionResponse {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn agent_name(&self) -> &str {
        &self.agent_name
    }
}

/// Response of `DELETE /v1/sessions/{id}` (omnigent `ConversationDeleted`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ConversationDeleted {
    id: SessionId,
    #[serde(default)]
    object: String,
    #[serde(default)]
    deleted: bool,
}
impl ConversationDeleted {
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn object(&self) -> &str {
        &self.object
    }
    pub fn deleted(&self) -> bool {
        self.deleted
    }
}

/// Pending elicitation state (`GET …/elicitations/{id}`). The response is
/// untyped server-side; expose typed getters for the fields the elicitation UI
/// consumes. Start with the correlation id + raw-typed status; extend as needed.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ElicitationState {
    #[serde(default)]
    elicitation_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}
impl ElicitationState {
    pub fn elicitation_id(&self) -> Option<&str> {
        self.elicitation_id.as_deref()
    }
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
}

/// A filesystem entry (`runner/app.py:14548-14556`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesystemEntry {
    id: String,
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(default)]
    bytes: Option<u64>,
    #[serde(default)]
    modified_at: Option<i64>,
}
impl FilesystemEntry {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn entry_type(&self) -> &str {
        &self.entry_type
    }
    pub fn bytes(&self) -> Option<u64> {
        self.bytes
    }
    pub fn modified_at(&self) -> Option<i64> {
        self.modified_at
    }
}

/// `{object:"list", data:[FilesystemEntry], has_more}` envelope.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesystemList {
    pub data: Vec<FilesystemEntry>,
    #[serde(default)]
    pub has_more: bool,
}

/// Query for environment search (`q` required; `include`/`exclude` globs; `limit` ≤ 500).
#[derive(Clone, Debug)]
pub struct SearchQuery {
    q: String,
    include: Option<String>,
    exclude: Option<String>,
    limit: Option<u32>,
}
impl SearchQuery {
    pub fn new(q: impl Into<String>) -> Self {
        Self {
            q: q.into(),
            include: None,
            exclude: None,
            limit: None,
        }
    }
    pub fn include(mut self, g: impl Into<String>) -> Self {
        self.include = Some(g.into());
        self
    }
    pub fn exclude(mut self, g: impl Into<String>) -> Self {
        self.exclude = Some(g.into());
        self
    }
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n.min(500));
        self
    }
    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![("q", self.q.clone())];
        if let Some(g) = &self.include {
            v.push(("include", g.clone()));
        }
        if let Some(g) = &self.exclude {
            v.push(("exclude", g.clone()));
        }
        if let Some(n) = self.limit {
            v.push(("limit", n.to_string()));
        }
        v
    }
}

/// `GET …/diff/{relative_path}` — `{before, after}` (NOT a unified diff).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileDiff {
    before: String,
    after: String,
}
impl FileDiff {
    pub fn before(&self) -> &str {
        &self.before
    }
    pub fn after(&self) -> &str {
        &self.after
    }
}

/// `GET …/filesystem/{relative_path}` — file read. ⚠ Mine the exact key names
/// (content vs base64, encoding, size) from the runner source when wiring the
/// editor; start with a `content()` getter over the field the runner returns.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileContent {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
}
impl FileContent {
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }
    pub fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }
}

/// One-shot shell result (`POST …/shell`). ⚠ Confirm field names (stdout/stderr/
/// exit_code) against the runner shell handler; these are the conventional keys.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ShellResult {
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    exit_code: Option<i64>,
}
impl ShellResult {
    pub fn stdout(&self) -> &str {
        &self.stdout
    }
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
    pub fn exit_code(&self) -> Option<i64> {
        self.exit_code
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileResource {
    id: FileId,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    bytes: Option<u64>,
}
impl FileResource {
    pub fn id(&self) -> &FileId {
        &self.id
    }
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }
    pub fn bytes(&self) -> Option<u64> {
        self.bytes
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesList {
    pub data: Vec<FileResource>,
    #[serde(default)]
    pub has_more: bool,
}

/// A generic session resource (environment/terminal/file). Untyped server-side;
/// expose id/object now, grow typed getters as the resource UI needs them.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ResourceObject {
    id: String,
    #[serde(default, rename = "object")]
    object: String,
}
impl ResourceObject {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn object(&self) -> &str {
        &self.object
    }
}

/// `GET /v1/sessions/{id}/resources` — paginated session resources. Hand-modeled
/// (not the generated type) so no `serde_json::Value` reaches consumers.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ResourceList {
    pub data: Vec<ResourceObject>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommentObject {
    #[serde(default)]
    id: Option<String>,
}
impl CommentObject {
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct OwnerInfo {
    #[serde(default)]
    user_id: Option<String>,
}
impl OwnerInfo {
    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PermissionsInfo {
    // ⚠ Untyped server-side; model the grants once the sharing UI consumes them
    // (e.g. a map of user_id -> level). Start minimal.
    #[serde(default)]
    public_level: Option<i64>,
}
impl PermissionsInfo {
    pub fn public_level(&self) -> Option<i64> {
        self.public_level
    }
}

/// The session subservice — borrows the `Client` for the duration of a call.
pub struct Sessions<'a> {
    client: &'a Client,
}

impl<'a> Sessions<'a> {
    pub(crate) fn new(client: &'a Client) -> Self {
        Self { client }
    }

    /// `GET /v1/sessions/{id}` — the session snapshot. Blocking.
    pub fn get(&self, id: &SessionId, opts: GetOpts) -> Result<SessionSnapshot> {
        self.client
            .get_json(&format!("/v1/sessions/{id}"), &opts.to_query())
    }

    /// `GET /v1/sessions` — fleet poll. Blocking.
    pub fn list(&self, filter: &SessionFilter) -> Result<SessionList> {
        self.client.get_json("/v1/sessions", &filter.to_query())
    }

    /// `GET /v1/sessions/{id}/items` — the durable transcript. Blocking.
    pub fn items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/items"), &page.to_query())
    }

    /// `GET /v1/sessions/{id}/child_sessions` — list sub-sessions. Blocking.
    pub fn child_sessions(
        &self,
        id: &SessionId,
        limit: Option<u32>,
        after: Option<&str>,
    ) -> Result<ChildSessionList> {
        let mut q = Vec::new();
        if let Some(n) = limit {
            q.push(("limit", n.to_string()));
        }
        if let Some(a) = after {
            q.push(("after", a.to_string()));
        }
        self.client
            .get_json(&format!("/v1/sessions/{id}/child_sessions"), &q)
    }

    /// `POST /v1/sessions` (JSON) — create a session against an existing agent.
    pub fn create(&self, req: &CreateSessionRequest) -> Result<SessionSnapshot> {
        self.client
            .send_json(reqwest::Method::POST, "/v1/sessions", &[], Some(req))
    }

    /// `POST /v1/sessions` (multipart) — create from an agent `bundle` (.tar.gz)
    /// with a JSON `metadata` part (omnigent `SessionCreateMetadata`).
    pub fn create_with_bundle(
        &self,
        metadata: &serde_json::Value,
        bundle: Vec<u8>,
        bundle_filename: &str,
    ) -> Result<CreatedSessionResponse> {
        let form = reqwest::blocking::multipart::Form::new()
            .text("metadata", serde_json::to_string(metadata)?)
            .part(
                "bundle",
                reqwest::blocking::multipart::Part::bytes(bundle)
                    .file_name(bundle_filename.to_string())
                    .mime_str("application/gzip")
                    .map_err(crate::error::ClientError::Network)?,
            );
        self.client
            .send_multipart(reqwest::Method::POST, "/v1/sessions", form)
    }

    /// `PATCH /v1/sessions/{id}` — update mutable session fields. Returns the
    /// updated snapshot. Build `req` from `lens_client::generated::UpdateSessionRequest`
    /// (fields: `runner_id`, `archived`, `silent`, `labels`, `model_override`,
    /// `reasoning_effort`, `collaboration_mode`, `terminal_launch_args`, …).
    pub fn patch(
        &self,
        id: &SessionId,
        req: &crate::generated::UpdateSessionRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(
            reqwest::Method::PATCH,
            &format!("/v1/sessions/{id}"),
            &[],
            Some(req),
        )
    }

    /// `DELETE /v1/sessions/{id}` — delete; `delete_branch` cleans the worktree.
    pub fn delete(&self, id: &SessionId, delete_branch: bool) -> Result<ConversationDeleted> {
        self.client.send_json::<ConversationDeleted, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}"),
            &[("delete_branch", delete_branch.to_string())],
            None,
        )
    }

    /// `POST /v1/sessions/{source_id}/fork` — clone the conversation onto a new
    /// session. Returns the new (idle) snapshot.
    pub fn fork(
        &self,
        source: &SessionId,
        req: &crate::generated::SessionForkRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{source}/fork"),
            &[],
            Some(req),
        )
    }

    /// `POST /v1/sessions/{id}/switch-agent` — switch the bound agent (fires
    /// `session.agent_changed`). Returns the updated snapshot. `req.agent_id` required.
    pub fn switch_agent(
        &self,
        id: &SessionId,
        req: &crate::generated::SessionSwitchAgentRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/switch-agent"),
            &[],
            Some(req),
        )
    }

    /// `PUT /v1/sessions/{id}/agent` — store/replace the agent bundle (storage
    /// only; does NOT fire `session.agent_changed`). Returns the stored `AgentObject`.
    pub fn put_agent(
        &self,
        id: &SessionId,
        bundle: Vec<u8>,
        bundle_filename: &str,
    ) -> Result<crate::generated::AgentObject> {
        let form = reqwest::blocking::multipart::Form::new().part(
            "bundle",
            reqwest::blocking::multipart::Part::bytes(bundle)
                .file_name(bundle_filename.to_string())
                .mime_str("application/gzip")
                .map_err(crate::error::ClientError::Network)?,
        );
        self.client.send_multipart(
            reqwest::Method::PUT,
            &format!("/v1/sessions/{id}/agent"),
            form,
        )
    }

    /// `GET /v1/sessions/{sid}/elicitations/{eid}` — deep-linkable pending state.
    pub fn elicitation(&self, sid: &SessionId, eid: &ElicitationId) -> Result<ElicitationState> {
        self.client
            .get_json(&format!("/v1/sessions/{sid}/elicitations/{eid}"), &[])
    }

    /// `POST …/elicitations/{eid}/resolve` — RESTful resolve (preferred over the
    /// `approval` event when an elicitation_id is on hand). Body is the generated
    /// `ElicitationResult {action, content?}`.
    pub fn resolve_elicitation(
        &self,
        sid: &SessionId,
        eid: &ElicitationId,
        result: &crate::generated::ElicitationResult,
    ) -> Result<crate::sessions::SendEventAck> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{sid}/elicitations/{eid}/resolve"),
            &[],
            Some(result),
        )
    }

    /// Open the live SSE event stream for a session. Live-tail, no-replay:
    /// the caller must subscribe BEFORE posting the message that should be
    /// observed (transport spike §4). Returns an `EventStream` whose reader
    /// thread is already running.
    pub fn stream(
        &self,
        id: &crate::ids::SessionId,
    ) -> crate::error::Result<crate::stream::EventStream> {
        let url = self
            .client
            .conn()
            .url(&format!("/v1/sessions/{id}/stream"))?;
        let resp = self
            .client
            .conn()
            .auth
            .apply(self.client.http().get(url))
            .send()?;
        let status = resp.status().as_u16();
        match crate::http::check_status("v1/sessions/stream", status) {
            Ok(()) => {
                // TODO(3b-2b follow-up): gated live reconnect smoke test (Task 6 step 3)
                let reopener = crate::reconnect::HttpReopener::new(self.client, id.clone());
                crate::stream::EventStream::spawn(resp, reopener)
            }
            Err(e) => Err(e),
        }
    }

    /// `POST /v1/sessions/{id}/events` — submit a typed event. Returns the
    /// server's ack (queued/item_id/denial). Blocking.
    pub fn send_event(&self, id: &SessionId, evt: &SessionEventInput) -> Result<SendEventAck> {
        let conn = self.client.conn();
        let url = conn.url(&format!("/v1/sessions/{id}/events"))?;
        let resp = conn
            .auth
            .apply(self.client.http().post(url).json(&evt.to_json()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        decode_json("sessions/events", status, &body)
    }

    /// `GET …/resources/environments/{env_id}/search` — server-side fs search.
    pub fn search(
        &self,
        id: &SessionId,
        env_id: &str,
        query: &SearchQuery,
    ) -> Result<FilesystemList> {
        self.client.get_json(
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/search"),
            &query.to_query(),
        )
    }

    pub fn list_filesystem(&self, id: &SessionId, env_id: &str) -> Result<FilesystemList> {
        self.client.get_json(
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/filesystem"),
            &[],
        )
    }

    pub fn read_file(
        &self,
        id: &SessionId,
        env_id: &str,
        relative_path: &str,
    ) -> Result<FileContent> {
        self.client.get_json(
            &format!(
                "/v1/sessions/{id}/resources/environments/{env_id}/filesystem/{relative_path}"
            ),
            &[],
        )
    }

    pub fn diff(&self, id: &SessionId, env_id: &str, relative_path: &str) -> Result<FileDiff> {
        self.client.get_json(
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/diff/{relative_path}"),
            &[],
        )
    }

    pub fn shell(&self, id: &SessionId, env_id: &str, command: &str) -> Result<ShellResult> {
        let body = serde_json::json!({ "command": command });
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/shell"),
            &[],
            Some(&body),
        )
    }

    pub fn resources(&self, id: &SessionId) -> Result<ResourceList> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/resources"), &[])
    }

    pub fn resource(&self, id: &SessionId, resource_id: &str) -> Result<ResourceObject> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/resources/{resource_id}"), &[])
    }

    pub fn environment(&self, id: &SessionId, env_id: &str) -> Result<ResourceObject> {
        self.client.get_json(
            &format!("/v1/sessions/{id}/resources/environments/{env_id}"),
            &[],
        )
    }

    pub fn files(&self, id: &SessionId) -> Result<FilesList> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/resources/files"), &[])
    }

    pub fn upload_file(
        &self,
        id: &SessionId,
        bytes: Vec<u8>,
        filename: &str,
        mime: &str,
    ) -> Result<FileResource> {
        let part = reqwest::blocking::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(crate::error::ClientError::Network)?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part);
        self.client.send_multipart(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/resources/files"),
            form,
        )
    }

    pub fn file(&self, id: &SessionId, file_id: &FileId) -> Result<FileResource> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/resources/files/{file_id}"), &[])
    }

    pub fn file_content(&self, id: &SessionId, file_id: &FileId) -> Result<Vec<u8>> {
        self.client.get_bytes(&format!(
            "/v1/sessions/{id}/resources/files/{file_id}/content"
        ))
    }

    pub fn create_terminal(
        &self,
        id: &SessionId,
        opts: &serde_json::Value,
    ) -> Result<ResourceObject> {
        // opts e.g. {"launch_args": [...]}; ⚠ confirm create body shape.
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/resources/terminals"),
            &[],
            Some(opts),
        )
    }

    pub fn delete_terminal(
        &self,
        id: &SessionId,
        terminal_id: &crate::ids::TerminalId,
    ) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}/resources/terminals/{terminal_id}"),
            &[],
            None,
        )?;
        Ok(())
    }

    pub fn transfer_terminal(
        &self,
        id: &SessionId,
        terminal_id: &crate::ids::TerminalId,
        target: &SessionId,
    ) -> Result<()> {
        let body = serde_json::json!({ "target_session_id": target.as_str() }); // ⚠ confirm body key
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/resources/terminals/{terminal_id}/transfer"),
            &[],
            Some(&body),
        )?;
        Ok(())
    }

    pub fn add_comment(
        &self,
        id: &SessionId,
        req: &crate::generated::AddCommentRequest,
    ) -> Result<CommentObject> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/comments"),
            &[],
            Some(req),
        )
    }

    pub fn edit_comment(
        &self,
        id: &SessionId,
        comment_id: &crate::ids::CommentId,
        req: &crate::generated::UpdateCommentRequest,
    ) -> Result<CommentObject> {
        self.client.send_json(
            reqwest::Method::PATCH,
            &format!("/v1/sessions/{id}/comments/{comment_id}"),
            &[],
            Some(req),
        )
    }

    pub fn delete_comment(&self, id: &SessionId, comment_id: &crate::ids::CommentId) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}/comments/{comment_id}"),
            &[],
            None,
        )?;
        Ok(())
    }

    pub fn send_comments(
        &self,
        id: &SessionId,
        req: &crate::generated::SendCommentsRequest,
    ) -> Result<()> {
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/comments/send"),
            &[],
            Some(req),
        )?;
        Ok(())
    }

    pub fn labels(&self, id: &SessionId) -> Result<crate::generated::SessionLabelsResponse> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/labels"), &[])
    }

    pub fn owner(&self, id: &SessionId) -> Result<OwnerInfo> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/owner"), &[])
    }

    pub fn permissions(&self, id: &SessionId) -> Result<PermissionsInfo> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/permissions"), &[])
    }

    /// Grant levels 1–3 only (read/edit/manage); owner (4) is not grantable (server 403s).
    pub fn grant_permission(
        &self,
        id: &SessionId,
        req: &crate::generated::GrantPermissionRequest,
    ) -> Result<()> {
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::PUT,
            &format!("/v1/sessions/{id}/permissions"),
            &[],
            Some(req),
        )?;
        Ok(())
    }

    pub fn revoke_permission(&self, id: &SessionId, target_user_id: &str) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}/permissions/{target_user_id}"),
            &[],
            None,
        )?;
        Ok(())
    }

    pub fn policies(&self, id: &SessionId) -> Result<crate::registries::PolicyList> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/policies"), &[])
    }
    pub fn create_policy(
        &self,
        id: &SessionId,
        req: &crate::generated::CreateSessionPolicyRequest,
    ) -> Result<crate::registries::PolicyObject> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/policies"),
            &[],
            Some(req),
        )
    }
    pub fn session_policy(
        &self,
        id: &SessionId,
        policy_id: &crate::ids::PolicyId,
    ) -> Result<crate::registries::PolicyObject> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/policies/{policy_id}"), &[])
    }
    pub fn delete_policy(&self, id: &SessionId, policy_id: &crate::ids::PolicyId) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}/policies/{policy_id}"),
            &[],
            None,
        )?;
        Ok(())
    }
    pub fn evaluate_policy(
        &self,
        id: &SessionId,
        input: &serde_json::Value,
    ) -> Result<crate::registries::PolicyEvaluation> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{id}/policies/evaluate"),
            &[],
            Some(input),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ElicitationId;
    use serde_json::json;

    #[test]
    fn message_serializes_role_user_and_content() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "Hello"})],
            model_override: None,
            tools: None,
        };
        assert_eq!(evt.discriminator(), "message");
        assert_eq!(
            evt.to_json(),
            json!({
                "type": "message",
                "data": {"role": "user", "content": [{"type": "input_text", "text": "Hello"}]}
            })
        );
    }

    #[test]
    fn message_includes_model_override_and_tools_when_present() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "hi"})],
            model_override: Some("anthropic/claude".into()),
            tools: Some(vec![json!({"type": "function", "function": {"name": "f"}})]),
        };
        let v = evt.to_json();
        assert_eq!(v["model_override"], json!("anthropic/claude"));
        assert_eq!(
            v["tools"],
            json!([{"type": "function", "function": {"name": "f"}}])
        );
    }

    #[test]
    fn function_call_output_carries_call_id_and_output() {
        let evt = SessionEventInput::FunctionCallOutput {
            call_id: "call_abc".into(),
            output: "{\"ok\":true}".into(),
        };
        assert_eq!(evt.discriminator(), "function_call_output");
        assert_eq!(
            evt.to_json(),
            json!({"type": "function_call_output", "data": {"call_id": "call_abc", "output": "{\"ok\":true}"}})
        );
    }

    #[test]
    fn approval_is_flat_with_elicitation_id_and_action() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_1"),
            action: ElicitationAction::Accept,
            content: Some(serde_json::Map::from_iter([(
                "choice".to_string(),
                json!("a"),
            )])),
        };
        assert_eq!(evt.discriminator(), "approval");
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_1", "action": "accept", "content": {"choice": "a"}}})
        );
    }

    #[test]
    fn approval_omits_content_when_none() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_2"),
            action: ElicitationAction::Decline,
            content: None,
        };
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_2", "action": "decline"}})
        );
    }

    #[test]
    fn control_events_send_empty_data() {
        for (evt, ty) in [
            (SessionEventInput::Interrupt, "interrupt"),
            (SessionEventInput::Compact, "compact"),
            (SessionEventInput::StopSession, "stop_session"),
        ] {
            assert_eq!(evt.discriminator(), ty);
            assert_eq!(evt.to_json(), json!({"type": ty, "data": {}}));
        }
    }

    #[test]
    fn allowed_event_types_is_the_pinned_30() {
        // Pinned to omnigent 0.3.0.dev0 (sessions.py _ALLOWED_EVENT_TYPES,
        // = ITEM_TYPE_TO_DATA_CLS keys ∪ control/external extras). Sorted.
        assert_eq!(ALLOWED_EVENT_TYPES.len(), 30);
        let mut sorted = ALLOWED_EVENT_TYPES;
        sorted.sort_unstable();
        assert_eq!(
            sorted, ALLOWED_EVENT_TYPES,
            "keep ALLOWED_EVENT_TYPES sorted"
        );
    }

    #[test]
    fn every_sent_discriminator_is_server_allowed() {
        for evt in [
            SessionEventInput::Message {
                content: vec![],
                model_override: None,
                tools: None,
            },
            SessionEventInput::FunctionCallOutput {
                call_id: "c".into(),
                output: "o".into(),
            },
            SessionEventInput::Approval {
                elicitation_id: crate::ids::ElicitationId::new("e"),
                action: ElicitationAction::Accept,
                content: None,
            },
            SessionEventInput::Interrupt,
            SessionEventInput::Compact,
            SessionEventInput::StopSession,
        ] {
            assert!(
                ALLOWED_EVENT_TYPES.contains(&evt.discriminator()),
                "{} not in ALLOWED_EVENT_TYPES",
                evt.discriminator()
            );
        }
    }

    #[test]
    fn ack_parses_queued_with_item_id() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "item_id": "item_42"}"#).unwrap();
        assert!(ack.queued);
        assert_eq!(ack.item_id.as_deref(), Some("item_42"));
        assert!(!ack.denied);
    }

    #[test]
    fn ack_parses_control_event_not_queued() {
        let ack: SendEventAck = serde_json::from_str(r#"{"queued": false}"#).unwrap();
        assert!(!ack.queued);
        assert_eq!(ack.item_id, None);
    }

    #[test]
    fn ack_parses_policy_denial() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": false, "denied": true, "reason": "blocked"}"#)
                .unwrap();
        assert!(ack.denied);
        assert_eq!(ack.reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn session_status_deserializes_rest_values() {
        use serde_json::json;
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("idle")).unwrap(),
            SessionStatus::Idle
        );
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("running")).unwrap(),
            SessionStatus::Running
        );
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("failed")).unwrap(),
            SessionStatus::Failed
        );
        // "waiting" is collapsed to "running" server-side and never reaches REST; reject it.
        assert!(serde_json::from_value::<SessionStatus>(json!("waiting")).is_err());
    }

    #[test]
    fn get_opts_builds_expected_query() {
        let q = GetOpts::default().to_query();
        assert!(q.contains(&("include_liveness", "true".to_string())));
        assert!(q.contains(&("include_items", "false".to_string())));
        assert!(q.contains(&("refresh_state", "false".to_string())));
    }

    #[test]
    fn session_list_parses_paginated_envelope() {
        let body = r#"{"object":"list","data":[
            {"id":"s1","status":"idle","agent_id":"ag","agent_name":null,"archived":false,
             "created_at":1,"updated_at":2,"labels":{}}],
            "first_id":"s1","last_id":"s1","has_more":false}"#;
        let list: SessionList = serde_json::from_str(body).unwrap();
        assert_eq!(list.data.len(), 1);
        assert_eq!(list.data[0].id().as_str(), "s1");
        assert_eq!(list.data[0].updated_at(), 2);
        assert!(!list.has_more);
    }

    #[test]
    fn session_filter_builds_query() {
        let f = SessionFilter::new()
            .kind(SessionKind::SubAgent)
            .include_archived(true)
            .search("foo")
            .limit(50);
        let q = f.to_query();
        assert!(q.contains(&("kind", "sub_agent".to_string())));
        assert!(q.contains(&("include_archived", "true".to_string())));
        assert!(q.contains(&("search_query", "foo".to_string())));
        assert!(q.contains(&("limit", "50".to_string())));
    }

    #[test]
    fn snapshot_tolerates_null_collections() {
        // The live server sends `null` (not [] / {}) for empty collections.
        let body = r#"{
            "id":"conv_1","status":"idle","agent_id":"ag_1","created_at":1,
            "labels":null,"usage_by_model":null,"skills":null,"items":null
        }"#;
        let snap: SessionSnapshot = serde_json::from_str(body).unwrap();
        assert!(snap.labels().is_empty());
        assert!(snap.usage_by_model().is_empty());
        assert_eq!(snap.id().as_str(), "conv_1");
    }

    #[test]
    fn session_snapshot_parses_core_fields_and_liveness() {
        let body = r#"{
            "id": "sess_1", "status": "running", "agent_id": "ag_1",
            "agent_name": "Builder", "archived": false, "created_at": 1719331200,
            "labels": {"env": "prod"}, "runner_online": true, "host_online": null,
            "host_resumable": false, "extra_unmodeled_field": 99
        }"#;
        let s: SessionSnapshot = serde_json::from_str(body).unwrap();
        assert_eq!(s.id().as_str(), "sess_1");
        assert_eq!(s.status(), SessionStatus::Running);
        assert_eq!(s.agent_id(), "ag_1");
        assert_eq!(s.agent_name(), Some("Builder"));
        assert!(!s.archived());
        assert_eq!(s.created_at(), 1719331200);
        assert_eq!(s.labels().get("env").map(String::as_str), Some("prod"));
        assert_eq!(s.runner_online(), Some(true));
        assert_eq!(s.host_online(), None);
        assert!(!s.host_resumable());
    }

    #[test]
    fn child_session_summary_parses_full_and_partial() {
        // Full (GET) shape.
        let full = r#"{"id":"c1","object":"child_session","parent_session_id":"p1",
            "title":"T","tool":"task","session_name":"sn","kind":"sub_agent",
            "created_at":1,"updated_at":2,"busy":true,"labels":{},"current_task_status":"running",
            "pending_elicitations_count":3}"#;
        let c: ChildSessionSummary = serde_json::from_str(full).unwrap();
        assert_eq!(c.id().as_str(), "c1");
        assert_eq!(c.object(), Some("child_session"));
        assert_eq!(c.kind(), Some("sub_agent"));
        assert_eq!(c.parent_session_id(), "p1");
        assert!(c.busy());
        assert_eq!(c.pending_elicitations_count(), 3);
        assert_eq!(c.current_task_status(), Some("running"));

        // Partial (event delta) shape — most fields absent; required-on-full
        // fields that events omit must default, not error.
        let partial = r#"{"id":"c1","busy":false,"current_task_status":"launching"}"#;
        let p: ChildSessionSummary = serde_json::from_str(partial).unwrap();
        assert_eq!(p.id().as_str(), "c1");
        assert_eq!(p.object(), None);
        assert_eq!(p.kind(), None);
        assert_eq!(p.parent_session_id(), "");
        assert_eq!(p.created_at(), 0);
    }

    #[test]
    fn created_session_response_parses() {
        let r: CreatedSessionResponse =
            serde_json::from_str(r#"{"session_id":"s1","agent_id":"ag","agent_name":"A"}"#)
                .unwrap();
        assert_eq!(r.session_id().as_str(), "s1");
        assert_eq!(r.agent_name(), "A");
    }

    #[test]
    fn conversation_deleted_parses() {
        let d: ConversationDeleted =
            serde_json::from_str(r#"{"id":"s1","object":"conversation.deleted","deleted":true}"#)
                .unwrap();
        assert_eq!(d.id().as_str(), "s1");
        assert!(d.deleted());
    }

    #[test]
    fn create_request_serializes_minimal_and_full() {
        use serde_json::json;
        let min = CreateSessionRequest::new("ag_1");
        assert_eq!(
            serde_json::to_value(&min).unwrap(),
            json!({"agent_id": "ag_1"})
        );

        let full = CreateSessionRequest::new("ag_1")
            .host_type(HostType::Managed)
            .host_id("host_9")
            .git("feature/x", Some("main"));
        let v = serde_json::to_value(&full).unwrap();
        assert_eq!(v["agent_id"], json!("ag_1"));
        assert_eq!(v["host_type"], json!("managed"));
        assert_eq!(v["host_id"], json!("host_9"));
        assert_eq!(
            v["git"],
            json!({"branch_name": "feature/x", "base_branch": "main"})
        );
    }

    #[test]
    fn ack_tolerates_unknown_and_missing_fields() {
        // Empty body (openapi's under-specified `{}`) must still deserialize.
        let ack: SendEventAck = serde_json::from_str("{}").unwrap();
        assert!(!ack.queued);
        // Unknown extra fields are ignored, not an error.
        let ack2: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "future_field": 1}"#).unwrap();
        assert!(ack2.queued);
    }

    #[test]
    fn filesystem_list_parses() {
        let body = r#"{"object":"list","has_more":false,"data":[
            {"id":"e1","object":"session.environment.filesystem.entry","name":"main.rs",
             "path":"src/main.rs","type":"file","bytes":1024,"modified_at":1719331200}]}"#;
        let l: FilesystemList = serde_json::from_str(body).unwrap();
        assert_eq!(l.data[0].path(), "src/main.rs");
        assert_eq!(l.data[0].entry_type(), "file");
        assert_eq!(l.data[0].bytes(), Some(1024));
    }

    #[test]
    fn search_query_builds() {
        let q = SearchQuery::new("fn main").include("*.rs").limit(100);
        let pairs = q.to_query();
        assert!(pairs.contains(&("q", "fn main".to_string())));
        assert!(pairs.contains(&("include", "*.rs".to_string())));
        assert!(pairs.contains(&("limit", "100".to_string())));
    }

    #[test]
    fn file_diff_parses_before_after() {
        let d: FileDiff = serde_json::from_str(r#"{"before":"a\n","after":"b\n"}"#).unwrap();
        assert_eq!(d.before(), "a\n");
        assert_eq!(d.after(), "b\n");
    }

    #[test]
    fn resource_list_parses() {
        let list: ResourceList = serde_json::from_str(
            r#"{"data":[{"id":"r1","object":"environment"}],"has_more":false}"#,
        )
        .unwrap();
        assert_eq!(list.data[0].id(), "r1");
        assert!(!list.has_more);
    }

    #[test]
    fn item_list_parses_the_golden_items_envelope() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.items.json");
        let list: super::ItemList = serde_json::from_str(raw).expect("parse items envelope");
        assert_eq!(list.items().len(), 11);
        assert!(!list.has_more());
        // The corpus opens with a resource_event and contains the function_call pair.
        assert!(matches!(
            list.items()[0],
            crate::stream::Item::ResourceEvent { .. }
        ));
        assert!(
            list.items()
                .iter()
                .any(|i| matches!(i, crate::stream::Item::FunctionCall { .. }))
        );
        assert!(
            list.items()
                .iter()
                .any(|i| matches!(i, crate::stream::Item::FunctionCallOutput { .. }))
        );
        // Every item is reconcilable by a non-empty id.
        assert!(list.items().iter().all(|i| !i.id().is_empty()));
    }

    #[test]
    fn items_page_to_query_skips_none() {
        let q = super::ItemsPage {
            limit: Some(50),
            order: Some("asc".into()),
            ..Default::default()
        }
        .to_query();
        assert!(q.contains(&("limit", "50".to_string())));
        assert!(q.contains(&("order", "asc".to_string())));
        assert!(!q.iter().any(|(k, _)| *k == "after" || *k == "before"));
    }

    #[test]
    fn snapshot_parses_bucket_b_scalars_from_golden() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.snapshot.json");
        let s: super::SessionSnapshot = serde_json::from_str(raw).expect("parse snapshot");
        assert_eq!(s.harness(), "claude-sdk");
        assert_eq!(s.workspace(), Some("/Users/aakshintala/work/lens"));
        assert_eq!(s.permission_level(), Some(4));
        assert_eq!(
            s.root_conversation_id(),
            Some("conv_91d8bde71cae41e7b32e01a648e00f72")
        );
        // Byte fact: total_cost_usd present, llm_model/context_window null on this turn.
        assert!(s.total_cost_usd().unwrap() > 0.0);
        assert_eq!(s.llm_model(), None);
        assert_eq!(s.context_window(), None);
    }

    #[test]
    fn snapshot_parses_bucket_b_collections_from_golden() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.snapshot.json");
        let s: super::SessionSnapshot = serde_json::from_str(raw).expect("parse snapshot");
        // usage_by_model: one model with token+cost detail.
        let usage = s.usage_by_model();
        assert!(usage.contains_key("claude-opus-4-8"));
        assert!(usage["claude-opus-4-8"].total_tokens() > 0);
        // skills: 20 attached, each with a name.
        assert_eq!(s.skills().len(), 20);
        assert!(s.skills().iter().all(|sk| !sk.name().is_empty()));
        // embedded items: 11 (snapshot was captured with include_items).
        assert_eq!(s.items().len(), 11);
        assert!(!s.items()[0].id().is_empty());
        // Payload is hoisted out of the snapshot's `data` envelope, so embedded
        // items are fully typed (not just id-bearing shells). The corpus opens
        // with the resource_event; its typed fields come from `data`.
        assert!(matches!(
            &s.items()[0],
            crate::stream::Item::ResourceEvent { event_type, resource_type, .. }
                if event_type == "session.resource.created" && resource_type == "terminal"
        ));
        // An assistant message carries its role + non-empty content (both under `data`).
        assert!(s.items().iter().any(|i| matches!(
            i,
            crate::stream::Item::Message { role, content, .. }
                if role == "assistant" && !content.is_empty()
        )));
        // A function_call carries its name (under `data`).
        assert!(s.items().iter().any(|i| matches!(
            i,
            crate::stream::Item::FunctionCall { name, .. } if !name.is_empty()
        )));
        // (todos/pending_elicitations/model_options are empty in this capture and
        //  deferred — the snapshot still parses with them present-but-empty.)
    }
}
