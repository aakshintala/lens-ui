//! GPUI physical-key strings → [`LensKey`], and pure Ghostty key encoding.

use libghostty_vt::error::Result;
use libghostty_vt::key::{Action, Encoder, Event, Key, Mods};

use super::command::{KeyAction, KeyInput, KeyMods, LensKey};

/// Whether a gpui keydown should enqueue via the special-key path (not plain printable text).
///
/// Unmodified printable keys are owned by [`gpui::EntityInputHandler::replace_text_in_range`]
/// so gpui does not double-emit `key_char` and keydown.
pub(crate) fn keydown_should_enqueue(key: &str, mods: &gpui::Modifiers) -> bool {
    if mods.control || mods.alt || mods.platform || mods.function {
        return true;
    }
    matches!(
        key,
        "up" | "down"
            | "left"
            | "right"
            | "pageup"
            | "pagedown"
            | "home"
            | "end"
            | "insert"
            | "enter"
            | "tab"
            | "escape"
            | "backspace"
            | "delete"
            | "f1"
            | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
            | "f13"
            | "f14"
            | "f15"
            | "f16"
            | "f17"
            | "f18"
            | "f19"
            | "f20"
            | "f21"
            | "f22"
            | "f23"
            | "f24"
            | "f25"
    )
}

/// Map gpui modifier state to engine [`KeyMods`].
pub(crate) fn gpui_mods_to_key_mods(mods: &gpui::Modifiers) -> KeyMods {
    KeyMods {
        shift: mods.shift,
        alt: mods.alt,
        ctrl: mods.control,
        super_key: mods.platform,
    }
}

