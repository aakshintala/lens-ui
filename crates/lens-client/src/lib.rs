//! `lens-client` — the typed seam over omnigent's HTTP/SSE/WS contract.
//! See `docs/design/typed-client.md` (contract) and
//! `docs/design/typed-client-implementation.md` (build decisions).

pub mod client;
pub mod connection;
pub mod error;
pub mod generated;
pub mod http;
pub mod ids;
pub mod info;
pub mod sessions;

/// The omnigent contract version this crate is pinned to (ADR-0001).
pub const PINNED_OMNIGENT_VERSION: &str = "0.3.0.dev0";

pub use client::Client;
pub use connection::{Auth, Connection};
pub use error::{ClientError, Result};
pub use sessions::{
    ChildSessionList, ChildSessionSummary, ConversationDeleted, CreateSessionRequest,
    CreatedSessionResponse, ElicitationAction, ElicitationState, FilesystemEntry, FilesystemList,
    HostType, SearchQuery, SendEventAck, SessionEventInput, SessionFilter, SessionKind,
    SessionList, SessionSnapshot, SessionStatus, SessionSummary, Sessions,
};
