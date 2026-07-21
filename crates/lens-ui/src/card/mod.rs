pub mod chrome;
pub mod model;
pub mod motion;
pub mod view;
pub mod wave;

pub use chrome::render_card_chrome;
pub use model::{
    CARD_HEIGHT_PX, CARD_WIDTH_PX, ConnectionOverlay, READY_DECAY_MS, RepoRef, SessionCard,
};
#[cfg(feature = "demo")]
pub use view::spawn_demo_paint_instrumentation;
pub use view::{SessionCardView, mount_cached_card};
pub use wave::{Wave, derive_wave};
