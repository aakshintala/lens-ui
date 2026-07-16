//! Typed omnigent terminal transport: REST CRUD + WS attach. No serde_json::Value
//! or raw WS types escape this module. Close causes are *classified*; lifecycle
//! *policy* is lens-terminal's (Slice 1d).
mod attach;
mod close;
mod rest;
mod wire;

pub use attach::{AttachHandle, AttachInspect, AttachOptions, Backoff, attach};
pub use close::CloseCause;
pub use rest::{TerminalCreate, TerminalMetadata, TerminalResource, Terminals};
pub use wire::{WsInbound, WsOutbound};

use crate::client::Client;
use crate::ids::SessionId;

impl Client {
    /// Typed terminal REST + attach subservice for one session.
    pub fn terminals(&self, session: SessionId) -> Terminals<'_> {
        Terminals::new(self, session)
    }
}
