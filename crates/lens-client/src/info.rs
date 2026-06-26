use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct VersionResponse {
    pub version: String,
}

/// `GET /v1/info` — unauthenticated capability/auth probe (no version field).
#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    #[serde(default)]
    pub accounts_enabled: bool,
    #[serde(default)]
    pub login_url: Option<String>,
    #[serde(default)]
    pub needs_setup: bool,
    #[serde(default)]
    pub databricks_features: serde_json::Value,
    #[serde(default)]
    pub managed_sandboxes_enabled: bool,
    #[serde(default)]
    pub sandbox_provider: Option<String>,
}
