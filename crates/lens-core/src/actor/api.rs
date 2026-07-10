//! Actor-thread HTTP surface and command outcomes (D16 send half).

use lens_client::error::ClientError;
use lens_client::ids::SessionId;
use lens_client::sessions::{SendEventAck, SessionEventInput};

/// Actor-thread HTTP surface (the lens-client Sessions subset the actor needs).
/// Send-not-Sync: owned by and moved to the actor OS thread (mirrors `Box<dyn Clock + Send>`).
pub trait SessionApi: Send {
    fn send_event(
        &self,
        id: &SessionId,
        evt: &SessionEventInput,
    ) -> Result<SendEventAck, ClientError>;
}

/// Command-branch outcomes surfaced to the foreground (Task 8 deepens Table B mapping).
#[derive(Clone, Debug)]
pub enum CommandOutcome {
    SendAccepted {
        lens_pending_id: String,
        ack: SendEventAck,
    },
    SendDenied {
        lens_pending_id: String,
        reason: Option<String>,
    },
    SendFailed {
        lens_pending_id: String,
        error: String,
    },
    /// Actor is parked (Unauthorized/SessionFailed/RetriesExhausted) — no bubble inserted.
    SendRejected { reason: String },
}
