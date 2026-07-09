//! `lens-core` — the framework-neutral state-model engine for one
//! `(connection, session)`. P0 defines the domain types (§2); later phases add
//! the reducer (§4), persistence (§6), and the actor (§8).

pub mod domain;

pub use domain::*;
