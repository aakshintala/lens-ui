//! Real-window mouse harness (Slice 2c Task 8).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens hit-testing,
//! so mouse correctness must be proven against a REAL painted window. This
//! `harness = false` binary opens one window and drives the FULL production
//! mouse path (foreground lowering → ordered engine command stream → arbiter →
//! egress / presentation) against a real engine, in four phases:
//!
//!   P-localclick  no tracking, a click (down+up, no drag) on an OSC-8 link cell
//!                 emits `OpenUrlRequest` for that URL; a plain cell does not.
//!   P-select+copy no tracking, a Left drag selects cells (painted `selected`)
//!                 and `Cmd+C` copy writes the selection text to the clipboard.
//!   P-report      mouse tracking ON, a Left down emits an SGR mouse report to
//!                 the PTY egress.
//!   P-readonly    tracking ON but engine read-only (`SetAccess(false)`), a Left
//!                 down does NOT egress (report suppressed) yet still selects.
//!
//! Pitfalls (memory `terminal-realwindow-harness-pitfalls`): (1) frames are
//! driven THROUGH the engine (`feed`) and polled via `latest_frame_for_test`
//! because the render sampler clobbers any injected frame; (2) `cx.emit` and the
//! two-stage copy land on a LATER effect cycle, so results are polled across
//! renders, never read synchronously; (3) the `Subscription` is held on the view.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Application, Bounds, ClipboardItem, Context, MouseButton, Pixels, Point, Render,
    TitlebarOptions, Window, WindowBounds, WindowOptions, point, prelude::*, px, size,
};
use lens_terminal::render_test_api::CellMetrics;
use lens_terminal::{
    EgressFrame, EngineConfig, EngineHandle, HostRequestId, TerminalEvent, TerminalTab,
};

const FRAME_COLS: u16 = 8;
const FRAME_ROWS: u16 = 3;
const LINK_COL: u16 = 2;
const LINK_ROW: u16 = 1;
const LINK_URL: &str = "https://osc.example/click";
const PLAIN_COL: u16 = 0;
const PLAIN_ROW: u16 = 0;
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;

const MAX_PRIME_POLLS: u32 = 600;
const MAX_RESULT_POLLS: u32 = 240;
/// Renders to wait after feeding a mode change before it is guaranteed applied by
/// the async worker (µs of work; this is generous slack, not a tight bound).
const MODE_SETTLE_POLLS: u32 = 60;
/// Renders to hold while confirming NO egress arrives (read-only suppression).
const NO_EGRESS_POLLS: u32 = 60;
const EGRESS_RECV: Duration = Duration::from_millis(2);

fn fail(msg: &str) -> ! {
    eprintln!("mouse_realwindow FAIL: {msg}");
    std::process::exit(1);
}

/// Content feed (no tracking): "copyme" on row 0 (selectable), and an OSC-8
/// hyperlink glyph `X` at (LINK_COL, LINK_ROW). Row 0 cols 0..5 are plain text.
fn content_feed() -> Vec<u8> {
    let mut feed = Vec::new();
    feed.extend_from_slice(b"copyme"); // row 0, cols 0..5
    feed.extend_from_slice(b"\r\n.."); // row 1, cols 0,1; cursor at col 2
    feed.extend_from_slice(b"\x1b]8;;");
    feed.extend_from_slice(LINK_URL.as_bytes());
    feed.extend_from_slice(b"\x1b\\"); // ST
    feed.extend_from_slice(b"X"); // link glyph at (2,1)
    feed.extend_from_slice(b"\x1b]8;;\x1b\\"); // OSC-8 close
    feed
}

