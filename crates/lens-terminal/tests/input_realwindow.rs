//! Real-window input harness (Slice 2a Task 5).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens IME /
//! `InputHandler` claims. This `harness = false` binary opens a real GPUI
//! window, paints a focused [`TerminalTab`], and dispatches real keystrokes via
//! [`Window::dispatch_keystroke`] / [`Window::dispatch_event`].
//!
//! Ground truth for no-double-emit: each risky key must produce exactly one
//! engine egress, verified with a post-emit quiet window (no fg sleep).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Application, Bounds, Context, KeyDownEvent, KeyUpEvent, Keystroke, Render, TitlebarOptions,
    Window, WindowBounds, WindowOptions, prelude::*, px, size,
};
use lens_terminal::render_test_api::ascii_frame;
use lens_terminal::{CursorPos, EgressFrame, EngineConfig, EngineHandle, TerminalTab};

const RECV_TIMEOUT: Duration = Duration::from_secs(2);
const DOUBLE_EMIT_GUARD: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    FocusTerminal,
    Dispatch(Step),
    Awaiting(Step),
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    ArrowUp,
    Tab,
    Enter,
    ShiftA,
    RepeatUp,
    KeyUpPress,
    KeyUpRelease,
    PrintableA,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::ArrowUp => "arrow up",
            Step::Tab => "tab",
            Step::Enter => "enter",
            Step::ShiftA => "shift-a",
            Step::RepeatUp => "repeat up",
            Step::KeyUpPress => "keyup press",
            Step::KeyUpRelease => "keyup release",
            Step::PrintableA => "printable a",
        }
    }

    fn expected(self) -> Option<&'static [u8]> {
        match self {
            Step::ArrowUp => Some(b"\x1b[A"),
            Step::Tab => Some(b"\t"),
            Step::Enter => Some(b"\r"),
            Step::ShiftA => Some(b"A"),
            Step::PrintableA => Some(b"a"),
            Step::RepeatUp | Step::KeyUpPress | Step::KeyUpRelease => None,
        }
    }

    fn allow_empty_egress(self) -> bool {
        matches!(self, Step::KeyUpRelease)
    }

    fn next(self) -> Option<Step> {
        match self {
            Step::ArrowUp => Some(Step::Tab),
            Step::Tab => Some(Step::Enter),
            Step::Enter => Some(Step::ShiftA),
            Step::ShiftA => Some(Step::RepeatUp),
            Step::RepeatUp => Some(Step::KeyUpPress),
            Step::KeyUpPress => Some(Step::KeyUpRelease),
            Step::KeyUpRelease => Some(Step::PrintableA),
            Step::PrintableA => None,
        }
    }
}

struct AwaitOutcome {
    step: Step,
}

type AwaitReceiver = async_channel::Receiver<Result<AwaitOutcome, String>>;

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    await_rx: Rc<RefCell<Option<AwaitReceiver>>>,
    egress: crossbeam_channel::Receiver<EgressFrame>,
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
        let egress = engine.attach_test_egress();
        let tab = TerminalTab::open_with_engine_for_test(Arc::clone(&engine), cx);
        let mut frame = ascii_frame(40, 10, ' ');
        frame.cursor = Some(CursorPos { col: 0, row: 0 });
        tab.update(cx, |tab, cx| {
            tab.set_frame_for_test(Arc::new(frame), cx);
        });
        Self {
            phase: Rc::new(RefCell::new(Phase::PrimePaint)),
            await_rx: Rc::new(RefCell::new(None)),
            egress,
            tab,
        }
    }

    fn drain_egress(&self) {
        while self.egress.try_recv().is_ok() {}
    }

    fn spawn_await(&self, cx: &mut Context<Self>, step: Step) {
        let egress = self.egress.clone();
        let (tx, rx) = async_channel::bounded(1);
        *self.await_rx.borrow_mut() = Some(rx);
        cx.spawn(async move |_weak, _cx| {
            let result = await_single_egress(&egress, step);
            let _ = tx.send(result).await;
        })
        .detach();
    }

    fn dispatch_step(&self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.drain_egress();
        match step {
            Step::ArrowUp => dispatch_keystroke(window, "up", cx),
            Step::Tab => dispatch_keystroke(window, "tab", cx),
            Step::Enter => dispatch_keystroke(window, "enter", cx),
            Step::ShiftA => {
                // Shift+letter is uppercase TEXT → real committed-text path (InputHandler),
                // not keydown. gpui's dispatch_keystroke text path needs a painted-frame-
                // registered InputHandler; invoke the real enqueue path directly instead
                // (keydown-suppression for shift is covered by the keydown_should_enqueue
                // unit test). Proves single-emit through the production text path.
                let tab = self.tab.clone();
                tab.update(cx, |tab, _cx| tab.debug_input_handler_text_for_test("A"));
            }
            Step::RepeatUp => {
                let ks = Keystroke::parse("up").expect("parse up");
                let tab = self.tab.clone();
                tab.update(cx, |tab, cx| {
                    tab.debug_handle_key_down_for_test(
                        &KeyDownEvent {
                            keystroke: ks,
                            is_held: true,
                        },
                        window,
                        cx,
                    );
                });
            }
            Step::KeyUpPress => dispatch_keystroke(window, "up", cx),
            Step::KeyUpRelease => {
                let ks = Keystroke::parse("up").expect("parse up");
                let tab = self.tab.clone();
                tab.update(cx, |tab, cx| {
                    tab.debug_handle_key_up_for_test(&KeyUpEvent { keystroke: ks }, window, cx);
                });
            }
            Step::PrintableA => {
                let tab = self.tab.clone();
                tab.update(cx, |tab, _cx| tab.debug_input_handler_text_for_test("a"));
            }
        }
    }
}

