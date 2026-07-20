//! Real-window presentation harness (Slice 2d Task 4).
//!
//! NOT under `#[gpui::test]`: gpui's `NoopTextSystem` false-greens hit-testing.
//! This `harness = false` binary opens a real GPUI window, paints a focused
//! [`TerminalTab`] with an OSC-8 hyperlink cell, and dispatches a left
//! mouse-down at the cell center. Asserts `TerminalEvent::OpenUrlRequest`
//! (click only — hover deferred).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Application, Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
    point, prelude::*, px, size,
};
use lens_terminal::{
    CellStyle, EngineConfig, EngineHandle, Frame, FrameCell, FrameRow, HostRequestId, Rgb,
    TerminalEvent, TerminalTab,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    PrimePaint,
    DispatchClick,
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

fn osc8_frame() -> Frame {
    let uri = Arc::<str>::from("https://osc.example/click");
    Frame {
        cols: 1,
        rows: 1,
        default_fg: Rgb {
            r: 200,
            g: 200,
            b: 200,
        },
        default_bg: Rgb { r: 0, g: 0, b: 0 },
        grid: vec![FrameRow {
            cells: vec![FrameCell {
                col: 0,
                grapheme: "X".into(),
                fg: Rgb {
                    r: 200,
                    g: 200,
                    b: 200,
                },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
                hyperlink_uri: Some(uri),
            }],
        }],
        cursor: None,
    }
}

impl HarnessView {
    fn new(cx: &mut Context<Self>) -> Self {
        let cfg = EngineConfig {
            cols: 10,
            rows: 5,
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
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let phase = *self.phase.borrow();

        match phase {
            Phase::PrimePaint => {
                let origin = self.tab.read(cx).last_paint_origin_for_test();
                let metrics = self.tab.read(cx).cell_metrics_for_test();
                if origin.is_some() && metrics.is_some() {
                    *self.phase.borrow_mut() = Phase::DispatchClick;
                }
            }
            Phase::DispatchClick => {
                let origin = self
                    .tab
                    .read(cx)
                    .last_paint_origin_for_test()
                    .expect("paint origin after prime");
                let metrics = self
                    .tab
                    .read(cx)
                    .cell_metrics_for_test()
                    .expect("cell metrics after prime");
                let pos = point(
                    origin.x + metrics.cell_w * 0.5,
                    origin.y + metrics.cell_h * 0.5,
                );
                let tab = self.tab.clone();
                tab.update(cx, |tab, cx| {
                    tab.debug_mouse_down_for_test(pos, window, cx);
                });
                let captured = self.captured.borrow();
                match captured.as_ref() {
                    Some((id, url)) if url == "https://osc.example/click" => {
                        println!("presentation_realwindow: OpenUrlRequest OK id={id:?} url={url}");
                        *self.phase.borrow_mut() = Phase::Done;
                    }
                    Some((id, url)) => {
                        fail(&format!("unexpected OpenUrlRequest id={id:?} url={url}"));
                    }
                    None => fail("click did not emit OpenUrlRequest"),
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
