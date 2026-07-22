use serde::{Deserialize, Serialize};

use crate::client::{Client, REST_TIMEOUT};
use crate::error::{ClientError, Result};
use crate::http::decode_json;
use crate::ids::{SessionId, TerminalId};

/// Typed terminal REST subservice for one session.
pub struct Terminals<'a> {
    client: &'a Client,
    session: SessionId,
}

impl<'a> Terminals<'a> {
    pub(crate) fn new(client: &'a Client, session: SessionId) -> Self {
        Self { client, session }
    }

    /// `GET /v1/sessions/{id}/resources/terminals` — terminal resources only.
    pub fn list(&self) -> Result<Vec<TerminalResource>> {
        let path = format!("/v1/sessions/{}/resources/terminals", self.session);
        let page: TerminalList = self.get_json(&path)?;
        Ok(page.data)
    }

    /// `GET /v1/sessions/{id}/resources/terminals/{tid}`.
    pub fn get(&self, tid: &TerminalId) -> Result<TerminalResource> {
        let path = format!("/v1/sessions/{}/resources/terminals/{tid}", self.session);
        self.get_json(&path)
    }

    /// `POST /v1/sessions/{id}/resources/terminals` — launch or return existing.
    pub fn create(&self, req: &TerminalCreate) -> Result<TerminalResource> {
        let path = format!("/v1/sessions/{}/resources/terminals", self.session);
        self.client
            .send_json(reqwest::Method::POST, &path, &[], Some(req))
    }

    /// `DELETE /v1/sessions/{id}/resources/terminals/{tid}`.
    pub fn delete(&self, tid: &TerminalId) -> Result<()> {
        let path = format!("/v1/sessions/{}/resources/terminals/{tid}", self.session);
        let url = self.client.conn().url(&path)?;
        let resp = self
            .client
            .conn()
            .auth
            .apply(self.client.http().delete(url).timeout(REST_TIMEOUT))
            .send()?;
        let status = resp.status().as_u16();
        match status {
            200..=299 => Ok(()),
            404 => Err(ClientError::NotFound {
                what: format!("terminal {tid}"),
            }),
            401 | 403 => Err(ClientError::Auth { status }),
            _ => {
                let body = resp.text().unwrap_or_default();
                let _: () = decode_json(&path, status, &body)?;
                Ok(())
            }
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.client.conn().url(path)?;
        let resp = self
            .client
            .conn()
            .auth
            .apply(self.client.http().get(url).timeout(REST_TIMEOUT))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        if status == 404 {
            return Err(ClientError::NotFound {
                what: path.to_string(),
            });
        }
        decode_json(path, status, &body)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalCreate {
    pub terminal: String,
    pub session_key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TerminalMetadata {
    #[serde(default)]
    pub terminal_name: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub running: Option<bool>,
    #[serde(default)]
    pub terminal_transport: Option<String>,
}

/// One terminal resource (`SessionResourceObject` with `type: "terminal"`).
#[derive(Clone, Debug, Deserialize)]
pub struct TerminalResource {
    pub id: TerminalId,
    pub session_id: SessionId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    pub metadata: TerminalMetadata,
}

#[derive(Clone, Debug, Deserialize)]
struct TerminalList {
    pub data: Vec<TerminalResource>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_resource_decodes_metadata() {
        let body = r#"{"id":"term_abc","session_id":"sess_1","name":"shell",
        "metadata":{"terminal_name":"shell","session_key":"main","running":true,
        "terminal_transport":"control"}}"#;
        let r: TerminalResource = serde_json::from_str(body).unwrap();
        assert_eq!(r.id.as_str(), "term_abc");
        assert_eq!(r.metadata.terminal_name.as_deref(), Some("shell"));
        assert_eq!(r.metadata.running, Some(true));
    }

    #[test]
    fn terminal_create_serializes_wire_shape() {
        let c = TerminalCreate {
            terminal: "shell".into(),
            session_key: "main".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"terminal":"shell","session_key":"main"})
        );
    }
}