fn dispatch_keystroke(window: &mut Window, source: &str, cx: &mut Context<HarnessView>) {
    let ks = Keystroke::parse(source).unwrap_or_else(|e| fail(&format!("parse {source}: {e}")));
    if !window.dispatch_keystroke(ks, cx) {
        fail(&format!("dispatch_keystroke {source} returned false"));
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("input_realwindow FAIL: {msg}");
    std::process::exit(1);
}

fn await_single_egress(
    egress: &crossbeam_channel::Receiver<EgressFrame>,
    step: Step,
) -> Result<AwaitOutcome, String> {
    let deadline = Instant::now() + RECV_TIMEOUT;
    let bytes = loop {
        if Instant::now() >= deadline {
            if step.allow_empty_egress() {
                println!(
                    "input_realwindow: {} OK (no egress — empty release encoding)",
                    step.label()
                );
                return Ok(AwaitOutcome { step });
            }
            return Err(format!("timeout waiting for egress ({})", step.label()));
        }
        if let Ok(frame) = egress.try_recv() {
            break frame.bytes;
        }
        std::thread::sleep(Duration::from_millis(1));
    };

    if let Some(expected) = step.expected()
        && bytes != expected
    {
        return Err(format!(
            "{}: expected {expected:?}, got {bytes:?}",
            step.label()
        ));
    }

    match egress.recv_timeout(DOUBLE_EMIT_GUARD) {
        Ok(extra) => Err(format!("{}: double-emit {:?}", step.label(), extra.bytes)),
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            println!("input_realwindow: {} OK ({bytes:?})", step.label());
            Ok(AwaitOutcome { step })
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            println!("input_realwindow: {} OK ({bytes:?})", step.label());
            Ok(AwaitOutcome { step })
        }
    }
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let phase = *self.phase.borrow();

        match phase {
            Phase::PrimePaint => {
                *self.phase.borrow_mut() = Phase::FocusTerminal;
            }
            Phase::FocusTerminal => {
                self.tab.update(cx, |tab, cx| {
                    tab.focus_handle(cx).focus(window);
                });
                *self.phase.borrow_mut() = Phase::Dispatch(Step::ArrowUp);
            }
            Phase::Dispatch(step) => {
                self.dispatch_step(step, window, cx);
                self.spawn_await(cx, step);
                *self.phase.borrow_mut() = Phase::Awaiting(step);
            }
            Phase::Awaiting(step) => {
                if let Some(rx) = self.await_rx.borrow_mut().take() {
                    match rx.try_recv() {
                        Ok(Ok(outcome)) => {
                            if outcome.step != step {
                                fail("await outcome step mismatch");
                            }
                            if let Some(next) = step.next() {
                                *self.phase.borrow_mut() = Phase::Dispatch(next);
                            } else {
                                *self.phase.borrow_mut() = Phase::Done;
                            }
                        }
                        Ok(Err(msg)) => fail(&msg),
                        Err(async_channel::TryRecvError::Empty) => {
                            *self.await_rx.borrow_mut() = Some(rx);
                        }
                        Err(async_channel::TryRecvError::Closed) => {
                            fail("await channel closed unexpectedly");
                        }
                    }
                }
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
