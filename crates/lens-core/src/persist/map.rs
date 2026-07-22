//! Column mapping: bare enum tokens (D-P2-8), json columns (D-P2-9), and the
//! `items.kind` vocabulary. Serialization of our OWN enums cannot fail on a
//! string-serializing type — the `expect` invariants below are never external data.

use crate::domain::ids::{AgentId, ConnectionId, HostId, ItemId, ResponseId, RunnerId, SessionId};
use crate::domain::item::{BlockContext, Item, ItemKind};
use crate::domain::scalars::{ErrorInfo, HostType, SessionLifecycle, SessionStatusValue};
use crate::domain::session::SessionState;
use crate::domain::usage::Cost;
use crate::persist::{Loaded, PersistError, Result, SkippedRow};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;

/// Drain `rows`, decoding each with `decode`. A per-row decode failure is recorded
/// in `Loaded::skipped` (keyed by `id_col`) and the row is skipped — so one corrupt
/// row never aborts the whole load (§6.3). A real cursor/IO error from `rows.next()`
/// still fails the load (outer `Err`).
pub fn collect_skipping<T>(
    rows: &mut rusqlite::Rows<'_>,
    id_col: usize,
    decode: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Loaded<T>> {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    while let Some(row) = rows.next()? {
        match decode(row) {
            Ok(v) => out.push(v),
            Err(e) => {
                let id = row
                    .get::<_, String>(id_col)
                    .unwrap_or_else(|_| "<unreadable-id>".to_string());
                skipped.push(SkippedRow {
                    id,
                    reason: e.to_string(),
                });
            }
        }
    }
    Ok(Loaded { rows: out, skipped })
}

/// A string-serializing enum → its bare token (`"waiting"`), for a Bridge column.
pub fn enum_token<T: Serialize>(v: &T) -> Result<String> {
    match serde_json::to_value(v)? {
        Value::String(s) => Ok(s),
        other => Err(PersistError::Json(
            <serde_json::Error as serde::ser::Error>::custom(format!(
                "expected a string-serializing enum, got {other}"
            )),
        )),
    }
}

/// A stored bare token → the enum (churn-safe via the enum's `#[serde(other)]`).
pub fn from_token<T: DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_value(Value::String(s))?)
}

/// Any serde type → a json `TEXT` column value.
pub fn json_string<T: Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// A json `TEXT` column value → the serde type.
pub fn from_json<T: DeserializeOwned>(s: &str) -> Result<T> {
    Ok(serde_json::from_str(s)?)
}

/// The stable `items.kind` vocabulary (§6.2 / D-P2-9). Matches `ItemKind`'s
/// snake_case serde tags exactly.
pub fn item_kind_token(k: &ItemKind) -> &'static str {
    match k {
        ItemKind::Message { .. } => "message",
        ItemKind::FunctionCall { .. } => "function_call",
        ItemKind::FunctionCallOutput { .. } => "function_call_output",
        ItemKind::Reasoning { .. } => "reasoning",
        ItemKind::NativeTool { .. } => "native_tool",
        ItemKind::Compaction { .. } => "compaction",
        ItemKind::SlashCommand { .. } => "slash_command",
        ItemKind::TerminalCommand { .. } => "terminal_command",
        ItemKind::Error { .. } => "error",
        ItemKind::ResourceEvent { .. } => "resource_event",
        ItemKind::AgentChanged { .. } => "agent_changed",
    }
}

/// The `sessions` SELECT column list — shared by `load_session` + `list_sessions`
/// so both feed one `row_to_session`. Order MUST match `row_to_session`'s `get(n)`.
pub const SESSION_COLUMNS: &str = "connection_id, id, agent_id, agent_name, runner_id, \
    parent_session_id, status, last_task_error, llm_model, model_override, reasoning_effort, \
    collaboration_mode, context_window, last_total_tokens, cost_json, workspace, git_branch, \
    host_type, host_id, title, labels, permission_level, owner, todos, skills, terminal_pending, \
    created_at, archived, lifecycle, last_focused_at";

