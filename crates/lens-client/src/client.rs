use std::time::Duration;

use crate::PINNED_OMNIGENT_VERSION;
use crate::connection::Connection;
use crate::error::{ClientError, Result};
use crate::http::{check_contract, check_status, decode_json};
use crate::info::{ServerInfo, VersionResponse};

/// Connect-phase timeout for ALL requests. Safe for SSE: it bounds only the
/// TCP/TLS handshake, never body reads — so a healthy quiet stream is untouched.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Total per-request timeout for SHORT REST calls only. Never applied to the
/// streaming GET (it would kill a healthy idle SSE body).
pub(crate) const REST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Client {
    conn: Connection,
    http: reqwest::blocking::Client,
    info: ServerInfo,
}

impl Client {
    /// Handshake + contract gate. Ready ladder: /health → /api/version → /v1/info.
    pub fn new(conn: Connection) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
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
            .apply(self.http().get(url).query(query).timeout(REST_TIMEOUT))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json(path, status, &body)
    }

    /// Send a request with an optional JSON body, mapping status → typed errors.
    /// `body: None::<&()>` for verbs without a body.
    pub(crate) fn send_json<T, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&B>,
    ) -> crate::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize,
    {
        let url = self.conn().url(path)?;
        let mut rb = self.http().request(method, url).query(query);
        rb = rb.timeout(REST_TIMEOUT);
        if let Some(b) = body {
            rb = rb.json(b);
        }
        let resp = self.conn().auth.apply(rb).send()?;
        let status = resp.status().as_u16();
        let text = resp.text()?;
        crate::http::decode_json(path, status, &text)
    }

    /// Send a JSON body and check status only, decoding no response body — for
    /// endpoints that return `204 No Content` (an empty body would fail
    /// `decode_json`'s 2xx `from_str`).
    pub(crate) fn send_no_content<B: serde::Serialize>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &B,
    ) -> crate::error::Result<()> {
        let url = self.conn().url(path)?;
        let rb = self
            .http()
            .request(method, url)
            .timeout(REST_TIMEOUT)
            .json(body);
        let resp = self.conn().auth.apply(rb).send()?;
        crate::http::check_status(path, resp.status().as_u16())
    }

    pub(crate) fn get_bytes(&self, path: &str) -> crate::error::Result<Vec<u8>> {
        let url = self.conn().url(path)?;
        let resp = self
            .conn()
            .auth
            .apply(self.http().get(url).timeout(REST_TIMEOUT))
            .send()?;
        crate::http::check_status(path, resp.status().as_u16())?;
        Ok(resp.bytes()?.to_vec())
    }

    /// Send a multipart/form-data request (bundle uploads).
    pub(crate) fn send_multipart<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        form: reqwest::blocking::multipart::Form,
    ) -> crate::error::Result<T> {
        let url = self.conn().url(path)?;
        let rb = self
            .http()
            .request(method, url)
            .multipart(form)
            .timeout(REST_TIMEOUT);
        let resp = self.conn().auth.apply(rb).send()?;
        let status = resp.status().as_u16();
        let text = resp.text()?;
        crate::http::decode_json(path, status, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::{CONNECT_TIMEOUT, REST_TIMEOUT};
    use std::time::Duration;

    #[test]
    fn timeouts_are_bounded_and_connect_is_shorter() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REST_TIMEOUT, Duration::from_secs(30));
        assert!(CONNECT_TIMEOUT < REST_TIMEOUT);
    }
}
