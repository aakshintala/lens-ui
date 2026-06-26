//! `lens-client` — the typed seam over omnigent's HTTP/SSE/WS contract.
//! See `docs/design/typed-client.md` (contract) and
//! `docs/design/typed-client-implementation.md` (build decisions).

pub mod error;

/// The omnigent contract version this crate is pinned to (ADR-0001).
pub const PINNED_OMNIGENT_VERSION: &str = "0.3.0.dev0";

pub use error::{ClientError, Result};
