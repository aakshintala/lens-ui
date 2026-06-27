//! No-replay reconnect (typed-client.md §7). The reader thread owns the protocol
//! end-to-end; the consumer only sees synthetic lifecycle ServerStreamEvents.
//! This module supplies the re-issue capability (`Reopen`) + helpers.

use std::io::Read;

use crate::client::Client;
use crate::connection::Connection;
use crate::error::Result;
use crate::ids::SessionId;
use crate::sessions::{GetOpts, ItemList, ItemsPage, SessionSnapshot};
use crate::stream::ServerStreamEvent;
use crate::stream::event::ResponseEvent;

/// §7 backoff schedule (ms): 100→200→400→800→1600→3000→3000. ~7s through six.
#[allow(dead_code)] // wired in Plan 3b-2b Task 5/6
pub(crate) const BACKOFF_MS: &[u64] = &[100, 200, 400, 800, 1600, 3000, 3000];

/// The reader's re-issue capability. `Send` so it can live on the reader thread;
/// a trait so the reconnect state machine is unit-testable with a scripted mock.
#[allow(dead_code)] // wired in Plan 3b-2b Task 5/6
pub(crate) trait Reopen: Send {
    /// Open a fresh `GET /stream` body.
    fn open_stream(&self) -> Result<Box<dyn Read + Send>>;
    /// `GET /v1/sessions/{id}` with items+liveness (bucket B chrome).
    fn snapshot(&self) -> Result<SessionSnapshot>;
    /// `GET /v1/sessions/{id}/items` (bucket A history).
    fn items(&self) -> Result<ItemList>;
}

/// Real impl: clones the cheap, `Send + 'static` request machinery. No `info`.
#[allow(dead_code)] // wired in Plan 3b-2b Task 5/6
pub(crate) struct HttpReopener {
    http: reqwest::blocking::Client,
    conn: Connection,
    session_id: SessionId,
}

#[allow(dead_code)] // wired in Plan 3b-2b Task 5/6
impl HttpReopener {
    pub(crate) fn new(client: &Client, session_id: SessionId) -> Self {
        Self {
            http: client.http().clone(),
            conn: client.conn().clone(),
            session_id,
        }
    }
}

#[allow(dead_code)] // wired in Plan 3b-2b Task 5/6
impl Reopen for HttpReopener {
    fn open_stream(&self) -> Result<Box<dyn Read + Send>> {
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
            .apply(self.http.get(url).query(&opts.to_query()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json("v1/sessions", status, &body)
    }

    fn items(&self) -> Result<ItemList> {
        let page = ItemsPage::default();
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}/items", self.session_id))?;
        let resp = self
            .conn
            .auth
            .apply(self.http.get(url).query(&page.to_query()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json("v1/sessions/items", status, &body)
    }
}

/// Bucket A: replay the durable transcript as `OutputItemDone` events. The
/// consumer merges by `Item::id()` (idempotent upsert), so duplicates are safe.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn items_to_replay(list: ItemList) -> Vec<ServerStreamEvent> {
    list.into_items()
        .into_iter()
        .map(|item| ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::ItemList;
    use crate::stream::ServerStreamEvent;
    use crate::stream::event::ResponseEvent;

    #[test]
    fn items_to_replay_maps_each_item_to_output_item_done() {
        // Build an ItemList from the golden /items capture so payloads are real.
        let raw =
            include_str!("../../../docs/spikes/captures/2026-06-26-sse/happy_path.items.json");
        let list: ItemList = serde_json::from_str(raw).expect("parse items capture");
        let n = list.items().len();
        assert!(n > 0, "fixture must have items");
        let out = items_to_replay(list);
        assert_eq!(out.len(), n);
        assert!(out.iter().all(|e| matches!(
            e,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
        )));
    }
}
