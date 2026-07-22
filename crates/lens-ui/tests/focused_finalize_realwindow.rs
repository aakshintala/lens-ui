//! Invokes the real-window staged-finalize probe binary on the main thread.
//! `Application::new().run()` cannot run inside `cargo test` worker threads.

// Requires window-server access; hangs headlessly (on_next_frame never advances as a `cargo test`
// subprocess). Run manually in a GUI session: `cargo run -p lens-ui --bin focused_finalize_probe`.
// The staged-finalize LOGIC is covered by the in-memory lib tests (staged_finalize_swaps_on_disk_row,
// collapse_timing_*); this probe is the paint/identity/scroll proof, gated out of the headless suite.
#[test]
#[ignore = "real-window probe needs a window server; run via `cargo run -p lens-ui --bin focused_finalize_probe`"]
fn focused_finalize_real_window_staged_handoff() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_focused_finalize_probe"))
        .status()
        .expect("spawn focused_finalize_probe binary");
    assert!(
        status.success(),
        "focused_finalize_probe exited with {:?}",
        status.code()
    );
}