/// Reconstruct a disk-snapshot `SessionState` (items empty; RAM-only fields
/// defaulted — D-P2-6). Total over decodable rows (never panics on disk data).
pub fn row_to_session(r: &rusqlite::Row) -> rusqlite::Result<SessionState> {
    // Lift a decode error out of a rusqlite row closure. NOTE (REVIEW#8): a
    // serde_json/enum decode failure is surfaced as `rusqlite::Error::
    // FromSqlConversionFailure`, which `?` then converts to `PersistError::Sqlite`
    // — NOT `PersistError::Json`. Totality is preserved (never a panic); callers
    // must not rely on the `Json` variant to distinguish a decode failure here.
    fn to_sql_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    }

    let connection_id = ConnectionId::new(r.get::<_, String>(0)?);
    let id = SessionId::new(r.get::<_, String>(1)?);
    let agent_id = AgentId::new(r.get::<_, String>(2)?);
    let mut st = SessionState::new(connection_id, id, agent_id);

    st.agent_name = r.get(3)?;
    st.runner_id = r.get::<_, Option<String>>(4)?.map(RunnerId::new);
    st.parent_session_id = r.get::<_, Option<String>>(5)?.map(SessionId::new);
    st.status = from_token::<SessionStatusValue>(r.get::<_, String>(6)?).map_err(to_sql_err)?;
    st.last_task_error = match r.get::<_, Option<String>>(7)? {
        Some(j) => Some(from_json::<ErrorInfo>(&j).map_err(to_sql_err)?),
        None => None,
    };
    st.llm_model = r.get(8)?;
    st.model_override = r.get(9)?;
    st.reasoning_effort = r.get(10)?;
    st.collaboration_mode = r.get(11)?;
    // REVIEW#6: read unsigned columns through i64 uniformly (like
    // permission_level) so a high-bit value loads (rusqlite's u64 FromSql errors
    // on > i64::MAX) — keeps loads total.
    st.context_window = r.get::<_, Option<i64>>(12)?.map(|v| v as u64);
    st.last_total_tokens = r.get::<_, Option<i64>>(13)?.map(|v| v as u64);
    st.cumulative_cost = match r.get::<_, Option<String>>(14)? {
        Some(j) => from_json::<Cost>(&j).map_err(to_sql_err)?,
        None => Cost::default(),
    };
    st.workspace = r.get(15)?;
    st.git_branch = r.get(16)?;
    st.host_type = from_token::<HostType>(r.get::<_, String>(17)?).map_err(to_sql_err)?;
    st.host_id = r.get::<_, Option<String>>(18)?.map(HostId::new);
    st.title = r.get(19)?;
    st.labels = match r.get::<_, Option<String>>(20)? {
        Some(j) => from_json::<BTreeMap<String, String>>(&j).map_err(to_sql_err)?,
        None => BTreeMap::new(),
    };
    st.permission_level = r.get::<_, Option<i64>>(21)?.map(|v| v as u8);
    st.owner = r.get(22)?;
    st.todos = match r.get::<_, Option<String>>(23)? {
        Some(j) => from_json(&j).map_err(to_sql_err)?,
        None => Vec::new(),
    };
    st.skills = match r.get::<_, Option<String>>(24)? {
        Some(j) => from_json(&j).map_err(to_sql_err)?,
        None => Vec::new(),
    };
    st.terminal_pending = r.get::<_, i64>(25)? != 0;
    st.created_at = r.get(26)?;
    st.archived = r.get::<_, i64>(27)? != 0;
    st.lifecycle = from_token::<SessionLifecycle>(r.get::<_, String>(28)?).map_err(to_sql_err)?;
    st.last_focused_at = r.get(29)?;
    // items, presence, stream, pending_user, model_options, sandbox_status,
    // pending_elicitations: NOT persisted (D-P2-5/D-P2-6) — left at `new()` defaults.
    Ok(st)
}

/// Reconstruct an `Item` from a transcript row. `payload` alone carries the full
/// tagged `ItemKind` (D-P2-9); `ordinal`/`kind` columns are read-contract only.
/// Total over decodable rows. Column order: item_id, live_seq, kind, payload,
/// agent, depth, created_at, response_id.
pub fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    row_to_item_at_offset(r, 0)
}

/// Same as `row_to_item`, but column indices are shifted by `offset` (reader SELECT
/// prepends `ordinal` at column 0).
pub(crate) fn row_to_item_at_offset(r: &rusqlite::Row, offset: usize) -> rusqlite::Result<Item> {
    fn to_sql_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    }
    let payload: String = r.get(offset + 3)?;
    let kind: ItemKind = from_json(&payload).map_err(to_sql_err)?;
    let response_id: Option<String> = r.get(offset + 7)?;
    Ok(Item {
        id: ItemId::new(r.get::<_, String>(offset)?),
        seq: r.get::<_, Option<i64>>(offset + 1)?.map(|v| v as u64),
        ctx: BlockContext {
            agent: r.get(offset + 4)?,
            depth: r.get::<_, i64>(offset + 5)? as u32,
            response_id: response_id.map(ResponseId::new),
        },
        created_at: r.get(offset + 6)?,
        kind,
    })
}

/// Decode a reader row: `ordinal` at column 0, then `row_to_item` columns at `1..`.
pub(crate) fn row_to_ordinal_item(r: &rusqlite::Row) -> rusqlite::Result<(i64, Item)> {
    let ordinal = r.get(0)?;
    let item = row_to_item_at_offset(r, 1)?;
    Ok((ordinal, item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::item::ItemKind;
    use crate::domain::scalars::SessionStatusValue;

    #[test]
    fn enum_token_is_bare_string_and_roundtrips_with_churn_safety() {
        let t = enum_token(&SessionStatusValue::Waiting).unwrap();
        assert_eq!(t, "waiting"); // NOT "\"waiting\""
        let back: SessionStatusValue = from_token(t).unwrap();
        assert_eq!(back, SessionStatusValue::Waiting);
        // Unknown stored token degrades, never errors (D-P2-8).
        let back: SessionStatusValue = from_token("superseded".to_string()).unwrap();
        assert_eq!(back, SessionStatusValue::Unknown);
    }

    #[test]
    fn item_kind_token_matches_schema_vocabulary() {
        assert_eq!(
            item_kind_token(&ItemKind::TerminalCommand {
                command: "ls".into()
            }),
            "terminal_command"
        );
        assert_eq!(
            item_kind_token(&ItemKind::Reasoning {
                full_text: String::new(),
                summary_text: String::new(),
                encrypted: false,
            }),
            "reasoning"
        );
    }
}
