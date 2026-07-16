//! Spike A — GPUI terminal-grid paint viability harness.
//!
//! Task 2: wire `full_redraw` at 80×24 through the liftable paint mapping.

mod fixtures;
mod paint;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Application, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
    canvas, point, prelude::*, px, size,
};
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::{Terminal, TerminalOptions};

use paint::{CellMetrics, Strategy, TextPlacement, paint_grid, per_row_alignment_ok};

struct VtEngine {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    cols: u16,
    rows_n: u16,
    frame_n: u64,
    seeded: bool,
}

impl VtEngine {
    fn new(cols: u16, rows_n: u16) -> Self {
        let terminal = Terminal::new(TerminalOptions {
            cols,
            rows: rows_n,
            max_scrollback: 1000,
        })
        .expect("terminal");
        let render_state = RenderState::new().expect("render state");
        let rows = RowIterator::new().expect("row iterator");
        let cells = CellIterator::new().expect("cell iterator");
        Self {
            terminal,
            render_state,
            rows,
            cells,
            cols,
            rows_n,
            frame_n: 0,
            seeded: false,
        }
    }
}

struct GridView {
    vt: Rc<RefCell<VtEngine>>,
    metrics: Option<CellMetrics>,
    placement: TextPlacement,
    alignment_logged: bool,
}

impl GridView {
    fn new(cols: u16, rows: u16, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            vt: Rc::new(RefCell::new(VtEngine::new(cols, rows))),
            metrics: None,
            placement: TextPlacement::PerRow,
            alignment_logged: false,
        }
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        if self.metrics.is_none() {
            let m = CellMetrics::resolve(window);
            if !self.alignment_logged {
                let ok = per_row_alignment_ok(window, &m);
                eprintln!(
                    "alignment probe (per-row shape_line vs grid): {}",
                    if ok { "OK" } else { "MISALIGNED → using PerCell" }
                );
                if !ok {
                    self.placement = TextPlacement::PerCell;
                }
                self.alignment_logged = true;
            }
            self.metrics = Some(m);
        }

        let vt = Rc::clone(&self.vt);
        let metrics = self.metrics.clone().expect("metrics");
        let placement = self.placement;

        canvas(
            |_bounds, _window, _cx| {},
            move |bounds, _prepaint, window, cx| {
                let mut vt = vt.borrow_mut();
                // full_redraw every frame → Dirty::Full
                let bytes = fixtures::full_redraw(vt.cols, vt.rows_n);
                if !vt.seeded {
                    eprintln!(
                        "painting full_redraw {}×{} placement={placement:?}",
                        vt.cols, vt.rows_n
                    );
                    vt.seeded = true;
                }
                vt.terminal.vt_write(&bytes);
                vt.frame_n += 1;

                let VtEngine {
                    terminal,
                    render_state,
                    rows,
                    cells,
                    ..
                } = &mut *vt;
                let snapshot = render_state.update(terminal).expect("render_state.update");
                let origin = point(bounds.origin.x + px(4.0), bounds.origin.y + px(4.0));
                let _ = paint_grid(
                    &snapshot,
                    rows,
                    cells,
                    origin,
                    &metrics,
                    Strategy::S1,
                    placement,
                    None,
                    window,
                    cx,
                )
                .expect("paint_grid");
            },
        )
        .size_full()
    }
}

fn main() {
    let cols: u16 = 80;
    let rows: u16 = 24;
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("terminal-render spike (Task 2)".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(900.0), px(600.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| GridView::new(cols, rows, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
