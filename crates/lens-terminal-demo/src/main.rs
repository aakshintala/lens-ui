//! Standalone GPUI terminal demo — `lens-terminal`'s first consumer.
//!
//! Slice 0 establishes the crate and proves the frozen surface is nameable from
//! a downstream binary. Slice 1d wires the real GPUI window: `open()` a tab
//! against a live omnigent 0.5.1 terminal and render its `Frame`.

fn main() {
    // Reference the frozen public type so the dependency is real, not decorative.
    println!(
        "lens-terminal-demo — GPUI demo lands in Slice 1d (host type: {}).",
        std::any::type_name::<lens_terminal::TerminalTab>()
    );
}
