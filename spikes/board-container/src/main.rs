//! Board container / culling spike (B-2 §4). Real-window gpui program
//! (`Application::new().run()`, harness=false — a TestAppContext fakes the text
//! system and would false-green paint/layout; memory: gpui-test-noop-text-system).
//!
//! Modes:
//!   cargo run                 # interactive: scroll with trackpad, watch HUD + card ticks
//!   cargo run -- --probe      # automated GO/NO-GO assertions, prints table, quits
//!   cargo run -- --all-timers # cull+gate OFF (every card's timer runs) — measurement baseline
//!
//! Build RELEASE for the CPU measurement (`./measure.sh`): gate perf in release
//! (memory: terminal-slice-1c-executed).
#![allow(dead_code)]

mod card;
mod container;
mod packer;
mod probe;

use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};

use container::Container;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let probe = args.iter().any(|a| a == "--probe");
    let cull_enabled = !args.iter().any(|a| a == "--all-timers");

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| cx.new(|cx| Container::new(cull_enabled, probe, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
