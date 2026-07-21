//! App-global keyboard shortcuts.
//!
//! These are registered as **global** actions (`cx.on_action`), not element-level `.on_action`
//! handlers, so they dispatch regardless of what element has focus. An element handler only fires
//! when the focused element is in its subtree — so a command triggered from a surface that holds no
//! keyboard focus (the board) is silently dropped. (This is why `cmd-shift-t` reload no-op'd as an
//! element handler; see the `gpui-global-vs-element-actions` note.)

use crate::actions::{BackToBoard, ReloadTheme};
use crate::fleet::store::FleetStore;
use gpui::{App, Entity, KeyBinding};

/// Bind the app's global shortcuts and register their app-global handlers. Call once per `App` at
/// startup (each `Application::run` is a fresh `App`), after the `FleetStore` exists.
pub fn register(fleet: &Entity<FleetStore>, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-.", BackToBoard, None),
        KeyBinding::new("cmd-shift-t", ReloadTheme, None),
    ]);

    // cmd-. — blur any focused session back to the board. Global so it fires from the focused view
    // regardless of which element there holds focus.
    let fleet = fleet.downgrade();
    cx.on_action::<BackToBoard>(move |_, cx| {
        fleet.update(cx, |fleet, cx| fleet.blur_to_board(cx)).ok();
    });

    // cmd-shift-t — reload the theme from disk (app-global; installs its own global task holder).
    crate::theme::register_reload_action(cx);
}
