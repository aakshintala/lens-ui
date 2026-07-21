//! Owns the `libghostty-vt` `Terminal` and builds Lens-owned [`Frame`] snapshots.
//! **The only module that names a `libghostty_vt` type.**

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use crossbeam_channel::{Sender, TrySendError};
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
use super::presentation::{
    ClipboardLocation, ClipboardMimePart, EnginePresentationEvent, MAX_HYPERLINK_URI_BYTES,
    MAX_OSC52_CLIPBOARD_BYTES, TitleUpdate, sanitize_reported_title,
};
use super::worker::WakerSlot;
use crate::engine::inspect::InspectShared;

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
    latest_title_slot: Arc<ArcSwapOption<TitleUpdate>>,
}

impl VtEngine {
    /// Construct a terminal with an `on_pty_write` reply buffer.
    pub fn new(
        cfg: &EngineConfig,
        on_reply: impl FnMut(&[u8]) + 'static,
        presentation_tx: Sender<EnginePresentationEvent>,
    ) -> Result<Self, EngineError> {
        Self::new_shared(
            cfg,
            on_reply,
            presentation_tx,
            Arc::new(ArcSwapOption::from(None)),
            None,
            None,
        )
    }

    pub(crate) fn new_shared(
        cfg: &EngineConfig,
        on_reply: impl FnMut(&[u8]) + 'static,
        presentation_tx: Sender<EnginePresentationEvent>,
        latest_title_slot: Arc<ArcSwapOption<TitleUpdate>>,
        waker: Option<WakerSlot>,
        inspect: Option<Arc<InspectShared>>,
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

        // Bare title enqueue — Task 2 wraps with sanitize/bound inside this closure.
        // Slice 2b re-threads `presentation_tx` for `on_clipboard_write`; the title
        // path may already hold one clone of that sender.
        let title_slot = Arc::clone(&latest_title_slot);
        let title_tx = presentation_tx;
        let clip_tx = title_tx.clone();
        let waker_for_clip = waker.clone();
        let inspect_for_clip = inspect.clone();
        let waker_for_title = waker.clone();
        let inspect_for_title = inspect.clone();
        terminal.on_title_changed(move |term| {
            let Ok(title) = term.title() else {
                return;
            };
            let wake = || {
                if let Some(w) = waker_for_title.as_ref()
                    && let Ok(guard) = w.lock()
                    && let Some(f) = guard.as_ref()
                {
                    f();
                }
            };
            match sanitize_reported_title(title) {
                Some(clean) => {
                    if let Some(insp) = inspect_for_title.as_ref()
                        && insp.is_enabled()
                        && title_slot.load().is_some()
                    {
                        insp.record_title_slot_overwrite();
                    }
                    title_slot.store(Some(Arc::new(TitleUpdate::Set(clean.clone()))));
                    if let Err(TrySendError::Full(_)) =
                        title_tx.try_send(EnginePresentationEvent::TitleChanged(clean))
                        && let Some(insp) = inspect_for_title.as_ref()
                    {
                        insp.record_presentation_channel_full_drop();
                    }
                    wake();
                }
                None => {
                    if let Some(insp) = inspect_for_title.as_ref()
                        && insp.is_enabled()
                        && title_slot.load().is_some()
                    {
                        insp.record_title_slot_overwrite();
                    }
                    title_slot.store(Some(Arc::new(TitleUpdate::Clear)));
                    if let Err(TrySendError::Full(_)) =
                        title_tx.try_send(EnginePresentationEvent::TitleChanged(String::new()))
                        && let Some(insp) = inspect_for_title.as_ref()
                    {
                        insp.record_presentation_channel_full_drop();
                    }
                    wake();
                }
            }
        })?;

        // --- 2b: OSC 52 clipboard-write effect (result IGNORED by OSC 52; cap + forward only) ---
        terminal.on_clipboard_write(move |_term, write| {
            let location = map_clipboard_location(write.location());
            // cap BEFORE clone: borrow (&mime,&data) refs (no data copy), sum decoded bytes.
            let parts: Vec<(&str, &str)> = write.contents().map(|c| (c.mime, c.data)).collect();
            let total: usize = parts.iter().map(|(_, d)| d.len()).sum();
            if total > MAX_OSC52_CLIPBOARD_BYTES {
                if let Some(insp) = inspect_for_clip.as_ref() {
                    insp.record_osc52_over_cap_drop();
                }
                return Ok(()); // OSC 52 ignores the result; drop with no owned allocation
            }
            let contents: Vec<ClipboardMimePart> = parts
                .into_iter()
                .map(|(mime, data)| ClipboardMimePart {
                    mime: mime.to_owned(),
                    data: data.to_owned(),
                })
                .collect();
            match clip_tx.try_send(EnginePresentationEvent::ClipboardWrite { location, contents }) {
                Ok(()) => {
                    if let Some(insp) = inspect_for_clip.as_ref() {
                        insp.record_osc52_forwarded();
                    }
                }
                Err(TrySendError::Full(_)) => {
                    if let Some(insp) = inspect_for_clip.as_ref() {
                        insp.record_presentation_channel_full_drop();
                    }
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
            if let Some(w) = waker_for_clip.as_ref()
                && let Ok(guard) = w.lock()
                && let Some(f) = guard.as_ref()
            {
                f();
            }
            Ok(())
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
            latest_title_slot,
        })
    }

    /// Take and clear the latest OSC title update (authoritative at drain).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "production drain uses EngineHandle::take_latest_title"
        )
    )]
    pub(crate) fn take_latest_title(&self) -> Option<TitleUpdate> {
        self.latest_title_slot
            .swap(None)
            .map(|update| (*update).clone())
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

    /// Encode paste bytes against the live bracketed-paste mode (mode 2004).
    pub(crate) fn encode_paste(&mut self, data: &[u8]) -> Result<Vec<u8>, EngineError> {
        use libghostty_vt::terminal::Mode;
        let bracketed = self.terminal.mode(Mode::BRACKETED_PASTE)?;
        let mut work = data.to_vec(); // paste::encode mutates in place
        let mut buf = vec![0u8; data.len() + 16]; // bracket wrapper is 12 bytes; strip/CR are 1:1
        let n = libghostty_vt::paste::encode(&mut work, bracketed, &mut buf)?;
        buf.truncate(n);
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
        let mut uri_intern: HashMap<Vec<u8>, Arc<str>> = HashMap::new();
        let mut row_iter = self.rows.update(&snapshot)?;
        let mut row_y: u32 = 0;
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

                let raw_cell = cell.raw_cell()?;
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
                let hyperlink_uri = if raw_cell.has_hyperlink().unwrap_or(false) {
                    read_hyperlink_uri(&self.terminal, this_col, row_y, &mut uri_intern)
                } else {
                    None
                };

                row_cells.push(FrameCell {
                    col: this_col,
                    grapheme,
                    fg,
                    bg,
                    wide: matches!(wide, CellWide::Wide),
                    selected,
                    style,
                    hyperlink_uri,
                });
            }
            grid.push(FrameRow { cells: row_cells });
            row_y += 1;
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

fn read_hyperlink_uri(
    terminal: &Terminal<'_, '_>,
    col: u16,
    row: u32,
    intern: &mut HashMap<Vec<u8>, Arc<str>>,
) -> Option<Arc<str>> {
    let grid_ref = terminal
        .grid_ref(Point::Viewport(PointCoordinate { x: col, y: row }))
        .ok()?;
    let mut buf = vec![0u8; 512];
    loop {
        match grid_ref.hyperlink_uri(&mut buf) {
            Ok(0) => return None,
            Ok(n) => {
                if n > MAX_HYPERLINK_URI_BYTES {
                    return None;
                }
                let bytes = &buf[..n];
                if let Some(existing) = intern.get(bytes) {
                    return Some(Arc::clone(existing));
                }
                let s = std::str::from_utf8(bytes).ok()?.to_owned();
                let arc: Arc<str> = Arc::from(s);
                intern.insert(bytes.to_vec(), Arc::clone(&arc));
                return Some(arc);
            }
            Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
                if required <= buf.len() || required > MAX_HYPERLINK_URI_BYTES {
                    return None;
                }
                buf.resize(required, 0);
            }
            Err(_) => return None,
        }
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

fn map_clipboard_location(loc: libghostty_vt::terminal::ClipboardLocation) -> ClipboardLocation {
    use libghostty_vt::terminal::ClipboardLocation as L;
    match loc {
        L::Standard => ClipboardLocation::Standard,
        L::Selection => ClipboardLocation::Selection,
        L::Primary => ClipboardLocation::Primary,
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
    fn osc8_hyperlink_populates_frame_cell_uri() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        // OSC 8 hyperlink open/close uses ST (`\x1b\\`) — BEL terminates early in libghostty.
        e.feed(b"\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\");
        let f = e.build_frame().unwrap();
        let cell = f.grid[0]
            .cells
            .iter()
            .find(|c| c.grapheme == "l")
            .expect("linked cell");
        assert_eq!(cell.hyperlink_uri.as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn osc8_closer_clears_subsequent_cells() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b]8;;https://example.com\x1b\\L\x1b]8;;\x1b\\X");
        let f = e.build_frame().unwrap();
        let l = f.grid[0].cells.iter().find(|c| c.grapheme == "L").unwrap();
        let x = f.grid[0].cells.iter().find(|c| c.grapheme == "X").unwrap();
        assert_eq!(l.hyperlink_uri.as_deref(), Some("https://example.com"));
        assert_eq!(x.hyperlink_uri, None);
    }

    #[test]
    fn osc2_title_is_sanitized_before_enqueue() {
        use std::time::Duration;

        use super::*;
        use crate::engine::presentation::{EnginePresentationEvent, PRESENTATION_CHANNEL_CAP};

        let (tx, rx) = crossbeam_channel::bounded(PRESENTATION_CHANNEL_CAP);
        let mut engine = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        // SOH (0x01) embeds a strippable C0 control; BEL (0x07) would terminate the OSC
        // sequence in libghostty before the title callback sees the full payload.
        engine.feed(b"\x1b]2;Hi\x01There\x1b\\");
        let ev = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(ev, EnginePresentationEvent::TitleChanged("HiThere".into()));
    }

    #[test]
    fn builds_frame_with_sgr_and_colors() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
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
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
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
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
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

    #[test]
    fn encode_paste_wraps_bracketed_when_mode_2004_enabled() {
        let cfg = EngineConfig {
            cols: 40,
            rows: 8,
            max_scrollback: 0,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
        engine.feed(b"\x1b[?2004h"); // enable bracketed paste
        let out = engine.encode_paste(b"ab").expect("encode");
        assert_eq!(out, b"\x1b[200~ab\x1b[201~");
    }

    #[test]
    fn encode_paste_plain_when_bracketed_disabled_and_strips_esc() {
        let cfg = EngineConfig {
            cols: 40,
            rows: 8,
            max_scrollback: 0,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
        let out = engine.encode_paste(b"a\x1bb").expect("encode"); // ESC stripped -> space
        assert_eq!(out, b"a b");
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use crate::engine::presentation::{
        ClipboardLocation, EnginePresentationEvent, MAX_OSC52_CLIPBOARD_BYTES,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    fn osc52(pc: &str, decoded: &[u8]) -> Vec<u8> {
        let mut v = Vec::from(format!("\x1b]52;{pc};").as_bytes());
        v.extend_from_slice(STANDARD.encode(decoded).as_bytes());
        v.push(0x07); // BEL terminator
        v
    }

    fn engine_with_rx() -> (
        VtEngine,
        crossbeam_channel::Receiver<EnginePresentationEvent>,
    ) {
        let (tx, rx) =
            crossbeam_channel::bounded(crate::engine::presentation::PRESENTATION_CHANNEL_CAP);
        let cfg = EngineConfig {
            cols: 40,
            rows: 8,
            max_scrollback: 0,
            cell_w_px: 8,
            cell_h_px: 16,
        };
        let engine = VtEngine::new(&cfg, |_| {}, tx).expect("engine");
        (engine, rx)
    }

    #[test]
    fn osc52_write_under_cap_emits_clipboard_event_with_location_and_data() {
        let (mut engine, rx) = engine_with_rx();
        engine.feed(&osc52("c", b"hello-copy"));
        match rx.try_recv().expect("clipboard event") {
            EnginePresentationEvent::ClipboardWrite { location, contents } => {
                assert_eq!(location, ClipboardLocation::Standard);
                assert_eq!(contents.len(), 1);
                assert_eq!(contents[0].data, "hello-copy");
            }
            other => panic!("expected ClipboardWrite, got {other:?}"),
        }
    }

    #[test]
    fn osc52_write_over_cap_drops_before_clone_no_event() {
        let (mut engine, rx) = engine_with_rx();
        let big = vec![b'x'; MAX_OSC52_CLIPBOARD_BYTES + 1];
        engine.feed(&osc52("c", &big));
        assert!(rx.try_recv().is_err(), "over-cap OSC 52 must emit no event");
    }

    #[test]
    fn osc52_write_cap_minus_one_emits() {
        let (mut engine, rx) = engine_with_rx();
        let below = vec![b'z'; MAX_OSC52_CLIPBOARD_BYTES - 1];
        engine.feed(&osc52("c", &below));
        assert!(matches!(
            rx.try_recv(),
            Ok(EnginePresentationEvent::ClipboardWrite { .. })
        ));
    }

    #[test]
    fn osc52_write_at_cap_emits() {
        let (mut engine, rx) = engine_with_rx();
        let at = vec![b'y'; MAX_OSC52_CLIPBOARD_BYTES];
        engine.feed(&osc52("c", &at));
        assert!(matches!(
            rx.try_recv(),
            Ok(EnginePresentationEvent::ClipboardWrite { .. })
        ));
    }

    #[test]
    fn osc52_read_query_emits_no_event() {
        let (mut engine, rx) = engine_with_rx();
        engine.feed(b"\x1b]52;c;?\x07"); // read request — binding never delivers reads
        assert!(
            rx.try_recv().is_err(),
            "OSC 52 read must not produce a host event"
        );
    }
}
