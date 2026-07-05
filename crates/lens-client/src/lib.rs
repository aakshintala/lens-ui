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
mod reconnect;
pub mod registries;
pub mod sessions;
pub mod stream;

/// The omnigent contract version this crate is pinned to (ADR-0001).
pub const PINNED_OMNIGENT_VERSION: &str = "0.4.0";

pub use client::Client;
pub use connection::{Auth, Connection};
pub use error::{ClientError, Result};
pub use registries::{
    AgentList, DirectoryObject, HostList, HostObject, Me, PolicyEvaluation, PolicyList,
    PolicyObject, PolicyRegistry, RunnerLaunchResult, RunnerStatus,
};
pub use sessions::{
    ChildSessionList, ChildSessionSummary, CommentObject, ConversationDeleted,
    CreateSessionRequest, CreatedSessionResponse, ElicitationAction, ElicitationResult,
    ElicitationState, FileContent, FileDiff, FileResource, FilesList, FilesystemEntry,
    FilesystemList, HostType, OwnerInfo, PermissionsInfo, ResourceList, ResourceObject,
    SearchQuery, SendEventAck, SessionEventInput, SessionFilter, SessionKind, SessionList,
    SessionSnapshot, SessionStatus, SessionSummary, Sessions, ShellResult,
};
