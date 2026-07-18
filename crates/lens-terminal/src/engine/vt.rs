//! Owns the `libghostty-vt` `Terminal` and builds Lens-owned [`Frame`] snapshots.
//! **The only module that names a `libghostty_vt` type.**

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{RgbColor, Style, StyleColor, Underline};
#[cfg(test)]
use libghostty_vt::terminal::ScrollViewport;
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace};
use libghostty_vt::{Terminal, TerminalOptions};
use thiserror::Error;

use super::command::{KeyInput, ScrollDelta};
use super::frame::{CellStyle, CursorPos, Frame, FrameCell, FrameRow, Rgb, UnderlineStyle};
use super::key_map::encode_key_pure;

type OnReplyFn = Box<dyn FnMut(&[u8]) + 'static>;

/// Engine thread configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub cols: u16,
    pub rows: u16,
    pub max_scrollback: usize,
    pub cell_w_px: u32,
    pub cell_h_px: u32,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Vt(#[from] libghostty_vt::error::Error),
}

/// Non-`Send` VT engine — lives on the dedicated worker thread only.
pub struct VtEngine {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    key_encoder: libghostty_vt::key::Encoder<'static>,
    key_event: libghostty_vt::key::Event<'static>,
    cell_w_px: u32,
    cell_h_px: u32,
    reply_buffer: Rc<RefCell<Vec<u8>>>,
    #[expect(dead_code, reason = "worker invokes after take_replies in Task 4")]
    on_reply: OnReplyFn,
}

impl VtEngine {
    /// Construct a terminal with an `on_pty_write` reply buffer.
    pub fn new(
        cfg: &EngineConfig,
        on_reply: impl FnMut(&[u8]) + 'static,
    ) -> Result<Self, EngineError> {
        let reply_buffer = Rc::new(RefCell::new(Vec::new()));
        let buf = Rc::clone(&reply_buffer);
        let mut terminal = Terminal::new(TerminalOptions {
            cols: cfg.cols,
            rows: cfg.rows,
            max_scrollback: cfg.max_scrollback,
        })?;
        terminal.on_pty_write(move |_term, data| {
            buf.borrow_mut().extend_from_slice(data);
        })?;

        Ok(Self {
            terminal,
            render_state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            key_encoder: libghostty_vt::key::Encoder::new()?,
            key_event: libghostty_vt::key::Event::new()?,
            cell_w_px: cfg.cell_w_px,
            cell_h_px: cfg.cell_h_px,
            reply_buffer,
            on_reply: Box::new(on_reply) as OnReplyFn,
        })
    }

    /// Encode a key event against the terminal's live modes.
    pub(crate) fn encode_key(&mut self, input: &KeyInput) -> Result<Vec<u8>, EngineError> {
        self.key_encoder.set_options_from_terminal(&self.terminal);
        // `set_options_from_terminal` resets macOS option-as-alt to False; Lens
        // treats Option as Alt for PTY encoding (ESC-prefix on printable keys).
        #[cfg(target_os = "macos")]
        self.key_encoder
            .set_macos_option_as_alt(libghostty_vt::key::OptionAsAlt::True);
        let mut buf = Vec::new();
        encode_key_pure(&mut self.key_encoder, &mut self.key_event, input, &mut buf)?;
        Ok(buf)
    }

    /// Encode a focus gained/lost report when mode 1004 is enabled.
    pub(crate) fn encode_focus_report(
        &mut self,
        focused: bool,
    ) -> Result<Option<Vec<u8>>, EngineError> {
        use libghostty_vt::focus::Event as FocusEv;
        use libghostty_vt::terminal::Mode;
        if !self.terminal.mode(Mode::FOCUS_EVENT)? {
            return Ok(None);
        }
        let ev = if focused {
            FocusEv::Gained
        } else {
            FocusEv::Lost
        };
        let mut buf = [0u8; 16];
        let n = ev.encode(&mut buf)?;
        Ok(Some(buf[..n].to_vec()))
    }

    /// Scroll the viewport locally (no PTY egress).
    pub(crate) fn local_scroll(&mut self, delta: ScrollDelta) {
        use libghostty_vt::terminal::ScrollViewport;
        let scroll = match delta {
            ScrollDelta::Lines(n) => ScrollViewport::Delta(n as isize),
            ScrollDelta::Top => ScrollViewport::Top,
            ScrollDelta::Bottom => ScrollViewport::Bottom,
        };
        self.terminal.scroll_viewport(scroll);
    }

    /// Feed server VT bytes into the terminal.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.vt_write(bytes);
    }

    /// Drain bytes accumulated by `on_pty_write` since the last drain.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.reply_buffer.borrow_mut().drain(..).collect()
    }

    /// Resize the grid; reflows content when wraparound is enabled.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), EngineError> {
        self.terminal
            .resize(cols, rows, self.cell_w_px, self.cell_h_px)?;
        Ok(())
    }

    /// Snapshot the visible grid into an owned [`Frame`].
    pub fn build_frame(&mut self) -> Result<Frame, EngineError> {
        let snapshot = self.render_state.update(&self.terminal)?;
        let colors = snapshot.colors()?;
        let default_fg = rgb_from_ghostty(colors.foreground);
        let default_bg = rgb_from_ghostty(colors.background);
        let cols = snapshot.cols()?;
        let rows = snapshot.rows()?;

        let mut grid = Vec::new();
        let mut row_iter = self.rows.update(&snapshot)?;
        while let Some(row) = row_iter.next() {
            let mut cell_iter = self.cells.update(row)?;
            let mut row_cells = Vec::new();
            let mut col: u16 = 0;
            while let Some(cell) = cell_iter.next() {
                let this_col = col;
                col = col.saturating_add(1);

                let wide = cell.raw_cell()?.wide()?;
                if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                    continue;
                }

                let graphemes = cell.graphemes()?;
                let fg = cell.fg_color()?.map(rgb_from_ghostty).unwrap_or(default_fg);
                let bg = cell.bg_color()?.map(rgb_from_ghostty);
                let style = cell_style_from_ghostty(cell.style()?);
                let selected = cell.is_selected()?;
                let grapheme = if graphemes.is_empty() {
                    " ".to_owned()
                } else {
                    graphemes.iter().collect()
                };

                row_cells.push(FrameCell {
                    col: this_col,
                    grapheme,
                    fg,
                    bg,
                    wide: matches!(wide, CellWide::Wide),
                    selected,
                    style,
                });
            }
            grid.push(FrameRow { cells: row_cells });
        }

        Ok(Frame {
            cols,
            rows,
            default_fg,
            default_bg,
            grid,
            cursor: viewport_cursor_pos(&self.terminal, cols, rows),
        })
    }
}

