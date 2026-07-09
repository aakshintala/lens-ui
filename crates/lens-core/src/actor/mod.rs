//! §8 ActiveSession actor — one OS thread per `(connection, session)` that
//! Selects over the event stream + command channel, reduces, persists, and emits.

mod runloop;

pub use runloop::{ActorHandle, ActorStores, OutputMode, SessionCommand, spawn_actor};
