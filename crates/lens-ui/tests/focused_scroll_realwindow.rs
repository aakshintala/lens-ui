//! Invokes the real-window scroll-contract probe binary on the main thread.

// Requires window-server access; hangs headlessly. Run manually:
// `cargo run -p lens-ui --bin focused_scroll_probe`
#[test]
#[ignore = "real-window probe needs a window server; run via `cargo run -p lens-ui --bin focused_scroll_probe`"]
fn focused_scroll_real_window_contracts() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_focused_scroll_probe"))
        .status()
        .expect("spawn focused_scroll_probe binary");
    assert!(
        status.success(),
        "focused_scroll_probe exited with {:?}",
        status.code()
    );
}
