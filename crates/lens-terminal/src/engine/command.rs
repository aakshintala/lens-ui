//! Engine input command types for the Slice 2a input path.

use crossbeam_channel::Sender;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyAction {
    Press,
    Release,
    Repeat,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct KeyMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_key: bool,
}

/// Full physical key set mirrored from Ghostty's [`libghostty_vt::key::Key`].
#[allow(dead_code, reason = "variants exercised as the key map grows")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub(crate) enum LensKey {
    Unidentified,
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Equal,
    IntlBackslash,
    IntlRo,
    IntlYen,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,
    AltLeft,
    AltRight,
    Backspace,
    CapsLock,
    ContextMenu,
    ControlLeft,
    ControlRight,
    Enter,
    MetaLeft,
    MetaRight,
    ShiftLeft,
    ShiftRight,
    Space,
    Tab,
    Convert,
    KanaMode,
    NonConvert,
    Delete,
    End,
    Help,
    Home,
    Insert,
    PageDown,
    PageUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    NumLock,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadBackspace,
    NumpadClear,
    NumpadClearEntry,
    NumpadComma,
    NumpadDecimal,
    NumpadDivide,
    NumpadEnter,
    NumpadEqual,
    NumpadMemoryAdd,
    NumpadMemoryClear,
    NumpadMemoryRecall,
    NumpadMemoryStore,
    NumpadMemorySubtract,
    NumpadMultiply,
    NumpadParenLeft,
    NumpadParenRight,
    NumpadSubtract,
    NumpadSeparator,
    NumpadUp,
    NumpadDown,
    NumpadRight,
    NumpadLeft,
    NumpadBegin,
    NumpadHome,
    NumpadEnd,
    NumpadInsert,
    NumpadDelete,
    NumpadPageUp,
    NumpadPageDown,
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    Fn,
    FnLock,
    PrintScreen,
    ScrollLock,
    Pause,
    BrowserBack,
    BrowserFavorites,
    BrowserForward,
    BrowserHome,
    BrowserRefresh,
    BrowserSearch,
    BrowserStop,
    Eject,
    LaunchApp1,
    LaunchApp2,
    LaunchMail,
    MediaPlayPause,
    MediaSelect,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,
    Power,
    Sleep,
    AudioVolumeDown,
    AudioVolumeMute,
    AudioVolumeUp,
    WakeUp,
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Debug)]
pub(crate) struct KeyInput {
    pub action: KeyAction,
    pub key: LensKey,
    pub mods: KeyMods,
    pub utf8: Option<String>,
    pub composing: bool,
    pub access_epoch: u64,
    pub ack: Option<Sender<InputAck>>,
}

impl KeyInput {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "test helper for ack-barrier tests")
    )]
    pub(crate) fn clone_without_ack(&self) -> Self {
        Self {
            action: self.action,
            key: self.key,
            mods: self.mods,
            utf8: self.utf8.clone(),
            composing: self.composing,
            access_epoch: self.access_epoch,
            ack: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PasteInput {
    pub bytes: Vec<u8>,
    pub access_epoch: u64,
    pub ack: Option<Sender<InputAck>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputAck {
    pub encoded: Vec<u8>,
    pub accepted: bool,
}

#[allow(
    dead_code,
    reason = "Top/Bottom for programmatic scroll; UI wheel maps to Lines"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScrollDelta {
    Lines(i32),
    Top,
    Bottom,
}

#[allow(dead_code, reason = "consumed in Task 3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MouseTracking {
    None,
    X10,
    Normal,
    Button,
    Any,
}

#[allow(dead_code, reason = "consumed in Task 3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MouseFormat {
    X10,
    Utf8,
    Sgr,
    Urxvt,
    SgrPixels,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MouseButtonKind {
    Left,
    Right,
    Middle,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MouseEventKind {
    Down,
    Move,
    Up,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MouseReportPolicy {
    #[default]
    Auto,
    ForceLocal,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GestureOwner {
    Report,
    Select,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GestureDisposition {
    Reported,
    Selected,
    LocalClick,
    Suppressed,
    Ignored,
    Coalesced,
    ScrolledLocal,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MouseAck {
    pub encoded: Vec<u8>,
    pub disposition: GestureDisposition,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Debug)]
pub(crate) struct MouseGesture {
    pub kind: MouseEventKind,
    pub button: Option<MouseButtonKind>, // None only for buttonless Move
    pub mods: KeyMods,                   // includes shift
    pub cell: Option<(u16, u16)>,        // None when outside the grid (up-out / move-out)
    pub px_x: f32,
    pub px_y: f32,      // surface-relative pixels
    pub time: Duration, // monotonic (for set_time multi-click)
    pub mouse_local: bool,
    pub policy: MouseReportPolicy,
    /// Foreground-minted click token (nonzero on a Left Down). Stored in the engine's Select
    /// latch and echoed on `LocalClick` so the foreground resolves the hyperlink against the
    /// frame captured at THIS click's down, correlating overlapping clicks (codex F2).
    pub click_seq: u64,
    pub access_epoch: u64,             // stamped by enqueue_input
    pub ack: Option<Sender<MouseAck>>, // tests; None in production
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Debug)]
pub(crate) struct WheelInput {
    pub lines: i32, // signed, from gpui_scroll_to_lens
    pub cell: Option<(u16, u16)>,
    pub px_x: f32,
    pub px_y: f32,
    pub mods: KeyMods,
    pub access_epoch: u64,
    pub ack: Option<Sender<MouseAck>>,
}

/// Engine-internal adapter used by `encode_mouse_report` (built inside the worker arm from a
/// `MouseGesture`/`WheelInput`).
#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct MouseReportEv {
    pub action: MouseEventKind,
    pub button: Option<MouseButtonKind>,
    pub wheel: Option<bool /*up*/>,
    pub mods: KeyMods,
    pub px_x: f32,
    pub px_y: f32,
    pub any_button_pressed: bool,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
#[derive(Clone, Debug)]
pub struct CopyResult {
    pub text: Option<String>,
}

#[allow(dead_code, reason = "consumed in Task 2/3/4")]
pub(crate) type CopyResponder = crossbeam_channel::Sender<CopyResult>;
