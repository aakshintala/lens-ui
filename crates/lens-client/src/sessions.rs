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

use crate::ids::{ElicitationId, SessionId};

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

/// A session snapshot (`GET /v1/sessions/{id}`). Mirrors the CORE fields of
/// omnigent's `SessionResponse` (`schemas.py:1601-1642`); unmodeled fields are
/// ignored. Fields are private — access is via the typed getters, so the wire
/// shape stays an lens-client implementation detail (single edit site for drift).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionSnapshot {
    id: SessionId,
    status: SessionStatus,
    agent_id: String,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    archived: bool,
    created_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    runner_online: Option<bool>,
    #[serde(default)]
    host_online: Option<bool>,
    #[serde(default)]
    host_resumable: bool,
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
    fn to_query(self) -> Vec<(&'static str, String)> {
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
    fn ack_tolerates_unknown_and_missing_fields() {
        // Empty body (openapi's under-specified `{}`) must still deserialize.
        let ack: SendEventAck = serde_json::from_str("{}").unwrap();
        assert!(!ack.queued);
        // Unknown extra fields are ignored, not an error.
        let ack2: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "future_field": 1}"#).unwrap();
        assert!(ack2.queued);
    }
}
