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

impl Client {
    pub fn runner_status(&self, runner_id: &RunnerId) -> Result<RunnerStatus> {
        self.get_json(&format!("/v1/runners/{runner_id}/status"), &[])
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
}
