//! Real-window presentation harness (Slice 2d Task 4).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens hit-testing.
//! This `harness = false` binary opens a real GPUI window and drives the FULL
//! production hyperlink path: it feeds an OSC-8 sequence to a real engine so the
//! engine-built frame carries `hyperlink_uri` on one specific cell (the same
//! frame `on_mouse_down` hit-tests via the render sampler — injecting a frame
//! with `set_frame_for_test` does not work here because the render loop's
//! `sample_latest_frame_from_engine` overwrites it every paint). It then clicks
//! a plain cell (negative) and the link cell (positive), asserting
//! `TerminalEvent::OpenUrlRequest` fires only for the correct cell (click only —
//! hover deferred).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Application, Bounds, Context, Pixels, Point, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, point, prelude::*, px, size,
};
use lens_terminal::render_test_api::CellMetrics;
use lens_terminal::{EngineConfig, EngineHandle, HostRequestId, TerminalEvent, TerminalTab};

const FRAME_COLS: u16 = 4;
const FRAME_ROWS: u16 = 2;
const LINK_COL: u16 = 2;
const LINK_ROW: u16 = 1;
const LINK_URL: &str = "https://osc.example/click";
const PLAIN_COL: u16 = 0;
const PLAIN_ROW: u16 = 0;
/// Bounded wait for the engine to publish the OSC-8 frame (async worker) — well
/// above the handful of paints it actually needs, low enough to fail fast.
const MAX_PRIME_POLLS: u32 = 600;
/// `cx.emit` delivers to subscribers on the effect cycle AFTER the render that
/// dispatched the click, so the link result must be polled across renders, not
/// read synchronously in the click's own render pass.
const MAX_RESULT_POLLS: u32 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    ClickPlainCell,
    ClickLinkCell,
    CheckLinkResult,
    Done,
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    prime_polls: Rc<RefCell<u32>>,
    result_polls: Rc<RefCell<u32>>,
    tab: gpui::Entity<TerminalTab>,
    captured: Rc<RefCell<Option<(HostRequestId, String)>>>,
    // Must be held: dropping the Subscription cancels the callback.
    _sub: gpui::Subscription,
}

fn fail(msg: &str) -> ! {
    eprintln!("presentation_realwindow FAIL: {msg}");
    std::process::exit(1);
}

/// OSC-8 feed placing the hyperlink on cell (LINK_COL, LINK_ROW):
/// `\r\n` → row 1 col 0; `..` → fill cols 0,1, cursor at col 2; then the OSC-8
/// open/close wrapping a single `X` glyph at col 2. Row 0 stays blank (plain).
fn osc8_feed() -> Vec<u8> {
    let mut feed = Vec::new();
    feed.extend_from_slice(b"\r\n..");
    feed.extend_from_slice(b"\x1b]8;;");
    feed.extend_from_slice(LINK_URL.as_bytes());
    feed.extend_from_slice(b"\x1b\\"); // ST
    feed.extend_from_slice(b"X");
    feed.extend_from_slice(b"\x1b]8;;\x1b\\"); // OSC-8 close
    feed
}

