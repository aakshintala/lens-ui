//! §8 ActiveSession actor — one OS thread per `(connection, session)` that
//! Selects over the event stream + command channel, reduces, persists, and emits.

mod api;
mod outcome;
mod runloop;
mod summary;
mod transport;

pub use api::{ClientSessionApi, CommandOutcome, SessionApi};
pub use outcome::ActorOutcome;
pub use runloop::{
    ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor, spawn_actor_dual,
};
pub use summary::SummaryUpdate;
pub use transport::{ActorTransport, ParkReason};
