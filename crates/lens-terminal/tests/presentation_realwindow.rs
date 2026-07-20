//! Real-window presentation harness (Slice 2d Task 4).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens hit-testing.
//! This `harness = false` binary opens a real GPUI window, paints a focused
//! [`TerminalTab`] with a multi-cell OSC-8 frame, clicks a plain cell (negative)
//! then the link cell (positive), and asserts `TerminalEvent::OpenUrlRequest`
//! only on the correct cell (click only — hover deferred).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Application, Bounds, Context, Pixels, Point, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, point, prelude::*, px, size,
};
use lens_terminal::render_test_api::CellMetrics;
use lens_terminal::{
    CellStyle, EngineConfig, EngineHandle, Frame, FrameCell, FrameRow, HostRequestId, Rgb,
    TerminalEvent, TerminalTab,
};

const FRAME_COLS: u16 = 4;
const FRAME_ROWS: u16 = 2;
const LINK_COL: u16 = 2;
const LINK_ROW: u16 = 1;
const LINK_URL: &str = "https://osc.example/click";
const PLAIN_COL: u16 = 0;
const PLAIN_ROW: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    ClickPlainCell,
    ClickLinkCell,
    Done,
}

struct HarnessView {
    phase: Rc<RefCell<Phase>>,
    tab: gpui::Entity<TerminalTab>,
    captured: Rc<RefCell<Option<(HostRequestId, String)>>>,
}

fn fail(msg: &str) -> ! {
    eprintln!("presentation_realwindow FAIL: {msg}");
    std::process::exit(1);
}

fn default_fg() -> Rgb {
    Rgb {
        r: 200,
        g: 200,
        b: 200,
    }
}

fn frame_cell(col: u16, grapheme: char, hyperlink_uri: Option<Arc<str>>) -> FrameCell {
    FrameCell {
        col,
        grapheme: grapheme.to_string(),
        fg: default_fg(),
        bg: None,
        wide: false,
        selected: false,
        style: CellStyle::default(),
        hyperlink_uri,
    }
}

fn osc8_frame() -> Frame {
    let link_uri = Arc::<str>::from(LINK_URL);
    let mut grid = Vec::with_capacity(usize::from(FRAME_ROWS));
    for row in 0..FRAME_ROWS {
        let mut cells = Vec::with_capacity(usize::from(FRAME_COLS));
        for col in 0..FRAME_COLS {
            let is_link = row == LINK_ROW && col == LINK_COL;
            cells.push(frame_cell(
                col,
                if is_link { 'X' } else { '.' },
                if is_link {
                    Some(Arc::clone(&link_uri))
                } else {
                    None
                },
            ));
        }
        grid.push(FrameRow { cells });
    }
    Frame {
        cols: FRAME_COLS,
        rows: FRAME_ROWS,
        default_fg: default_fg(),
        default_bg: Rgb { r: 0, g: 0, b: 0 },
        grid,
        cursor: None,
    }
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
        let engine = Arc::new(EngineHandle::spawn(cfg));
        let tab = TerminalTab::open_with_engine_for_test(Arc::clone(&engine), cx);
        tab.update(cx, |tab, cx| {
            tab.set_frame_for_test(Arc::new(osc8_frame()), cx);
        });

        let captured = Rc::new(RefCell::new(None));
        let captured_sub = Rc::clone(&captured);
        let _ = cx.subscribe(&tab, move |_this, _tab, event, _cx| {
            if let TerminalEvent::OpenUrlRequest { id, url } = event {
                *captured_sub.borrow_mut() = Some((*id, url.clone()));
            }
        });

        Self {
            phase: Rc::new(RefCell::new(Phase::PrimePaint)),
            tab,
            captured,
        }
    }

    fn paint_ready(&self, cx: &Context<Self>) -> bool {
        self.tab.read(cx).last_paint_origin_for_test().is_some()
            && self.tab.read(cx).cell_metrics_for_test().is_some()
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
                if self.paint_ready(cx) {
                    *self.phase.borrow_mut() = Phase::ClickPlainCell;
                }
            }
            Phase::ClickPlainCell => {
                self.dispatch_click(PLAIN_COL, PLAIN_ROW, window, cx);
                if self.captured.borrow().is_some() {
                    fail("plain-cell click must not emit OpenUrlRequest");
                }
                println!("presentation_realwindow: plain-cell click OK (no OpenUrlRequest)");
                *self.phase.borrow_mut() = Phase::ClickLinkCell;
            }
            Phase::ClickLinkCell => {
                self.dispatch_click(LINK_COL, LINK_ROW, window, cx);
                match self.captured.borrow().clone() {
                    Some((id, url)) if url == LINK_URL => {
                        println!(
                            "presentation_realwindow: link-cell OpenUrlRequest OK id={id:?} url={url}"
                        );
                        *self.phase.borrow_mut() = Phase::Done;
                    }
                    Some((id, url)) => {
                        fail(&format!("unexpected OpenUrlRequest id={id:?} url={url}"));
                    }
                    None => fail("link-cell click did not emit OpenUrlRequest"),
                }
            }
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