fn cell_center(origin: Point<Pixels>, metrics: &CellMetrics, col: u16, row: u16) -> Point<Pixels> {
    point(
        origin.x + metrics.cell_w * (f32::from(col) + 0.5),
        origin.y + metrics.cell_h * (f32::from(row) + 0.5),
    )
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = EngineConfig {
            cols: FRAME_COLS,
            rows: FRAME_ROWS,
            max_scrollback: 32,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let engine = Arc::new(EngineHandle::spawn(cfg).expect("spawn engine for test"));
        // Ensure the engine publishes frames, then feed the OSC-8 so the
        // engine-built frame (the one the render sampler hands to on_mouse_down)
        // carries the hyperlink.
        let _ = engine.set_visible(true);
        let _ = engine.feed(osc8_feed());
        let tab = TerminalTab::open_with_engine_for_test(Arc::clone(&engine), cx);

        let captured = Rc::new(RefCell::new(None));
        let captured_sub = Rc::clone(&captured);
        let sub = cx.subscribe(&tab, move |_this, _tab, event, _cx| {
            if let TerminalEvent::OpenUrlRequest { id, url } = event {
                *captured_sub.borrow_mut() = Some((*id, url.clone()));
            }
        });

        Self {
            phase: Rc::new(RefCell::new(Phase::PrimePaint)),
            prime_polls: Rc::new(RefCell::new(0)),
            result_polls: Rc::new(RefCell::new(0)),
            tab,
            captured,
            _sub: sub,
        }
    }

    fn paint_ready(&self, cx: &Context<Self>) -> bool {
        self.tab.read(cx).last_paint_origin_for_test().is_some()
            && self.tab.read(cx).cell_metrics_for_test().is_some()
    }

    /// True once the render loop has sampled an engine frame that carries the
    /// OSC-8 hyperlink on the link cell — i.e. the same frame `on_mouse_down`
    /// will hit-test.
    fn link_frame_ready(&self, cx: &Context<Self>) -> bool {
        let Some(frame) = self.tab.read(cx).latest_frame_for_test() else {
            return false;
        };
        let Some(row) = frame.grid.get(LINK_ROW as usize) else {
            return false;
        };
        row.cells
            .iter()
            .any(|c| c.col == LINK_COL && c.hyperlink_uri.as_deref() == Some(LINK_URL))
    }

    fn dispatch_click(&self, col: u16, row: u16, window: &mut Window, cx: &mut Context<Self>) {
        let origin = self
            .tab
            .read(cx)
            .last_paint_origin_for_test()
            .expect("paint origin");
        let metrics = self
            .tab
            .read(cx)
            .cell_metrics_for_test()
            .expect("cell metrics");
        let pos = cell_center(origin, &metrics, col, row);
        let tab = self.tab.clone();
        tab.update(cx, |tab, cx| {
            tab.debug_mouse_down_for_test(pos, window, cx);
        });
    }
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let phase = *self.phase.borrow();

        match phase {
            Phase::PrimePaint => {
                if self.paint_ready(cx) && self.link_frame_ready(cx) {
                    *self.phase.borrow_mut() = Phase::ClickPlainCell;
                } else {
                    let mut polls = self.prime_polls.borrow_mut();
                    *polls += 1;
                    if *polls > MAX_PRIME_POLLS {
                        fail("engine never published a frame carrying the OSC-8 link at (2,1)");
                    }
                }
            }
            Phase::ClickPlainCell => {
                // The plain-cell path returns early in `on_mouse_down` (no URI),
                // so it never schedules an emit — a synchronous None check is
                // sound here.
                self.dispatch_click(PLAIN_COL, PLAIN_ROW, window, cx);
                if self.captured.borrow().is_some() {
                    fail("plain-cell click must not emit OpenUrlRequest");
                }
                println!("presentation_realwindow: plain-cell click OK (no OpenUrlRequest)");
                *self.phase.borrow_mut() = Phase::ClickLinkCell;
            }
            Phase::ClickLinkCell => {
                // Dispatch the link click; its `cx.emit` is delivered on a later
                // effect cycle, so read the result in CheckLinkResult, not here.
                self.dispatch_click(LINK_COL, LINK_ROW, window, cx);
                *self.phase.borrow_mut() = Phase::CheckLinkResult;
            }
            Phase::CheckLinkResult => match self.captured.borrow().clone() {
                Some((id, url)) if url == LINK_URL => {
                    println!(
                        "presentation_realwindow: link-cell OpenUrlRequest OK id={id:?} url={url}"
                    );
                    *self.phase.borrow_mut() = Phase::Done;
                }
                Some((id, url)) => {
                    fail(&format!("unexpected OpenUrlRequest id={id:?} url={url}"));
                }
                None => {
                    let mut polls = self.result_polls.borrow_mut();
                    *polls += 1;
                    if *polls > MAX_RESULT_POLLS {
                        fail("link-cell click did not emit OpenUrlRequest");
                    }
                }
            },
            Phase::Done => {
                println!("presentation_realwindow: all phases OK");
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
                    title: Some("lens-terminal presentation_realwindow".into()),
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