/// Map a gpui [`Keystroke`](gpui::Keystroke) physical `key` string to [`LensKey`].
pub(crate) fn keystroke_to_lens(key: &str) -> LensKey {
    match key {
        "a" => LensKey::A,
        "b" => LensKey::B,
        "c" => LensKey::C,
        "d" => LensKey::D,
        "e" => LensKey::E,
        "f" => LensKey::F,
        "g" => LensKey::G,
        "h" => LensKey::H,
        "i" => LensKey::I,
        "j" => LensKey::J,
        "k" => LensKey::K,
        "l" => LensKey::L,
        "m" => LensKey::M,
        "n" => LensKey::N,
        "o" => LensKey::O,
        "p" => LensKey::P,
        "q" => LensKey::Q,
        "r" => LensKey::R,
        "s" => LensKey::S,
        "t" => LensKey::T,
        "u" => LensKey::U,
        "v" => LensKey::V,
        "w" => LensKey::W,
        "x" => LensKey::X,
        "y" => LensKey::Y,
        "z" => LensKey::Z,
        "0" => LensKey::Digit0,
        "1" => LensKey::Digit1,
        "2" => LensKey::Digit2,
        "3" => LensKey::Digit3,
        "4" => LensKey::Digit4,
        "5" => LensKey::Digit5,
        "6" => LensKey::Digit6,
        "7" => LensKey::Digit7,
        "8" => LensKey::Digit8,
        "9" => LensKey::Digit9,
        "space" => LensKey::Space,
        "tab" => LensKey::Tab,
        "enter" => LensKey::Enter,
        "backspace" => LensKey::Backspace,
        "escape" => LensKey::Escape,
        "up" => LensKey::ArrowUp,
        "down" => LensKey::ArrowDown,
        "left" => LensKey::ArrowLeft,
        "right" => LensKey::ArrowRight,
        "pageup" => LensKey::PageUp,
        "pagedown" => LensKey::PageDown,
        "home" => LensKey::Home,
        "end" => LensKey::End,
        "insert" => LensKey::Insert,
        "delete" => LensKey::Delete,
        "f1" => LensKey::F1,
        "f2" => LensKey::F2,
        "f3" => LensKey::F3,
        "f4" => LensKey::F4,
        "f5" => LensKey::F5,
        "f6" => LensKey::F6,
        "f7" => LensKey::F7,
        "f8" => LensKey::F8,
        "f9" => LensKey::F9,
        "f10" => LensKey::F10,
        "f11" => LensKey::F11,
        "f12" => LensKey::F12,
        "f13" => LensKey::F13,
        "f14" => LensKey::F14,
        "f15" => LensKey::F15,
        "f16" => LensKey::F16,
        "f17" => LensKey::F17,
        "f18" => LensKey::F18,
        "f19" => LensKey::F19,
        "f20" => LensKey::F20,
        "f21" => LensKey::F21,
        "f22" => LensKey::F22,
        "f23" => LensKey::F23,
        "f24" => LensKey::F24,
        "f25" => LensKey::F25,
        "shift" => LensKey::ShiftLeft,
        "control" => LensKey::ControlLeft,
        "alt" => LensKey::AltLeft,
        "platform" => LensKey::MetaLeft,
        // unreachable on macOS gpui (collapses keypad → "enter"/"0".."9"); retained for non-mac/synthetic events
        "numpad0" => LensKey::Numpad0,
        "numpad1" => LensKey::Numpad1,
        "numpad2" => LensKey::Numpad2,
        "numpad3" => LensKey::Numpad3,
        "numpad4" => LensKey::Numpad4,
        "numpad5" => LensKey::Numpad5,
        "numpad6" => LensKey::Numpad6,
        "numpad7" => LensKey::Numpad7,
        "numpad8" => LensKey::Numpad8,
        "numpad9" => LensKey::Numpad9,
        "numpadenter" => LensKey::NumpadEnter,
        "numpadadd" => LensKey::NumpadAdd,
        "numpadsubtract" => LensKey::NumpadSubtract,
        "numpadmultiply" => LensKey::NumpadMultiply,
        "numpaddivide" => LensKey::NumpadDivide,
        "numpaddecimal" => LensKey::NumpadDecimal,
        "numpadequal" => LensKey::NumpadEqual,
        "numpadcomma" => LensKey::NumpadComma,
        "`" => LensKey::Backquote,
        "-" => LensKey::Minus,
        "=" => LensKey::Equal,
        "[" => LensKey::BracketLeft,
        "]" => LensKey::BracketRight,
        ";" => LensKey::Semicolon,
        "'" => LensKey::Quote,
        "," => LensKey::Comma,
        "." => LensKey::Period,
        "/" => LensKey::Slash,
        "\\" => LensKey::Backslash,
        _ => LensKey::Unidentified,
    }
}

/// Apply a [`KeyInput`] onto a reusable Ghostty [`Event`].
pub(crate) fn apply_key_input_to_event(ev: &mut Event<'_>, input: &KeyInput) -> Result<()> {
    ev.set_action(input.action.into());
    ev.set_key(input.key.into());
    ev.set_mods(input.mods.into());
    ev.set_composing(input.composing);
    ev.set_utf8(input.utf8.clone());
    Ok(())
}

/// Encode `input` with a pre-configured encoder; composing → empty output.
///
/// Does not set macOS option-as-alt policy — the caller/terminal owns that via
/// [`Encoder::set_macos_option_as_alt`] (Task 2 `encode_key` / `set_options_from_terminal`).
pub(crate) fn encode_key_pure(
    enc: &mut Encoder<'_>,
    ev: &mut Event<'_>,
    input: &KeyInput,
    buf: &mut Vec<u8>,
) -> Result<()> {
    buf.clear();
    if input.composing {
        return Ok(());
    }
    apply_key_input_to_event(ev, input)?;
    enc.encode_to_vec(ev, buf)
}

impl From<KeyAction> for Action {
    fn from(action: KeyAction) -> Self {
        match action {
            KeyAction::Press => Action::Press,
            KeyAction::Release => Action::Release,
            KeyAction::Repeat => Action::Repeat,
        }
    }
}