fn cell_center(origin: Point<Pixels>, metrics: &CellMetrics, col: u16, row: u16) -> Point<Pixels> {
    point(
        origin.x + metrics.cell_w * (f32::from(col) + 0.5),
        origin.y + metrics.cell_h * (f32::from(row) + 0.5),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    ClickPlainNegative,
    ClickLink,
    CheckLink,
    SelectDrag,
    CheckSelectThenCopy,
    CheckCopy,
    EnableTracking,
    ReportDown,
    SetReadOnly,
    ReadOnlyDownNoEgress,
    Done,
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    counter: Rc<RefCell<u32>>,
    tab: gpui::Entity<TerminalTab>,
    engine: Arc<EngineHandle>,
    egress: crossbeam_channel::Receiver<EgressFrame>,
    captured: Rc<RefCell<Option<(HostRequestId, String)>>>,
    _sub: gpui::Subscription,
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = EngineConfig {
            cols: FRAME_COLS,
            rows: FRAME_ROWS,
            max_scrollback: 32,
            cell_w_px: CELL_W,
            cell_h_px: CELL_H,
        };
        let engine = Arc::new(EngineHandle::spawn(cfg).expect("spawn engine for test"));
        let egress = engine.attach_test_egress();
        let _ = engine.set_visible(true);
        let _ = engine.feed(content_feed());
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
            counter: Rc::new(RefCell::new(0)),
            tab,
            engine,
            egress,
            captured,
            _sub: sub,
        }
    }

    fn set_phase(&self, p: Phase) {
        *self.phase.borrow_mut() = p;
        *self.counter.borrow_mut() = 0;
    }

    fn tick(&self, cap: u32, what: &str) {
        let mut c = self.counter.borrow_mut();
        *c += 1;
        if *c > cap {
            fail(what);
        }
    }

    fn paint_ready(&self, cx: &Context<Self>) -> bool {
        self.tab.read(cx).last_paint_origin_for_test().is_some()
            && self.tab.read(cx).cell_metrics_for_test().is_some()
    }

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

    fn drain_egress(&self) {
        while self.egress.try_recv().is_ok() {}
    }

    fn origin_metrics(&self, cx: &Context<Self>) -> (Point<Pixels>, CellMetrics) {
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
        (origin, metrics)
    }

    fn down(&self, col: u16, row: u16, window: &mut Window, cx: &mut Context<Self>) {
        let (origin, metrics) = self.origin_metrics(cx);
        let pos = cell_center(origin, &metrics, col, row);
        self.tab.clone().update(cx, |tab, cx| {
            tab.debug_mouse_down_for_test(pos, window, cx);
        });
    }

    fn up(&self, col: u16, row: u16, window: &mut Window, cx: &mut Context<Self>) {
        let (origin, metrics) = self.origin_metrics(cx);
        let pos = cell_center(origin, &metrics, col, row);
        self.tab.clone().update(cx, |tab, cx| {
            tab.debug_mouse_up_for_test(pos, MouseButton::Left, window, cx);
        });
    }

    fn move_to(&self, col: u16, row: u16, window: &mut Window, cx: &mut Context<Self>) {
        let (origin, metrics) = self.origin_metrics(cx);
        let pos = cell_center(origin, &metrics, col, row);
        self.tab.clone().update(cx, |tab, cx| {
            tab.debug_mouse_move_for_test(pos, Some(MouseButton::Left), window, cx);
        });
    }

    fn selected_cols_row0(&self, cx: &Context<Self>) -> Vec<u16> {
        let Some(frame) = self.tab.read(cx).latest_frame_for_test() else {
            return Vec::new();
        };
        frame.grid[0]
            .cells
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.col)
            .collect()
    }
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let phase = *self.phase.borrow();

        match phase {
            Phase::PrimePaint => {
                if self.paint_ready(cx) && self.link_frame_ready(cx) {
                    self.set_phase(Phase::ClickPlainNegative);
                } else {
                    self.tick(MAX_PRIME_POLLS, "engine never published the primed frame");
                }
            }
            Phase::ClickPlainNegative => {
                // A click on a plain text cell emits a LocalClick but resolves to no URL.
                self.down(PLAIN_COL, PLAIN_ROW, window, cx);
                self.up(PLAIN_COL, PLAIN_ROW, window, cx);
                // The negative is confirmed after CheckLink proves the positive still fires;
                // capture must stay empty through the link click's own dispatch.
                if self.captured.borrow().is_some() {
                    fail("plain-cell click must not emit OpenUrlRequest");
                }
                println!("mouse_realwindow: P-localclick plain cell OK (no OpenUrlRequest)");
                self.set_phase(Phase::ClickLink);
            }
            Phase::ClickLink => {
                self.down(LINK_COL, LINK_ROW, window, cx);
                self.up(LINK_COL, LINK_ROW, window, cx);
                self.set_phase(Phase::CheckLink);
            }
            Phase::CheckLink => match self.captured.borrow().clone() {
                Some((_, url)) if url == LINK_URL => {
                    println!("mouse_realwindow: P-localclick link OpenUrlRequest OK url={url}");
                    self.set_phase(Phase::SelectDrag);
                }
                Some((id, url)) => fail(&format!("unexpected OpenUrlRequest id={id:?} url={url}")),
                None => self.tick(MAX_RESULT_POLLS, "link click did not emit OpenUrlRequest"),
            },
            Phase::SelectDrag => {
                // No tracking: Left down + drag selects. Drag across cols 0..3 on row 0.
                self.down(0, 0, window, cx);
                self.move_to(3, 0, window, cx);
                self.up(3, 0, window, cx);
                self.set_phase(Phase::CheckSelectThenCopy);
            }
            Phase::CheckSelectThenCopy => {
                let cols = self.selected_cols_row0(cx);
                if cols.contains(&0) && cols.contains(&3) {
                    println!("mouse_realwindow: P-select drag painted selection OK cols={cols:?}");
                    // Clear the clipboard sentinel then trigger the async two-stage copy.
                    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                    self.tab.clone().update(cx, |tab, cx| {
                        tab.debug_handle_copy_for_test(cx);
                    });
                    self.set_phase(Phase::CheckCopy);
                } else {
                    self.tick(MAX_RESULT_POLLS, "drag did not paint a selection on row 0");
                }
            }
            Phase::CheckCopy => {
                let clip = cx.read_from_clipboard().and_then(|c| c.text());
                match clip.as_deref() {
                    Some(t) if t.contains("copy") => {
                        println!("mouse_realwindow: P-copy clipboard OK text={t:?}");
                        self.set_phase(Phase::EnableTracking);
                    }
                    _ => self.tick(MAX_RESULT_POLLS, "Cmd+C did not write the selection text"),
                }
            }
            Phase::EnableTracking => {
                let c = *self.counter.borrow();
                if c == 0 {
                    let _ = self.engine.feed(b"\x1b[?1000h\x1b[?1006h".to_vec()); // Normal + SGR
                    self.drain_egress();
                }
                if c >= MODE_SETTLE_POLLS {
                    self.set_phase(Phase::ReportDown);
                } else {
                    *self.counter.borrow_mut() += 1;
                }
            }
            Phase::ReportDown => {
                let c = *self.counter.borrow();
                if c == 0 {
                    self.drain_egress();
                    self.down(1, 1, window, cx);
                }
                match self.egress.recv_timeout(EGRESS_RECV) {
                    Ok(frame) => {
                        // SGR mouse report: CSI < ... M. Left press at (col 1,row 1) => 1-based 2;2.
                        if frame.bytes.starts_with(b"\x1b[<") {
                            println!(
                                "mouse_realwindow: P-report SGR egress OK bytes={:?}",
                                String::from_utf8_lossy(&frame.bytes)
                            );
                            self.up(1, 1, window, cx);
                            self.drain_egress();
                            self.set_phase(Phase::SetReadOnly);
                        } else {
                            fail(&format!("report egress not SGR: {:?}", frame.bytes));
                        }
                    }
                    Err(_) => self.tick(
                        MAX_RESULT_POLLS,
                        "tracked Left down did not egress a report",
                    ),
                }
            }
            Phase::SetReadOnly => {
                self.engine.debug_set_access_for_test(false);
                self.drain_egress();
                self.set_phase(Phase::ReadOnlyDownNoEgress);
            }
            Phase::ReadOnlyDownNoEgress => {
                let c = *self.counter.borrow();
                if c == 0 {
                    // Settle the ordered SetAccess(false), then a tracked Left down must NOT
                    // egress (report suppressed) — it falls back to local selection instead.
                    self.down(1, 1, window, cx);
                }
                if let Ok(frame) = self.egress.try_recv() {
                    fail(&format!(
                        "read-only tracked down must not egress, got {:?}",
                        frame.bytes
                    ));
                }
                if c >= NO_EGRESS_POLLS {
                    self.up(1, 1, window, cx);
                    println!("mouse_realwindow: P-readonly suppressed report OK (no egress)");
                    self.set_phase(Phase::Done);
                } else {
                    *self.counter.borrow_mut() += 1;
                }
            }
            Phase::Done => {
                println!("mouse_realwindow: all phases OK");
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
                    title: Some("lens-terminal mouse_realwindow".into()),
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
