//! Real-window input harness (Slice 2a Task 5).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens IME /
//! `InputHandler` claims. This `harness = false` binary opens a real GPUI
//! window, paints a focused [`TerminalTab`], and dispatches real keystrokes via
//! [`Window::dispatch_keystroke`].
//!
//! Asserts:
//! - ArrowUp → special-only keydown path → CSI `ESC [ A`
//! - printable `a` → `InputHandler` path → single `a` egress (no keydown double-emit)

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Application, Bounds, Context, Keystroke, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, prelude::*, px, size,
};
use lens_terminal::render_test_api::ascii_frame;
use lens_terminal::{CursorPos, EngineConfig, EngineHandle, TerminalTab};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    DispatchArrowUp,
    AwaitArrowUp,
    DispatchPrintable,
    AwaitPrintable,
    Done,
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    engine: Arc<EngineHandle>,
    tab: gpui::Entity<TerminalTab>,
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = EngineConfig {
            cols: 40,
            rows: 10,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let engine = Arc::new(EngineHandle::spawn(cfg));
        let tab = TerminalTab::open_with_engine_for_test(Arc::clone(&engine), cx);
        let mut frame = ascii_frame(40, 10, ' ');
        frame.cursor = Some(CursorPos { col: 0, row: 0 });
        tab.update(cx, |tab, cx| {
            tab.set_frame_for_test(Arc::new(frame), cx);
        });
        Self {
            phase: Rc::new(RefCell::new(Phase::PrimePaint)),
            engine,
            tab,
        }
    }

    fn drain_egress(&self) {
        while self.engine.egress_rx().try_recv().is_ok() {}
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("input_realwindow FAIL: {msg}");
    std::process::exit(1);
}

fn recv_egress(engine: &EngineHandle, deadline: Instant, label: &str) -> Vec<u8> {
    while Instant::now() < deadline {
        if let Ok(bytes) = engine.egress_rx().try_recv() {
            return bytes;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    fail(&format!("timeout waiting for egress ({label})"));
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let phase = *self.phase.borrow();
        match phase {
            Phase::PrimePaint => {
                *self.phase.borrow_mut() = Phase::DispatchArrowUp;
            }
            Phase::DispatchArrowUp => {
                self.drain_egress();
                let ks = Keystroke::parse("up").expect("parse up");
                if !window.dispatch_keystroke(ks, cx) {
                    fail("dispatch_keystroke up returned false");
                }
                *self.phase.borrow_mut() = Phase::AwaitArrowUp;
            }
            Phase::AwaitArrowUp => {
                let bytes = recv_egress(
                    self.engine.as_ref(),
                    Instant::now() + Duration::from_secs(2),
                    "arrow up",
                );
                if bytes != b"\x1b[A" {
                    fail(&format!("arrow up expected ESC[A, got {bytes:?}"));
                }
                println!("input_realwindow: ArrowUp OK ({bytes:?})");
                *self.phase.borrow_mut() = Phase::DispatchPrintable;
            }
            Phase::DispatchPrintable => {
                self.drain_egress();
                let ks = Keystroke::parse("a").expect("parse a");
                if !window.dispatch_keystroke(ks, cx) {
                    fail("dispatch_keystroke a returned false");
                }
                *self.phase.borrow_mut() = Phase::AwaitPrintable;
            }
            Phase::AwaitPrintable => {
                let bytes = recv_egress(
                    self.engine.as_ref(),
                    Instant::now() + Duration::from_secs(2),
                    "printable a",
                );
                if bytes != b"a" {
                    fail(&format!("printable expected b\"a\", got {bytes:?}"));
                }
                if self.engine.egress_rx().try_recv().is_ok() {
                    fail("printable double-emit: more than one egress byte");
                }
                println!("input_realwindow: printable-a OK (exactly once)");
                *self.phase.borrow_mut() = Phase::Done;
            }
            Phase::Done => {
                println!("input_realwindow: all phases OK");
                std::process::exit(0);
            }
        }
        self.tab.clone()
    }
}

fn main() {
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal input_realwindow".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(640.0), px(480.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_window, cx| cx.new(HarnessView::new),
        )
        .expect("open_window");
        cx.activate(true);
    });
}