fn viewport_cursor_pos(term: &Terminal<'_, '_>, cols: u16, rows: u16) -> Option<CursorPos> {
    // cursor_x/y are ACTIVE-AREA coords — NEVER unwrap_or(0).
    if !term.is_cursor_visible().ok()? {
        return None;
    }
    let ax = term.cursor_x().ok()?;
    let ay = term.cursor_y().ok()?;
    let grid_ref = term
        .grid_ref(Point::Active(PointCoordinate {
            x: ax,
            y: u32::from(ay),
        }))
        .ok()?;
    let vp = term
        .point_from_grid_ref(&grid_ref, PointSpace::Viewport)
        .ok()??;
    if vp.x >= cols || vp.y >= u32::from(rows) {
        return None;
    }
    let row = u16::try_from(vp.y).ok()?;
    Some(CursorPos { col: vp.x, row })
}

fn rgb_from_ghostty(c: RgbColor) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

fn cell_style_from_ghostty(style: Style) -> CellStyle {
    CellStyle {
        bold: style.bold,
        italic: style.italic,
        faint: style.faint,
        blink: style.blink,
        inverse: style.inverse,
        invisible: style.invisible,
        strikethrough: style.strikethrough,
        overline: style.overline,
        underline: underline_from_ghostty(style.underline),
        underline_color: match style.underline_color {
            StyleColor::Rgb(c) => Some(rgb_from_ghostty(c)),
            StyleColor::None | StyleColor::Palette(_) => None,
        },
    }
}

fn underline_from_ghostty(u: Underline) -> UnderlineStyle {
    match u {
        Underline::None => UnderlineStyle::None,
        Underline::Single => UnderlineStyle::Single,
        Underline::Double => UnderlineStyle::Double,
        Underline::Curly => UnderlineStyle::Curly,
        Underline::Dotted => UnderlineStyle::Dotted,
        Underline::Dashed => UnderlineStyle::Dashed,
        _ => UnderlineStyle::None,
    }
}

#[cfg(test)]
impl VtEngine {
    pub fn scrollback_rows_for_test(&self) -> usize {
        self.terminal.scrollback_rows().unwrap_or(0)
    }

    fn scroll_viewport_for_test(&mut self, scroll: ScrollViewport) {
        self.terminal.scroll_viewport(scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EngineConfig {
        EngineConfig {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
            cell_w_px: 8,
            cell_h_px: 16,
        }
    }

    #[test]
    fn builds_frame_with_sgr_and_colors() {
        let mut e = VtEngine::new(&test_config(), |_| {}).unwrap();
        e.feed(b"\x1b[1;31mHi\x1b[0m\r\n");
        let f = e.build_frame().unwrap();
        assert_eq!((f.cols, f.rows), (20, 3));
        let c0 = &f.grid[0].cells[0];
        assert_eq!(c0.grapheme, "H");
        assert!(c0.style.bold);
        assert_eq!(
            c0.fg,
            Rgb {
                r: 204,
                g: 102,
                b: 102
            }
        );
    }

    #[test]
    fn builds_frame_with_wide_chars_and_emoji() {
        let mut e = VtEngine::new(&test_config(), |_| {}).unwrap();
        e.feed("a日b😀c".as_bytes());
        let f = e.build_frame().unwrap();
        let row = &f.grid[0].cells;
        assert_eq!(row[0].grapheme, "a");
        assert!(!row[0].wide);
        assert_eq!(row[0].col, 0);

        let wide = row
            .iter()
            .find(|c| c.grapheme == "日")
            .expect("CJK wide cell");
        assert!(wide.wide);
        assert_eq!(wide.col, 1);

        let emoji = row.iter().find(|c| c.grapheme == "😀").expect("emoji cell");
        assert!(emoji.wide);
        assert_eq!(emoji.col, 4);

        let cols: Vec<u16> = row.iter().map(|c| c.col).collect();
        assert!(cols.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn build_frame_cursor_none_when_scrolled_out_of_viewport() {
        let mut e = VtEngine::new(&test_config(), |_| {}).unwrap();
        for i in 0..20 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        let f = e.build_frame().unwrap();
        assert_eq!(
            f.cursor,
            Some(CursorPos { col: 0, row: 2 }),
            "cursor must be visible in viewport before scroll"
        );
        e.scroll_viewport_for_test(ScrollViewport::Top);
        let f = e.build_frame().unwrap();
        assert!(
            f.cursor.is_none(),
            "cursor must be None when scrolled out of viewport, got {:?}",
            f.cursor
        );
    }
}
