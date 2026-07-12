//! Actor-thread HTTP surface and command outcomes (D16 send half).

use lens_client::Client;
use lens_client::error::ClientError;
use lens_client::ids::SessionId;
use lens_client::sessions::{
    GetOpts, ItemList, ItemsPage, SendEventAck, SessionEventInput, SessionStatus,
};

/// Actor-thread HTTP surface (the lens-client Sessions subset the actor needs).
/// Send-not-Sync: owned by and moved to the actor OS thread (mirrors `Box<dyn Clock + Send>`).
pub trait SessionApi: Send {
    fn send_event(
        &self,
        id: &SessionId,
        evt: &SessionEventInput,
    ) -> Result<SendEventAck, ClientError>;

    /// D19: the actor is the SOLE `/items` fetcher (forward catch-up). Blocking GET.
    fn fetch_items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList, ClientError>;

    /// D26: re-read live server status on reconnect/attach. Blocking GET /session.
    fn fetch_status(&self, id: &SessionId) -> Result<SessionStatus, ClientError>;
}

/// Production adapter: thin delegation over `lens_client::Sessions`.
pub struct ClientSessionApi {
    client: Client,
}

impl ClientSessionApi {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl SessionApi for ClientSessionApi {
    fn send_event(
        &self,
        id: &SessionId,
        evt: &SessionEventInput,
    ) -> Result<SendEventAck, ClientError> {
        self.client.sessions().send_event(id, evt)
    }

    fn fetch_items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList, ClientError> {
        self.client.sessions().items(id, page)
    }

    fn fetch_status(&self, id: &SessionId) -> Result<SessionStatus, ClientError> {
        Ok(self.client.sessions().get(id, GetOpts::default())?.status())
    }
}

/// Command-branch outcomes surfaced to the foreground (Task 8 deepens Table B mapping).
///
/// A send outcome carries `content` **iff it removes the bubble**. Fail/Denied/Lost
/// restore-to-composer → carry content; Pending keeps the bubble (the bubble is the
/// home of the text) → no content.
#[derive(Clone, Debug)]
pub enum CommandOutcome {
    SendAccepted {
        lens_pending_id: String,
        ack: SendEventAck,
    },
    SendDenied {
        lens_pending_id: String,
        content: String,
        reason: Option<String>,
    },
    SendFailed {
        lens_pending_id: String,
        content: String,
        error: String,
    },
    /// Held/maybe-landed: bubble stays, soft pending — no content (text lives in the bubble).
    SendPending { lens_pending_id: String },
}
