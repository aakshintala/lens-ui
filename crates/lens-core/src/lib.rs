//! `lens-core` — the framework-neutral state-model engine for one
//! `(connection, session)`. P0 defines the domain types (§2); later phases add
//! the reducer (§4), persistence (§6), and the actor (§8).

pub mod actor;
pub mod clock;
pub mod domain;
pub mod pack;
pub mod persist;
pub mod reduce;

pub use clock::{Clock, ManualClock};
pub use domain::*;
pub use persist::{
    BoardStore, ControlStore, LiveKey, Loaded, PersistError, ReconcileOutcome, SkippedRow,
    SqliteBoardStore, SqliteControlStore, StoreMode, TranscriptStore,
};
pub use reduce::{
    StreamUpdate, Updates, ViewBlock, group_work_section, pair_tool_spans, project, project_all,
    project_filtered, reduce,
};
