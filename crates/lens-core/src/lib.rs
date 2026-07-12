//! `lens-core` — the framework-neutral state-model engine for one
//! `(connection, session)`. P0 defines the domain types (§2); later phases add
//! the reducer (§4), persistence (§6), and the actor (§8).

pub mod actor;
pub mod clock;
pub mod domain;
pub mod persist;
pub mod reduce;

pub use clock::{Clock, ManualClock};
pub use domain::*;
pub use persist::{
    ControlStore, LiveKey, Loaded, PersistError, ReconcileOutcome, SkippedRow, StoreMode,
    TranscriptStore,
};
pub use reduce::{StreamUpdate, Updates, reduce};
