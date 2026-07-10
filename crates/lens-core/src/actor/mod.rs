//! §8 ActiveSession actor — one OS thread per `(connection, session)` that
//! Selects over the event stream + command channel, reduces, persists, and emits.

mod api;
mod runloop;
mod summary;

pub use api::{CommandOutcome, SessionApi};
pub use runloop::{
    ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor, spawn_actor_dual,
};
pub use summary::SummaryUpdate;
