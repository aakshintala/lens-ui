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

use super::command::{
    KeyInput, KeyMods, MouseButtonKind, MouseEventKind, MouseFormat, MouseReportEv, MouseTracking,
    ScrollDelta,
};
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
    row_iter: RowIterator<'static>,
    cells: CellIterator<'static>,
    key_encoder: libghostty_vt::key::Encoder<'static>,
    key_event: libghostty_vt::key::Event<'static>,
    cols: u16,
    rows: u16,
    cell_w_px: u32,
    cell_h_px: u32,
    mouse_encoder: libghostty_vt::mouse::Encoder<'static>,
    mouse_event: libghostty_vt::mouse::Event<'static>,
    selection_gesture: libghostty_vt::selection::gesture::Gesture<'static>,
    press_event: libghostty_vt::selection::gesture::PressEvent<'static>,
    drag_event: libghostty_vt::selection::gesture::DragEvent<'static>,
    release_event: libghostty_vt::selection::gesture::ReleaseEvent<'static>,
    applied_mouse_opts: Option<(MouseTracking, MouseFormat)>,
    /// Coalesce dedup identity of the last report event. When tracking, format, OR
    /// mods change vs. the previous report, the encoder's last-cell dedup is reset so a
    /// same-cell motion cannot inherit stale dedup across a mode/modifier transition
    /// (Rev-2 I11/I15): e.g. Sgr→SgrPixels→Sgr same-cell must re-emit, and a
    /// plain-Move→Ctrl-Move same-cell must re-emit.
    mouse_coalesce_key: Option<(MouseTracking, MouseFormat, KeyMods)>,
    applied_encoder_size: Option<(u32, u32, u32, u32)>,
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
            row_iter: RowIterator::new()?,
            cells: CellIterator::new()?,
            key_encoder: libghostty_vt::key::Encoder::new()?,
            key_event: libghostty_vt::key::Event::new()?,
            cols: cfg.cols,
            rows: cfg.rows,
            cell_w_px: cfg.cell_w_px,
            cell_h_px: cfg.cell_h_px,
            mouse_encoder: libghostty_vt::mouse::Encoder::new()?,
            mouse_event: libghostty_vt::mouse::Event::new()?,
            selection_gesture: libghostty_vt::selection::gesture::Gesture::new()?,
            press_event: libghostty_vt::selection::gesture::PressEvent::new()?,
            drag_event: libghostty_vt::selection::gesture::DragEvent::new()?,
            release_event: libghostty_vt::selection::gesture::ReleaseEvent::new()?,
            applied_mouse_opts: None,
            mouse_coalesce_key: None,
            applied_encoder_size: None,
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

    /// The emulator's total retained row count (scrollback + active viewport).
    /// `None` on FFI error so the caller can skip the sample rather than record a
    /// spurious 0 (which would make a live tab look empty to fleet accounting).
    /// NOTE: this is the *active screen*'s total — a program on the alternate
    /// screen (vim/less) reports ~viewport rows even though primary scrollback is
    /// still retained. That under-count is a documented estimate limitation
    /// (memory `terminal-max-scrollback-bytes-and-worker-stack`); a primary+alt
    /// sum needs a vendored accessor that does not exist today.
    pub(crate) fn total_rows(&self) -> Option<usize> {
        self.terminal.total_rows().ok()
    }

    /// Drain bytes accumulated by `on_pty_write` since the last drain.
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.reply_buffer.borrow_mut().drain(..).collect()
    }

    /// Resize the grid; reflows content when wraparound is enabled.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), EngineError> {
        self.terminal
            .resize(cols, rows, self.cell_w_px, self.cell_h_px)?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    #[allow(dead_code, reason = "consumed in Task 3 worker arbitration")]
    pub(crate) fn read_live_tracking(&self) -> MouseTracking {
        use libghostty_vt::terminal::Mode;
        if self.terminal.mode(Mode::ANY_MOUSE).unwrap_or(false) {
            return MouseTracking::Any;
        }
        if self.terminal.mode(Mode::BUTTON_MOUSE).unwrap_or(false) {
            return MouseTracking::Button;
        }
        if self.terminal.mode(Mode::NORMAL_MOUSE).unwrap_or(false) {
            return MouseTracking::Normal;
        }
        if self.terminal.mode(Mode::X10_MOUSE).unwrap_or(false) {
            return MouseTracking::X10;
        }
        MouseTracking::None
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn read_live_format(&self) -> MouseFormat {
        use libghostty_vt::terminal::Mode;
        if self.terminal.mode(Mode::SGR_PIXELS_MOUSE).unwrap_or(false) {
            return MouseFormat::SgrPixels;
        }
        if self.terminal.mode(Mode::SGR_MOUSE).unwrap_or(false) {
            return MouseFormat::Sgr;
        }
        if self.terminal.mode(Mode::URXVT_MOUSE).unwrap_or(false) {
            return MouseFormat::Urxvt;
        }
        if self.terminal.mode(Mode::UTF8_MOUSE).unwrap_or(false) {
            return MouseFormat::Utf8;
        }
        MouseFormat::X10
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    /// Invalidate the mouse motion-dedup scope: clears the encoder's last-cell dedup and
    /// the coalesce key so the next report re-emits even at a previously-seen cell. Called
    /// by the worker on a new report gesture and when tracking turns off (codex F6).
    pub(crate) fn reset_mouse_coalesce(&mut self) {
        self.mouse_encoder.reset();
        self.mouse_coalesce_key = None;
    }

    pub(crate) fn encode_mouse_report(
        &mut self,
        ev: &MouseReportEv,
    ) -> Result<Vec<u8>, EngineError> {
        use libghostty_vt::key::Mods;
        use libghostty_vt::mouse::{Action, Button, EncoderSize, Position};

        let tracking = self.read_live_tracking();
        let format = self.read_live_format();
        let opts = (tracking, format);
        if self.applied_mouse_opts != Some(opts) {
            self.mouse_encoder.set_options_from_terminal(&self.terminal);
            self.applied_mouse_opts = Some(opts);
        }
        // Reset the encoder's last-cell dedup on ANY tracking/format/mods transition so a
        // same-cell motion after the transition re-emits instead of coalescing against a
        // stale cell (Rev-2 I11/I15). Press/Release never coalesce, so an incidental reset
        // on those is harmless.
        let coalesce_key = (tracking, format, ev.mods);
        if self.mouse_coalesce_key != Some(coalesce_key) {
            self.mouse_encoder.reset();
            self.mouse_coalesce_key = Some(coalesce_key);
        }
        let size_tuple = (
            self.cell_w_px.saturating_mul(u32::from(self.cols)),
            self.cell_h_px.saturating_mul(u32::from(self.rows)),
            self.cell_w_px,
            self.cell_h_px,
        );
        if self.applied_encoder_size != Some(size_tuple) {
            self.mouse_encoder.set_size(EncoderSize {
                screen_width: size_tuple.0,
                screen_height: size_tuple.1,
                cell_width: size_tuple.2,
                cell_height: size_tuple.3,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            });
            self.applied_encoder_size = Some(size_tuple);
        }
        self.mouse_encoder
            .set_any_button_pressed(ev.any_button_pressed);
        self.mouse_encoder
            .set_track_last_cell(format != MouseFormat::SgrPixels);
        let action = match ev.action {
            MouseEventKind::Down => Action::Press,
            MouseEventKind::Up => Action::Release,
            MouseEventKind::Move => Action::Motion,
        };
        let button = match ev.wheel {
            Some(true) => Some(Button::Four),
            Some(false) => Some(Button::Five),
            None => ev.button.map(|b| match b {
                MouseButtonKind::Left => Button::Left,
                MouseButtonKind::Middle => Button::Middle,
                MouseButtonKind::Right => Button::Right,
            }),
        };
        self.mouse_event.set_action(action);
        self.mouse_event.set_button(button);
        self.mouse_event.set_mods(Mods::from(ev.mods));
        self.mouse_event.set_position(Position {
            x: ev.px_x,
            y: ev.px_y,
        });
        let mut out = Vec::new();
        self.mouse_encoder
            .encode_to_vec(&self.mouse_event, &mut out)?;
        Ok(out)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn apply_selection_press(
        &mut self,
        col: u16,
        row: u16,
        px_x: f32,
        px_y: f32,
        time: std::time::Duration,
    ) -> Result<bool, EngineError> {
        use libghostty_vt::selection::gesture::{Behavior, Behaviors};
        use libghostty_vt::terminal::{Point, PointCoordinate};

        let Self {
            terminal,
            press_event,
            selection_gesture,
            cell_w_px,
            ..
        } = self;
        let gref = terminal.grid_ref(Point::Viewport(PointCoordinate {
            x: col,
            y: u32::from(row),
        }))?;
        let behaviors = Behaviors::new()
            .with_single_click_behavior(Behavior::Cell)
            .with_double_click_behavior(Behavior::Word)
            .with_triple_click_behavior(Behavior::Line);
        press_event.set_behaviors(&behaviors)?;
        press_event.set_repeat_interval(std::time::Duration::from_millis(500))?;
        press_event.set_repeat_distance(f64::from(*cell_w_px))?;
        press_event.set_position(f64::from(px_x), f64::from(px_y))?;
        press_event.set_time(time)?;
        let sel = press_event.apply(selection_gesture, terminal, gref)?;
        terminal.set_selection(sel.as_ref())?;
        Ok(true)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn apply_selection_drag(
        &mut self,
        col: u16,
        row: u16,
        px_x: f32,
        px_y: f32,
    ) -> Result<bool, EngineError> {
        use libghostty_vt::selection::gesture::Geometry;
        use libghostty_vt::terminal::{Point, PointCoordinate};

        let (cols, rows, cw, ch) = (self.cols, self.rows, self.cell_w_px, self.cell_h_px);
        let Self {
            terminal,
            drag_event,
            selection_gesture,
            ..
        } = self;
        let gref = terminal.grid_ref(Point::Viewport(PointCoordinate {
            x: col,
            y: u32::from(row),
        }))?;
        let geom = Geometry {
            columns: u32::from(cols).max(1),
            cell_width: cw.max(1),
            padding_left: 0,
            screen_height: ch.saturating_mul(u32::from(rows)).max(1),
        };
        drag_event.set_position(f64::from(px_x), f64::from(px_y))?;
        let sel = drag_event.apply(selection_gesture, terminal, gref, geom)?;
        terminal.set_selection(sel.as_ref())?;
        Ok(true)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn apply_selection_release(
        &mut self,
        cell: Option<(u16, u16)>,
    ) -> Result<(), EngineError> {
        use libghostty_vt::terminal::{Point, PointCoordinate};

        let Self {
            terminal,
            release_event,
            selection_gesture,
            ..
        } = self;
        let gref = match cell {
            Some((c, r)) => Some(terminal.grid_ref(Point::Viewport(PointCoordinate {
                x: c,
                y: u32::from(r),
            }))?),
            None => None,
        };
        release_event.apply(selection_gesture, terminal, gref)?;
        Ok(())
    }

    #[allow(dead_code, reason = "consumed in Task 3 LocalClick classification")]
    pub(crate) fn gesture_dragged(&self) -> bool {
        self.selection_gesture
            .dragged(&self.terminal)
            .unwrap_or(false)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn select_all(&mut self) -> Result<bool, EngineError> {
        let sel = self.terminal.select_all()?;
        self.terminal.set_selection(sel.as_ref())?;
        Ok(true)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn clear_selection(&mut self) -> Result<bool, EngineError> {
        self.terminal.set_selection(None)?;
        Ok(true)
    }

    #[allow(dead_code, reason = "consumed in Task 3/4")]
    pub(crate) fn extract_selection_text(&self) -> Option<String> {
        use libghostty_vt::selection::FormatOptions;
        let opts = FormatOptions::default().with_unwrap(true).with_trim(true);
        match self.terminal.format_selection_alloc(None, opts) {
            Ok(Some(bytes)) => Some(String::from_utf8_lossy(bytes.as_ref()).into_owned()),
            _ => None,
        }
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
        let mut row_iter = self.row_iter.update(&snapshot)?;
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

    #[test]
    fn encode_mouse_report_coalesces_same_cell_motion() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b[?1003h\x1b[?1006h");
        let ev = MouseReportEv {
            action: MouseEventKind::Move,
            button: None,
            wheel: None,
            mods: super::super::command::KeyMods::default(),
            px_x: 16.0,
            px_y: 0.0,
            any_button_pressed: false,
        };
        let first = e.encode_mouse_report(&ev).expect("first");
        assert!(!first.is_empty(), "first same-cell motion must emit");
        let second = e.encode_mouse_report(&ev).expect("second");
        assert!(second.is_empty(), "second same-cell motion must coalesce");
    }

    #[test]
    fn encode_mouse_report_sgr_pixels_does_not_coalesce_motion() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b[?1003h\x1b[?1006h\x1b[?1016h");
        let ev = MouseReportEv {
            action: MouseEventKind::Move,
            button: None,
            wheel: None,
            mods: super::super::command::KeyMods::default(),
            px_x: 16.0,
            px_y: 0.0,
            any_button_pressed: false,
        };
        let first = e.encode_mouse_report(&ev).expect("first");
        assert!(!first.is_empty());
        let second = e.encode_mouse_report(&ev).expect("second");
        assert!(!second.is_empty(), "SgrPixels must emit on every motion");
    }

    #[test]
    fn encode_mouse_report_mods_change_resets_coalesce() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b[?1003h\x1b[?1006h"); // Any tracking + SGR
        let ev = |mods| MouseReportEv {
            action: MouseEventKind::Move,
            button: None,
            wheel: None,
            mods,
            px_x: 16.0,
            px_y: 0.0,
            any_button_pressed: false,
        };
        let none = KeyMods::default();
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::default()
        };
        assert!(
            !e.encode_mouse_report(&ev(none)).unwrap().is_empty(),
            "first move emits"
        );
        assert!(
            e.encode_mouse_report(&ev(none)).unwrap().is_empty(),
            "same-cell same-mods motion coalesces"
        );
        assert!(
            !e.encode_mouse_report(&ev(ctrl)).unwrap().is_empty(),
            "same-cell mods change must re-emit (coalesce dedup reset)"
        );
    }

    #[test]
    fn encode_mouse_report_format_transition_resets_coalesce() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b[?1003h\x1b[?1006h"); // Any tracking + SGR
        let ev = MouseReportEv {
            action: MouseEventKind::Move,
            button: None,
            wheel: None,
            mods: KeyMods::default(),
            px_x: 16.0,
            px_y: 0.0,
            any_button_pressed: false,
        };
        assert!(
            !e.encode_mouse_report(&ev).unwrap().is_empty(),
            "first SGR move emits"
        );
        assert!(
            e.encode_mouse_report(&ev).unwrap().is_empty(),
            "second SGR same-cell motion coalesces"
        );
        e.feed(b"\x1b[?1016h"); // switch to SgrPixels
        assert!(
            !e.encode_mouse_report(&ev).unwrap().is_empty(),
            "SgrPixels move emits"
        );
        e.feed(b"\x1b[?1016l"); // back to SGR
        assert!(
            !e.encode_mouse_report(&ev).unwrap().is_empty(),
            "same-cell SGR move after a format round-trip must re-emit (coalesce dedup reset)"
        );
    }

    #[test]
    fn encode_mouse_report_sgr_press_left() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"\x1b[?1000h\x1b[?1006h");
        let bytes = e
            .encode_mouse_report(&MouseReportEv {
                action: MouseEventKind::Down,
                button: Some(MouseButtonKind::Left),
                wheel: None,
                mods: super::super::command::KeyMods::default(),
                px_x: 0.0,
                px_y: 0.0,
                any_button_pressed: true,
            })
            .expect("encode");
        assert_eq!(bytes, b"\x1b[<0;1;1M");
    }

    #[test]
    fn selection_press_drag_release_marks_and_extracts() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"copyme");
        assert!(
            e.apply_selection_press(0, 0, 0.0, 0.0, std::time::Duration::ZERO)
                .expect("press")
        );
        assert!(e.apply_selection_drag(3, 0, 31.0, 0.0).expect("drag"));
        e.apply_selection_release(Some((3, 0))).expect("release");
        assert_eq!(e.extract_selection_text().as_deref(), Some("copy"));
        let cols: Vec<u16> = e.build_frame().expect("f").grid[0]
            .cells
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.col)
            .collect();
        assert!(cols.contains(&0) && cols.contains(&3), "got {cols:?}");
        assert!(e.clear_selection().expect("clear"));
        assert_eq!(e.extract_selection_text(), None);
        assert!(
            e.build_frame().expect("f").grid[0]
                .cells
                .iter()
                .all(|c| !c.selected)
        );
    }

    #[test]
    fn select_all_extract_and_double_click_word() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"foo bar");
        assert!(e.select_all().expect("all"));
        assert_eq!(
            e.extract_selection_text().as_deref().map(str::trim),
            Some("foo bar")
        );
        e.clear_selection().expect("clear");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(0))
            .expect("p1");
        e.apply_selection_release(Some((4, 0))).expect("r1");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(120))
            .expect("p2");
        assert_eq!(e.extract_selection_text().as_deref(), Some("bar"));
    }

    #[test]
    fn double_click_word_respects_repeat_interval() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
        e.feed(b"foo bar");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(0))
            .expect("p1");
        e.apply_selection_release(Some((4, 0))).expect("r1");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(600))
            .expect("p2");
        assert_ne!(
            e.extract_selection_text().as_deref(),
            Some("bar"),
            "gap >500ms must not widen to word"
        );

        e.clear_selection().expect("clear");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(0))
            .expect("p1_fast");
        e.apply_selection_release(Some((4, 0))).expect("r1_fast");
        e.apply_selection_press(4, 0, 32.0, 0.0, std::time::Duration::from_millis(120))
            .expect("p2_fast");
        assert_eq!(
            e.extract_selection_text().as_deref(),
            Some("bar"),
            "gap <=500ms still widens to word"
        );
    }

    #[test]
    fn total_rows_grows_past_viewport_after_scrollback() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap(); // 20x3, scrollback 100
        for i in 0..50 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        let rows = e.total_rows().expect("total_rows FFI");
        assert!(
            rows > 3,
            "total_rows must exceed the 3-row viewport once scrollback fills, got {rows}"
        );
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
