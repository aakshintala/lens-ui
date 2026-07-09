//! Column mapping: bare enum tokens (D-P2-8), json columns (D-P2-9), and the
//! `items.kind` vocabulary. Serialization of our OWN enums cannot fail on a
//! string-serializing type — the `expect` invariants below are never external data.

use crate::domain::item::ItemKind;
use crate::persist::{PersistError, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

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
