//! No-replay reconnect (typed-client.md §7). The reader thread owns the protocol
//! end-to-end; the consumer only sees synthetic lifecycle ServerStreamEvents.
//! This module supplies the re-issue capability (`Reopen`) + helpers.

use std::io::Read;

use crate::client::Client;
use crate::connection::Connection;
use crate::error::Result;
use crate::ids::SessionId;
use crate::sessions::{GetOpts, SessionSnapshot};

/// §7 backoff schedule (ms): 100→200→400→800→1600→3000→3000. ~7s through six.
pub(crate) const BACKOFF_MS: &[u64] = &[100, 200, 400, 800, 1600, 3000, 3000];

/// The reader's re-issue capability. `Send` so it can live on the reader thread;
/// a trait so the reconnect state machine is unit-testable with a scripted mock.
pub(crate) trait Reopen: Send {
    /// Open a fresh `GET /stream` body.
    fn open_stream(&self) -> Result<Box<dyn Read + Send>>;
    /// `GET /v1/sessions/{id}` with items+liveness (bucket B chrome).
    fn snapshot(&self) -> Result<SessionSnapshot>;
}

/// Real impl: clones the cheap, `Send + 'static` request machinery. No `info`.
pub(crate) struct HttpReopener {
    http: reqwest::blocking::Client,
    conn: Connection,
    session_id: SessionId,
}

impl HttpReopener {
    pub(crate) fn new(client: &Client, session_id: SessionId) -> Self {
        Self {
            http: client.http().clone(),
            conn: client.conn().clone(),
            session_id,
        }
    }
}

impl Reopen for HttpReopener {
    fn open_stream(&self) -> Result<Box<dyn Read + Send>> {
        // No total timeout: this is the long-lived SSE body. connect_timeout
        // (client-level) bounds the handshake; idle is handled by the stop flag.
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}/stream", self.session_id))?;
        let resp = self.conn.auth.apply(self.http.get(url)).send()?;
        crate::http::check_status("v1/sessions/stream", resp.status().as_u16())?;
        Ok(Box::new(resp))
    }

    fn snapshot(&self) -> Result<SessionSnapshot> {
        let opts = GetOpts {
            include_items: true,
            include_liveness: true,
            refresh_state: false,
        };
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}", self.session_id))?;
        let resp = self
            .conn
            .auth
            .apply(
                self.http
                    .get(url)
                    .query(&opts.to_query())
                    .timeout(crate::client::REST_TIMEOUT),
            )
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json("v1/sessions", status, &body)
    }
}
