//! §2 domain model — pure data + serde, no logic.

pub mod board;
pub mod controls;
pub mod ids;
pub mod item;
pub mod scalars;
pub mod session;
pub mod usage;

pub use board::*;
pub use controls::*;
pub use ids::*;
pub use item::*;
pub use scalars::*;
pub use session::*;
pub use usage::*;
