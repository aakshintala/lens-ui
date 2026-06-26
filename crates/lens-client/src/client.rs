use crate::PINNED_OMNIGENT_VERSION;
use crate::connection::Connection;
use crate::error::{ClientError, Result};
use crate::http::check_contract;
use crate::info::{ServerInfo, VersionResponse};

pub struct Client {
    conn: Connection,
    http: reqwest::blocking::Client,
    info: ServerInfo,
}

impl Client {
    /// Handshake + contract gate. Ready ladder: /health → /api/version → /v1/info.
    pub fn new(conn: Connection) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(ClientError::Network)?;

        // 1. liveness
        let health = conn.auth.apply(http.get(conn.url("/health")?)).send()?;
        if !health.status().is_success() {
            return Err(ClientError::Server {
                status: health.status().as_u16(),
                body: serde_json::json!({ "stage": "health" }),
            });
        }

        // 2. contract gate
        let ver: VersionResponse = conn
            .auth
            .apply(http.get(conn.url("/api/version")?))
            .send()?
            .json()?;
        check_contract(PINNED_OMNIGENT_VERSION, &ver.version)?;

        // 3. capabilities
        let info: ServerInfo = conn
            .auth
            .apply(http.get(conn.url("/v1/info")?))
            .send()?
            .json()?;

        Ok(Self { conn, http, info })
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// Internal accessors for later REST modules.
    // Consumed by Plan 2 (REST surface); allow until then.
    #[allow(dead_code)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
    // Consumed by Plan 2 (REST surface); allow until then.
    #[allow(dead_code)]
    pub(crate) fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }
}