impl From<LensKey> for Key {
    fn from(key: LensKey) -> Self {
        match key {
            LensKey::Unidentified => Key::Unidentified,
            LensKey::Backquote => Key::Backquote,
            LensKey::Backslash => Key::Backslash,
            LensKey::BracketLeft => Key::BracketLeft,
            LensKey::BracketRight => Key::BracketRight,
            LensKey::Comma => Key::Comma,
            LensKey::Digit0 => Key::Digit0,
            LensKey::Digit1 => Key::Digit1,
            LensKey::Digit2 => Key::Digit2,
            LensKey::Digit3 => Key::Digit3,
            LensKey::Digit4 => Key::Digit4,
            LensKey::Digit5 => Key::Digit5,
            LensKey::Digit6 => Key::Digit6,
            LensKey::Digit7 => Key::Digit7,
            LensKey::Digit8 => Key::Digit8,
            LensKey::Digit9 => Key::Digit9,
            LensKey::Equal => Key::Equal,
            LensKey::IntlBackslash => Key::IntlBackslash,
            LensKey::IntlRo => Key::IntlRo,
            LensKey::IntlYen => Key::IntlYen,
            LensKey::A => Key::A,
            LensKey::B => Key::B,
            LensKey::C => Key::C,
            LensKey::D => Key::D,
            LensKey::E => Key::E,
            LensKey::F => Key::F,
            LensKey::G => Key::G,
            LensKey::H => Key::H,
            LensKey::I => Key::I,
            LensKey::J => Key::J,
            LensKey::K => Key::K,
            LensKey::L => Key::L,
            LensKey::M => Key::M,
            LensKey::N => Key::N,
            LensKey::O => Key::O,
            LensKey::P => Key::P,
            LensKey::Q => Key::Q,
            LensKey::R => Key::R,
            LensKey::S => Key::S,
            LensKey::T => Key::T,
            LensKey::U => Key::U,
            LensKey::V => Key::V,
            LensKey::W => Key::W,
            LensKey::X => Key::X,
            LensKey::Y => Key::Y,
            LensKey::Z => Key::Z,
            LensKey::Minus => Key::Minus,
            LensKey::Period => Key::Period,
            LensKey::Quote => Key::Quote,
            LensKey::Semicolon => Key::Semicolon,
            LensKey::Slash => Key::Slash,
            LensKey::AltLeft => Key::AltLeft,
            LensKey::AltRight => Key::AltRight,
            LensKey::Backspace => Key::Backspace,
            LensKey::CapsLock => Key::CapsLock,
            LensKey::ContextMenu => Key::ContextMenu,
            LensKey::ControlLeft => Key::ControlLeft,
            LensKey::ControlRight => Key::ControlRight,
            LensKey::Enter => Key::Enter,
            LensKey::MetaLeft => Key::MetaLeft,
            LensKey::MetaRight => Key::MetaRight,
            LensKey::ShiftLeft => Key::ShiftLeft,
            LensKey::ShiftRight => Key::ShiftRight,
            LensKey::Space => Key::Space,
            LensKey::Tab => Key::Tab,
            LensKey::Convert => Key::Convert,
            LensKey::KanaMode => Key::KanaMode,
            LensKey::NonConvert => Key::NonConvert,
            LensKey::Delete => Key::Delete,
            LensKey::End => Key::End,
            LensKey::Help => Key::Help,
            LensKey::Home => Key::Home,
            LensKey::Insert => Key::Insert,
            LensKey::PageDown => Key::PageDown,
            LensKey::PageUp => Key::PageUp,
            LensKey::ArrowDown => Key::ArrowDown,
            LensKey::ArrowLeft => Key::ArrowLeft,
            LensKey::ArrowRight => Key::ArrowRight,
            LensKey::ArrowUp => Key::ArrowUp,
            LensKey::NumLock => Key::NumLock,
            LensKey::Numpad0 => Key::Numpad0,
            LensKey::Numpad1 => Key::Numpad1,
            LensKey::Numpad2 => Key::Numpad2,
            LensKey::Numpad3 => Key::Numpad3,
            LensKey::Numpad4 => Key::Numpad4,
            LensKey::Numpad5 => Key::Numpad5,
            LensKey::Numpad6 => Key::Numpad6,
            LensKey::Numpad7 => Key::Numpad7,
            LensKey::Numpad8 => Key::Numpad8,
            LensKey::Numpad9 => Key::Numpad9,
            LensKey::NumpadAdd => Key::NumpadAdd,
            LensKey::NumpadBackspace => Key::NumpadBackspace,
            LensKey::NumpadClear => Key::NumpadClear,
            LensKey::NumpadClearEntry => Key::NumpadClearEntry,
            LensKey::NumpadComma => Key::NumpadComma,
            LensKey::NumpadDecimal => Key::NumpadDecimal,
            LensKey::NumpadDivide => Key::NumpadDivide,
            LensKey::NumpadEnter => Key::NumpadEnter,
            LensKey::NumpadEqual => Key::NumpadEqual,
            LensKey::NumpadMemoryAdd => Key::NumpadMemoryAdd,
            LensKey::NumpadMemoryClear => Key::NumpadMemoryClear,
            LensKey::NumpadMemoryRecall => Key::NumpadMemoryRecall,
            LensKey::NumpadMemoryStore => Key::NumpadMemoryStore,
            LensKey::NumpadMemorySubtract => Key::NumpadMemorySubtract,
            LensKey::NumpadMultiply => Key::NumpadMultiply,
            LensKey::NumpadParenLeft => Key::NumpadParenLeft,
            LensKey::NumpadParenRight => Key::NumpadParenRight,
            LensKey::NumpadSubtract => Key::NumpadSubtract,
            LensKey::NumpadSeparator => Key::NumpadSeparator,
            LensKey::NumpadUp => Key::NumpadUp,
            LensKey::NumpadDown => Key::NumpadDown,
            LensKey::NumpadRight => Key::NumpadRight,
            LensKey::NumpadLeft => Key::NumpadLeft,
            LensKey::NumpadBegin => Key::NumpadBegin,
            LensKey::NumpadHome => Key::NumpadHome,
            LensKey::NumpadEnd => Key::NumpadEnd,
            LensKey::NumpadInsert => Key::NumpadInsert,
            LensKey::NumpadDelete => Key::NumpadDelete,
            LensKey::NumpadPageUp => Key::NumpadPageUp,
            LensKey::NumpadPageDown => Key::NumpadPageDown,
            LensKey::Escape => Key::Escape,
            LensKey::F1 => Key::F1,
            LensKey::F2 => Key::F2,
            LensKey::F3 => Key::F3,
            LensKey::F4 => Key::F4,
            LensKey::F5 => Key::F5,
            LensKey::F6 => Key::F6,
            LensKey::F7 => Key::F7,
            LensKey::F8 => Key::F8,
            LensKey::F9 => Key::F9,
            LensKey::F10 => Key::F10,
            LensKey::F11 => Key::F11,
            LensKey::F12 => Key::F12,
            LensKey::F13 => Key::F13,
            LensKey::F14 => Key::F14,
            LensKey::F15 => Key::F15,
            LensKey::F16 => Key::F16,
            LensKey::F17 => Key::F17,
            LensKey::F18 => Key::F18,
            LensKey::F19 => Key::F19,
            LensKey::F20 => Key::F20,
            LensKey::F21 => Key::F21,
            LensKey::F22 => Key::F22,
            LensKey::F23 => Key::F23,
            LensKey::F24 => Key::F24,
            LensKey::F25 => Key::F25,
            LensKey::Fn => Key::Fn,
            LensKey::FnLock => Key::FnLock,
            LensKey::PrintScreen => Key::PrintScreen,
            LensKey::ScrollLock => Key::ScrollLock,
            LensKey::Pause => Key::Pause,
            LensKey::BrowserBack => Key::BrowserBack,
            LensKey::BrowserFavorites => Key::BrowserFavorites,
            LensKey::BrowserForward => Key::BrowserForward,
            LensKey::BrowserHome => Key::BrowserHome,
            LensKey::BrowserRefresh => Key::BrowserRefresh,
            LensKey::BrowserSearch => Key::BrowserSearch,
            LensKey::BrowserStop => Key::BrowserStop,
            LensKey::Eject => Key::Eject,
            LensKey::LaunchApp1 => Key::LaunchApp1,
            LensKey::LaunchApp2 => Key::LaunchApp2,
            LensKey::LaunchMail => Key::LaunchMail,
            LensKey::MediaPlayPause => Key::MediaPlayPause,
            LensKey::MediaSelect => Key::MediaSelect,
            LensKey::MediaStop => Key::MediaStop,
            LensKey::MediaTrackNext => Key::MediaTrackNext,
            LensKey::MediaTrackPrevious => Key::MediaTrackPrevious,
            LensKey::Power => Key::Power,
            LensKey::Sleep => Key::Sleep,
            LensKey::AudioVolumeDown => Key::AudioVolumeDown,
            LensKey::AudioVolumeMute => Key::AudioVolumeMute,
            LensKey::AudioVolumeUp => Key::AudioVolumeUp,
            LensKey::WakeUp => Key::WakeUp,
            LensKey::Copy => Key::Copy,
            LensKey::Cut => Key::Cut,
            LensKey::Paste => Key::Paste,
        }
    }
}

