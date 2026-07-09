//! Scalar enums and small leaf structs (§2.2/§2.3/§2.5).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// Origin of a persisted `Error` item (§2.3 `ItemKind::Error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSource {
    Server,
    Client,
    Runner,
    /// Any source literal this version does not know.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostType {
    External,
    Managed,
}

/// Lens-local lifecycle (§2.2). Distinct from the server `archived` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionLifecycle {
    Active,
    Slept,
    Deleted,
}

/// Canonical fine-grained status (§2.2). The full 5-state set is only observable
/// from SSE (`SessionStatusEvent`); the REST poll is coarse 3-state and is
/// normalized into this by the reducer (P1). `Unknown` covers dev0 churn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatusValue {
    Idle,
    Launching,
    Running,
    Waiting,
    Failed,
    #[serde(other)]
    Unknown,
}

/// Present iff `SessionState.status == Failed` (§2.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_lowercase_and_roundtrips() {
        assert_eq!(
            serde_json::to_string(&SessionStatusValue::Waiting).unwrap(),
            "\"waiting\""
        );
        let back: SessionStatusValue = serde_json::from_str("\"launching\"").unwrap();
        assert_eq!(back, SessionStatusValue::Launching);
    }

    #[test]
    fn status_unknown_literal_maps_to_unknown_variant() {
        let back: SessionStatusValue = serde_json::from_str("\"superseded\"").unwrap();
        assert_eq!(back, SessionStatusValue::Unknown);
    }

    #[test]
    fn error_source_unknown_is_churn_safe() {
        let back: ErrorSource = serde_json::from_str("\"gateway\"").unwrap();
        assert_eq!(back, ErrorSource::Unknown);
    }

    #[test]
    fn error_info_roundtrips() {
        let e = ErrorInfo {
            code: "rate_limited".into(),
            message: "slow down".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: ErrorInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn role_and_hosttype_and_lifecycle_roundtrip() {
        for r in [Role::User, Role::Assistant] {
            let back: Role = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(back, r);
        }
        let back: HostType =
            serde_json::from_str(&serde_json::to_string(&HostType::Managed).unwrap()).unwrap();
        assert_eq!(back, HostType::Managed);
        let back: SessionLifecycle =
            serde_json::from_str(&serde_json::to_string(&SessionLifecycle::Slept).unwrap())
                .unwrap();
        assert_eq!(back, SessionLifecycle::Slept);
    }
}
