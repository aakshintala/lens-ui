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

/// Bench/live-test-only wrappers over `pub(crate)` codec fns.
#[cfg(any(feature = "bench", feature = "live-tests"))]
#[doc(hidden)]
pub mod bench_api {
    use tokio_tungstenite::tungstenite::Message;

    pub use super::wire::{WsInbound, WsOutbound};

    pub fn encode_outbound(o: &WsOutbound) -> Message {
        super::wire::encode_outbound(o)
    }

    pub fn classify_inbound(msg: Message) -> Option<WsInbound> {
        super::wire::classify_inbound(msg)
    }
}

use crate::client::Client;
use crate::ids::SessionId;

impl Client {
    /// Typed terminal REST + attach subservice for one session.
    pub fn terminals(&self, session: SessionId) -> Terminals<'_> {
        Terminals::new(self, session)
    }
}
