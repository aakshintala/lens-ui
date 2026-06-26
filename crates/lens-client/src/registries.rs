//! Server-level registries (agents, hosts, runners, policies) + `/v1/me`.
//! Methods hang off `Client`. Read responses are typed wrappers (no `Value`
//! reaches consumers); request bodies use generated types where available.

use crate::client::Client;
use crate::error::Result;
use crate::ids::RunnerId;

/// `GET /v1/runners/{id}/status` (`runner_tunnel.py:234-265`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RunnerStatus {
    runner_id: RunnerId,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    error: Option<String>,
}
impl RunnerStatus {
    pub fn runner_id(&self) -> &RunnerId {
        &self.runner_id
    }
    pub fn online(&self) -> bool {
        self.online
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// `GET /v1/me` — auth identity (`app.py:1566-1592`). 200 carries `user_id`
/// (null when accounts are off); a 401 (OIDC unauthenticated) is surfaced as
/// `ClientError::Auth` by `decode_json`, so this type models the 200 body only.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Me {
    #[serde(default)]
    user_id: Option<String>,
}
impl Me {
    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }
}

/// `GET /v1/agents` — agent registry (read-only; authoring is filesystem YAML +
/// bundle upload). Elements are the generated `AgentObject`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AgentList {
    #[serde(default)]
    pub data: Vec<crate::generated::AgentObject>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

impl Client {
    pub fn runner_status(&self, runner_id: &RunnerId) -> Result<RunnerStatus> {
        self.get_json(&format!("/v1/runners/{runner_id}/status"), &[])
    }

    pub fn me(&self) -> Result<Me> {
        self.get_json("/v1/me", &[])
    }

    pub fn list_agents(&self) -> Result<AgentList> {
        self.get_json("/v1/agents", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_status_parses_online_and_offline() {
        let on: RunnerStatus = serde_json::from_str(r#"{"runner_id":"r1","online":true}"#).unwrap();
        assert_eq!(on.runner_id().as_str(), "r1");
        assert!(on.online());
        assert_eq!(on.error(), None);
        let off: RunnerStatus =
            serde_json::from_str(r#"{"runner_id":"r1","online":false,"error":"exited 1"}"#)
                .unwrap();
        assert!(!off.online());
        assert_eq!(off.error(), Some("exited 1"));
    }

    #[test]
    fn me_parses_user_id() {
        let m: Me = serde_json::from_str(r#"{"user_id":"u_42"}"#).unwrap();
        assert_eq!(m.user_id(), Some("u_42"));
        let anon: Me = serde_json::from_str(r#"{"user_id":null}"#).unwrap();
        assert_eq!(anon.user_id(), None);
    }

    #[test]
    fn agent_list_parses() {
        let body = r#"{"data":[{"id":"ag","name":"A","created_at":1}],"has_more":false}"#;
        let list: AgentList = serde_json::from_str(body).unwrap();
        assert_eq!(list.data[0].id, "ag");
        assert_eq!(list.data[0].name, "A");
        assert!(!list.has_more);
    }
}
