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
    pub use super::wire::{WsInbound, WsOutbound};

    /// Encoded WS frame payload — no tungstenite types on the public surface.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum EncodedFrame {
        Binary(Vec<u8>),
        Text(String),
    }

    /// Raw inbound wire bytes for bench classify paths.
    #[derive(Clone, Debug)]
    pub enum WireInbound {
        Binary(Vec<u8>),
        Text(String),
        Close(u16),
        Ping(Vec<u8>),
    }

    pub fn encode_outbound(o: &WsOutbound) -> EncodedFrame {
        match super::wire::encode_outbound(o) {
            tokio_tungstenite::tungstenite::Message::Binary(b) => EncodedFrame::Binary(b.into()),
            tokio_tungstenite::tungstenite::Message::Text(t) => EncodedFrame::Text(t.to_string()),
            _ => EncodedFrame::Binary(Vec::new()),
        }
    }

    pub fn classify_inbound(frame: WireInbound) -> Option<WsInbound> {
        use tokio_tungstenite::tungstenite::Message;
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;

        let msg = match frame {
            WireInbound::Binary(bytes) => Message::Binary(bytes.into()),
            WireInbound::Text(text) => Message::Text(text.into()),
            WireInbound::Close(code) => Message::Close(Some(CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                    code,
                ),
                reason: Default::default(),
            })),
            WireInbound::Ping(bytes) => Message::Ping(bytes.into()),
        };
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
