//! Session control/chrome sub-types (§2.2). Domain-owned mirrors of wire
//! wrappers; rendering/actions belong to their surface documents.

use crate::domain::ids::{ElicitationId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Unknown,
}

/// The agent's live todos — rendered inline in chat (§2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
    pub active_form: String,
}

/// 0.2.0 chrome (§2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: Option<String>,
}

/// Drives the model picker (§2.2 `model_options`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

/// Managed-sandbox launch progress (§2.2). `detail` set when `stage == "failed"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxStatus {
    pub stage: String,
    pub detail: Option<String>,
}

/// Elicitation request parameters (mirrors `lens_client::stream::ElicitationParams`,
/// which is deserialize-only). Grown by the §4.3 elicitation surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElicitationParams {
    pub mode: String,
    pub message: String,
    pub url: Option<String>,
    pub phase: Option<String>,
    pub policy_name: Option<String>,
    pub content_preview: Option<String>,
}

/// A pending elicitation prompt (§2.2, PLURAL). Carries `target_session_id` for
/// resolve routing (fan-out parents mirror multiple child prompts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Elicitation {
    pub id: ElicitationId,
    pub target_session_id: SessionId,
    pub params: ElicitationParams,
}

/// Optimistic, pre-`consumed` user message (§7). RAM-only intent; carried on
/// `SessionState.pending_user`. `pending_id` is Lens-local until/unless the
/// server returns one (P3 live-verify item).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUserMessage {
    pub pending_id: String,
    pub content: String,
    /// Epoch millis, injected-clock-stamped when the send is issued (P3).
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_roundtrips_and_unknown_status_is_churn_safe() {
        let t = Todo {
            content: "wire the reducer".into(),
            status: TodoStatus::InProgress,
            active_form: "wiring the reducer".into(),
        };
        let back: Todo = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        let unk: TodoStatus = serde_json::from_str("\"blocked\"").unwrap();
        assert_eq!(unk, TodoStatus::Unknown);
    }

    #[test]
    fn elicitation_roundtrips() {
        let e = Elicitation {
            id: ElicitationId::new("elic_1"),
            target_session_id: SessionId::new("conv_1"),
            params: ElicitationParams {
                mode: "url".into(),
                message: "approve?".into(),
                url: Some("https://x".into()),
                phase: None,
                policy_name: None,
                content_preview: None,
            },
        };
        let back: Elicitation = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn pending_user_message_roundtrips() {
        let p = PendingUserMessage {
            pending_id: "pend_1".into(),
            content: "hello".into(),
            created_at: 1_700_000_000_000,
        };
        let back: PendingUserMessage =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn skill_model_sandbox_roundtrip() {
        let s = SkillSummary {
            name: "grep".into(),
            description: None,
        };
        assert_eq!(
            serde_json::from_str::<SkillSummary>(&serde_json::to_string(&s).unwrap()).unwrap(),
            s
        );
        let m = ModelOption {
            id: "opus".into(),
            label: "Opus 4.8".into(),
        };
        assert_eq!(
            serde_json::from_str::<ModelOption>(&serde_json::to_string(&m).unwrap()).unwrap(),
            m
        );
        let sb = SandboxStatus {
            stage: "provisioning".into(),
            detail: None,
        };
        assert_eq!(
            serde_json::from_str::<SandboxStatus>(&serde_json::to_string(&sb).unwrap()).unwrap(),
            sb
        );
    }
}