impl From<KeyMods> for Mods {
    fn from(mods: KeyMods) -> Self {
        let mut out = Mods::empty();
        if mods.shift {
            out |= Mods::SHIFT;
        }
        if mods.alt {
            out |= Mods::ALT;
        }
        if mods.ctrl {
            out |= Mods::CTRL;
        }
        if mods.super_key {
            out |= Mods::SUPER;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use libghostty_vt::key::OptionAsAlt;
    use libghostty_vt::key::{Encoder, Event, KittyKeyFlags};

    fn base_arrow() -> KeyInput {
        KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: None,
        }
    }

    #[test]
    fn arrow_up_normal_mode_encodes_csi_a() {
        let mut enc = Encoder::new().unwrap();
        enc.set_cursor_key_application(false);
        let mut ev = Event::new().unwrap();
        let input = base_arrow();
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x1b[A");
    }

    #[test]
    fn ctrl_c_encodes_etx() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::C,
            mods: KeyMods {
                ctrl: true,
                ..KeyMods::default()
            },
            utf8: None, // physical key + mods — NOT utf8 "c"
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x03");
    }

    #[test]
    fn alt_printable_a_encodes_esc_prefixed() {
        let mut enc = Encoder::new().unwrap();
        enc.set_alt_esc_prefix(true);
        #[cfg(target_os = "macos")]
        enc.set_macos_option_as_alt(OptionAsAlt::True);
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods {
                alt: true,
                ..KeyMods::default()
            },
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x1ba");
    }

    #[test]
    fn keypad_enter_encodes_under_application_keypad() {
        let mut enc = Encoder::new().unwrap();
        enc.set_keypad_key_application(true);
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::NumpadEnter,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x1bOM");
    }

    #[test]
    fn kitty_flags_encode_release_and_repeat_distinctly() {
        let mut enc = Encoder::new().unwrap();
        enc.set_kitty_flags(KittyKeyFlags::DISAMBIGUATE | KittyKeyFlags::REPORT_EVENTS);
        let mut ev = Event::new().unwrap();
        let press_in = base_arrow();
        let mut press = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &press_in, &mut press).unwrap();
        let mut release = Vec::new();
        let rel = KeyInput {
            action: KeyAction::Release,
            ..press_in.clone_without_ack()
        };
        encode_key_pure(&mut enc, &mut ev, &rel, &mut release).unwrap();
        let mut repeat = Vec::new();
        let rep = KeyInput {
            action: KeyAction::Repeat,
            ..press_in.clone_without_ack()
        };
        encode_key_pure(&mut enc, &mut ev, &rep, &mut repeat).unwrap();
        assert_ne!(press, release, "Kitty release must differ from press");
        assert_ne!(press, repeat, "Kitty repeat must differ from press");
        assert_eq!(press, b"\x1b[1;1:1A");
        assert_eq!(release, b"\x1b[1;1:3A");
        assert_eq!(repeat, b"\x1b[1;1:2A");
    }

