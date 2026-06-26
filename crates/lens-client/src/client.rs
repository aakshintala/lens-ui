use crate::PINNED_OMNIGENT_VERSION;
use crate::connection::Connection;
use crate::error::{ClientError, Result};
use crate::http::{check_contract, check_status, decode_json};
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
        check_status("health", health.status().as_u16())?;

        // 2. contract gate
        let version_resp = conn
            .auth
            .apply(http.get(conn.url("/api/version")?))
            .send()?;
        let status = version_resp.status().as_u16();
        let body = version_resp.text()?;
        let ver: VersionResponse = decode_json("api/version", status, &body)?;
        check_contract(PINNED_OMNIGENT_VERSION, &ver.version)?;

        // 3. capabilities
        let info_resp = conn.auth.apply(http.get(conn.url("/v1/info")?)).send()?;
        let status = info_resp.status().as_u16();
        let body = info_resp.text()?;
        let info: ServerInfo = decode_json("v1/info", status, &body)?;

        Ok(Self { conn, http, info })
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// The session subservice (`POST /events`, and read methods in later plans).
    pub fn sessions(&self) -> crate::sessions::Sessions<'_> {
        crate::sessions::Sessions::new(self)
    }

    /// Internal accessors for later REST modules.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
    pub(crate) fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }

    /// Issue a GET expecting a JSON body, mapping status → typed errors. Internal
    /// building block for the typed REST methods. `query` pairs are appended as-is.
    pub(crate) fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> crate::error::Result<T> {
        let url = self.conn().url(path)?;
        let resp = self
            .conn()
            .auth
            .apply(self.http().get(url).query(query))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json(path, status, &body)
    }
}
