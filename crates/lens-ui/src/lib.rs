pub mod actions;
pub mod assets;
pub mod board;
pub mod card;
pub mod clock;
pub mod fleet;
pub mod focused;
pub mod md;
pub mod security;
pub mod shortcuts;
pub mod slot;
pub mod theme;

pub use board::{BoardView, ShellMode};
pub use clock::{ManualUiClock, UiClock, WallUiClock};
pub use fleet::fake::{FEED_CAPACITY, FakeFleet, FakeSessionHandles};
pub use fleet::poller::spawn_session_poller;
pub use fleet::store::FleetStore;
pub use slot::{ContentTab, PlaceholderTab, TabHandle, focused_transcript_tab, placeholder_tab};
pub use theme::{ActiveLensTheme, LensTheme};

use std::cell::Cell;
use std::rc::Rc;

/// Stub PTY byte counter — Task 5 asserts BackToBoard does not increment it.
#[derive(Clone)]
pub struct PtyProbe {
    pub bytes_sent: Rc<Cell<usize>>,
}