    #[test]
    fn keydown_should_enqueue_special_and_modified_only() {
        use gpui::Modifiers;

        assert!(keydown_should_enqueue("up", &Modifiers::default()));
        assert!(keydown_should_enqueue("enter", &Modifiers::default()));
        assert!(!keydown_should_enqueue("a", &Modifiers::default()));
        assert!(!keydown_should_enqueue("z", &Modifiers::default()));
        assert!(keydown_should_enqueue(
            "c",
            &Modifiers {
                control: true,
                ..Modifiers::default()
            }
        ));
        assert!(!keydown_should_enqueue(
            "a",
            &Modifiers {
                shift: true,
                ..Modifiers::default()
            }
        ));
        assert!(keydown_should_enqueue(
            "tab",
            &Modifiers {
                shift: true,
                ..Modifiers::default()
            }
        ));
    }

    #[test]
    fn keystroke_to_lens_maps_gpui_physical_strings() {
        assert_eq!(keystroke_to_lens("up"), LensKey::ArrowUp);
        assert_eq!(keystroke_to_lens("enter"), LensKey::Enter);
        assert_eq!(keystroke_to_lens("tab"), LensKey::Tab);
        assert_eq!(keystroke_to_lens("escape"), LensKey::Escape);
        assert_eq!(keystroke_to_lens("backspace"), LensKey::Backspace);
        assert_eq!(keystroke_to_lens("delete"), LensKey::Delete);
        assert_eq!(keystroke_to_lens("space"), LensKey::Space);
        assert_eq!(keystroke_to_lens("home"), LensKey::Home);
        assert_eq!(keystroke_to_lens("end"), LensKey::End);
        assert_eq!(keystroke_to_lens("a"), LensKey::A);
        assert_eq!(keystroke_to_lens("z"), LensKey::Z);
        assert_eq!(keystroke_to_lens("0"), LensKey::Digit0);
        assert_eq!(keystroke_to_lens("9"), LensKey::Digit9);
        assert_eq!(keystroke_to_lens("]"), LensKey::BracketRight);
        assert_eq!(keystroke_to_lens(";"), LensKey::Semicolon);
        assert_eq!(keystroke_to_lens("/"), LensKey::Slash);
        assert_eq!(keystroke_to_lens("\\"), LensKey::Backslash);
        assert_eq!(keystroke_to_lens("-"), LensKey::Minus);
        assert_eq!(keystroke_to_lens("="), LensKey::Equal);
        assert_eq!(keystroke_to_lens("f1"), LensKey::F1);
        assert_eq!(keystroke_to_lens("f25"), LensKey::F25);
        assert_eq!(keystroke_to_lens("f26"), LensKey::Unidentified);
    }

    #[test]
    fn keystroke_to_lens_enter_is_regular_not_numpad() {
        // macOS gpui collapses keypad Enter to "enter" (never "numpadenter").
        assert_eq!(keystroke_to_lens("enter"), LensKey::Enter);
    }

    #[test]
    fn ime_commit_utf8_field_encodes_text_bytes() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: KeyMods::default(),
            utf8: Some("你好".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, "你好".as_bytes());
    }

    #[test]
    fn composing_true_produces_no_pty_bytes() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: KeyMods::default(),
            utf8: Some("n".into()),
            composing: true,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert!(
            buf.is_empty(),
            "preedit must not emit PTY bytes; got {buf:?}"
        );
    }
}
