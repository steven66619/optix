/// Keyboard input handling: keybinding resolution and terminal key encoding.
use std::collections::HashMap;

use alacritty_terminal::term::TermMode;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

/// Modifier state relevant for keybindings.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_: bool,
}

impl Mods {
    pub fn from_state(state: ModifiersState) -> Self {
        Self {
            ctrl: state.control_key(),
            shift: state.shift_key(),
            alt: state.alt_key(),
            super_: state.super_key(),
        }
    }

    pub fn is_empty(self) -> bool {
        !self.ctrl && !self.shift && !self.alt && !self.super_
    }
}

/// A key binding lookup result.
pub enum BindingResult {
    /// The key combination resolved to a configured action.
    Action(String),
    /// No binding matched; the key should be forwarded to the terminal.
    Passthrough,
}

/// Logical key name used in binding strings.
fn logical_name(key: &Key) -> Option<String> {
    match key {
        Key::Named(named) => Some(match named {
            NamedKey::ArrowUp => "up".into(),
            NamedKey::ArrowDown => "down".into(),
            NamedKey::ArrowLeft => "left".into(),
            NamedKey::ArrowRight => "right".into(),
            NamedKey::PageUp => "pageup".into(),
            NamedKey::PageDown => "pagedown".into(),
            NamedKey::Home => "home".into(),
            NamedKey::End => "end".into(),
            NamedKey::Enter => "enter".into(),
            NamedKey::Tab => "tab".into(),
            NamedKey::Escape => "escape".into(),
            NamedKey::Backspace => "backspace".into(),
            NamedKey::Delete => "delete".into(),
            NamedKey::Insert => "insert".into(),
            NamedKey::Space => "space".into(),
            NamedKey::F1 => "f1".into(),
            NamedKey::F2 => "f2".into(),
            NamedKey::F3 => "f3".into(),
            NamedKey::F4 => "f4".into(),
            NamedKey::F5 => "f5".into(),
            NamedKey::F6 => "f6".into(),
            NamedKey::F7 => "f7".into(),
            NamedKey::F8 => "f8".into(),
            NamedKey::F9 => "f9".into(),
            NamedKey::F10 => "f10".into(),
            NamedKey::F11 => "f11".into(),
            NamedKey::F12 => "f12".into(),
            _ => format!("{named:?}").to_lowercase(),
        }),
        Key::Character(ch) => {
            let ch = ch.as_str();
            match ch {
                " " => Some("space".into()),
                "+" => Some("plus".into()),
                "-" => Some("minus".into()),
                "=" => Some("equal".into()),
                "[" => Some("[".into()),
                "]" => Some("]".into()),
                _ => Some(ch.to_lowercase()),
            }
        },
        Key::Unidentified(_) => None,
        _ => None,
    }
}

/// Physical key name used for shifted-symbol disambiguation (e.g. `1` for `!`).
fn physical_name(key: &Key, physical: PhysicalKey) -> Option<String> {
    let code = match physical {
        PhysicalKey::Code(code) => code,
        PhysicalKey::Unidentified(_) => {
            // Fall back to the logical key when no physical code is available.
            return logical_name(key);
        },
    };
    Some(match code {
        KeyCode::KeyA => "a".to_string(),
        KeyCode::KeyB => "b".to_string(),
        KeyCode::KeyC => "c".to_string(),
        KeyCode::KeyD => "d".to_string(),
        KeyCode::KeyE => "e".to_string(),
        KeyCode::KeyF => "f".to_string(),
        KeyCode::KeyG => "g".to_string(),
        KeyCode::KeyH => "h".to_string(),
        KeyCode::KeyI => "i".to_string(),
        KeyCode::KeyJ => "j".to_string(),
        KeyCode::KeyK => "k".to_string(),
        KeyCode::KeyL => "l".to_string(),
        KeyCode::KeyM => "m".to_string(),
        KeyCode::KeyN => "n".to_string(),
        KeyCode::KeyO => "o".to_string(),
        KeyCode::KeyP => "p".to_string(),
        KeyCode::KeyQ => "q".to_string(),
        KeyCode::KeyR => "r".to_string(),
        KeyCode::KeyS => "s".to_string(),
        KeyCode::KeyT => "t".to_string(),
        KeyCode::KeyU => "u".to_string(),
        KeyCode::KeyV => "v".to_string(),
        KeyCode::KeyW => "w".to_string(),
        KeyCode::KeyX => "x".to_string(),
        KeyCode::KeyY => "y".to_string(),
        KeyCode::KeyZ => "z".to_string(),
        KeyCode::Digit1 => "1".to_string(),
        KeyCode::Digit2 => "2".to_string(),
        KeyCode::Digit3 => "3".to_string(),
        KeyCode::Digit4 => "4".to_string(),
        KeyCode::Digit5 => "5".to_string(),
        KeyCode::Digit6 => "6".to_string(),
        KeyCode::Digit7 => "7".to_string(),
        KeyCode::Digit8 => "8".to_string(),
        KeyCode::Digit9 => "9".to_string(),
        KeyCode::Digit0 => "0".to_string(),
        KeyCode::ArrowUp => "up".to_string(),
        KeyCode::ArrowDown => "down".to_string(),
        KeyCode::ArrowLeft => "left".to_string(),
        KeyCode::ArrowRight => "right".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Escape => "escape".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::Space => "space".to_string(),
        KeyCode::Equal => "equal".to_string(),
        KeyCode::Minus => "minus".to_string(),
        KeyCode::BracketRight => "]".to_string(),
        KeyCode::BracketLeft => "[".to_string(),
        KeyCode::F1 => "f1".to_string(),
        KeyCode::F2 => "f2".to_string(),
        KeyCode::F3 => "f3".to_string(),
        KeyCode::F4 => "f4".to_string(),
        KeyCode::F5 => "f5".to_string(),
        KeyCode::F6 => "f6".to_string(),
        KeyCode::F7 => "f7".to_string(),
        KeyCode::F8 => "f8".to_string(),
        KeyCode::F9 => "f9".to_string(),
        KeyCode::F10 => "f10".to_string(),
        KeyCode::F11 => "f11".to_string(),
        KeyCode::F12 => "f12".to_string(),
        _ => format!("{code:?}").to_lowercase(),
    })
}

/// Characters whose typing already requires Shift to be held.
fn is_shifted_symbol(ch: &str) -> bool {
    matches!(
        ch,
        "!" | "@" | "#" | "$" | "%" | "^" | "&" | "*" | "(" | ")"
            | "_" | "+" | "{" | "}" | ":" | "\"" | "|" | "<" | ">" | "?"
            | "~"
    )
}

fn mods_string(mods: Mods) -> Vec<&'static str> {
    let mut out = Vec::new();
    if mods.ctrl {
        out.push("ctrl");
    }
    if mods.alt {
        out.push("alt");
    }
    if mods.super_ {
        out.push("super");
    }
    if mods.shift {
        out.push("shift");
    }
    out
}

/// Build the candidate binding strings for a key event, from most to least specific.
fn binding_candidates(event: &KeyEvent, mods: Mods, logical: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    let join = |mods: Mods, name: &str| {
        let mut parts = mods_string(mods);
        parts.push(name);
        parts.join("+")
    };

    // 1. Modifiers + logical name.
    candidates.push(join(mods, logical));

    // 2. Shift is absorbed into shifted symbols (ctrl+shift+= behaves as ctrl+plus).
    let key_char = match &event.logical_key {
        Key::Character(ch) => Some(ch.as_str()),
        _ => None,
    };
    if mods.shift && key_char.map(is_shifted_symbol).unwrap_or(false) {
        let mut no_shift = mods;
        no_shift.shift = false;
        candidates.push(join(no_shift, logical));
    }

    // 3. Physical key name (resolves shift+1 to `1`, etc.).
    if let Some(name) = physical_name(&event.logical_key, event.physical_key) {
        if name != logical {
            candidates.push(join(mods, &name));
            if mods.shift {
                // Also try absorbing shift into symbols typed via a shifted physical key.
                if matches!(name.as_str(), "equal" | "minus" | "bracketright" | "bracketleft") {
                    let mut no_shift = mods;
                    no_shift.shift = false;
                    candidates.push(join(no_shift, &name));
                }
            }
        }
    }

    candidates
}

/// Resolve a key event against the configured bindings.
pub fn resolve(event: &KeyEvent, mods: Mods, bindings: &HashMap<String, String>) -> BindingResult {
    let Some(logical) = logical_name(&event.logical_key) else {
        return BindingResult::Passthrough;
    };

    for candidate in binding_candidates(event, mods, &logical) {
        if let Some(action) = bindings.get(&candidate) {
            return BindingResult::Action(action.clone());
        }
    }
    BindingResult::Passthrough
}

/// Convert a key event into bytes for the terminal.
///
/// `mode` is the focused pane's current [`TermMode`]. It selects between the
/// kitty keyboard protocol (CSI `u` sequences) and classic encodings, and
/// drives DECCKM/DECKPAM behavior. Returns an empty vec when the key produces
/// no output.
pub fn encode_key(event: &KeyEvent, mods: Mods, mode: TermMode) -> Vec<u8> {
    let kitty_seq = kitty_active(mode);

    // Released keys are only reported under the kitty protocol's
    // `report_event_types` mode; otherwise a release produces no bytes.
    if event.state != ElementState::Pressed {
        if kitty_seq && mode.contains(TermMode::REPORT_EVENT_TYPES) {
            return encode_released(event, mods, mode);
        }
        return Vec::new();
    }

    if kitty_seq {
        encode_pressed_kitty(event, mods, mode)
    } else {
        encode_classic(event, mods, mode)
    }
}

/// Classic (pre-kitty) key encoding: control characters, ESC-prefixed Alt,
/// and DEC-style sequences for named keys.
fn encode_classic(event: &KeyEvent, mods: Mods, mode: TermMode) -> Vec<u8> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // Named keys with dedicated sequences.
    if let Key::Named(named) = &event.logical_key {
        if let Some(seq) = named_sequence(*named, mods, app_cursor) {
            return seq;
        }
    }

    // Character keys.
    if let Key::Character(ch) = &event.logical_key {
        let ch = ch.as_str();
        if ch.len() == 1 {
            let c = ch.as_bytes()[0];

            // Control characters: ctrl+letter, ctrl+[ \ ] ^ _ @ space 2.
            if mods.ctrl {
                let code = match c {
                    b' ' | b'2' => Some(0x00u8),
                    b'[' => Some(0x1b),
                    b'\\' => Some(0x1c),
                    b']' => Some(0x1d),
                    b'^' => Some(0x1e),
                    b'_' => Some(0x1f),
                    b'a'..=b'z' => Some(c - b'a' + 1),
                    b'A'..=b'Z' => Some(c - b'A' + 1),
                    _ => None,
                };
                if let Some(code) = code {
                    return vec![code];
                }
            }

            // Alt produces ESC-prefixed characters.
            if mods.alt {
                return vec![0x1b, c];
            }

            // Plain typing.
            return ch.as_bytes().to_vec();
        }

        // Multi-char or special text (e.g. emoji): send as-is when not modified.
        if !mods.ctrl && !mods.alt {
            return ch.as_bytes().to_vec();
        }
    }

    Vec::new()
}

/// Encode a key press under the kitty keyboard protocol.
fn encode_pressed_kitty(event: &KeyEvent, mods: Mods, mode: TermMode) -> Vec<u8> {
    let text = event.text.clone().unwrap_or_default();

    // Alt on keys with text is sent as an ESC prefix, so it is masked out of
    // the modifier bits (kitty protocol). Alt on keys without text stays a
    // modifier.
    let mods = if mods.alt && alt_send_esc(event, &text) {
        Mods { alt: false, ..mods }
    } else {
        mods
    };

    if should_build_sequence(event, &text, mode, mods) {
        build_kitty_sequence(event, mods, mode)
    } else {
        let mut bytes = Vec::new();
        if mods.alt {
            bytes.push(b'\x1b');
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
}

/// Encode a key release under the kitty keyboard protocol.
fn encode_released(event: &KeyEvent, mods: Mods, mode: TermMode) -> Vec<u8> {
    // Enter/Tab/Backspace never report releases unless every key is encoded.
    match &event.logical_key {
        Key::Named(NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace)
            if !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) =>
        {
            return Vec::new();
        },
        _ => {},
    }

    let text = event.text.clone().unwrap_or_default();
    let mods = if mods.alt && alt_send_esc(event, &text) {
        Mods { alt: false, ..mods }
    } else {
        mods
    };

    build_kitty_sequence(event, mods, mode)
}

/// Whether `Alt` should be encoded as an ESC prefix rather than a modifier.
fn alt_send_esc(event: &KeyEvent, text: &str) -> bool {
    match &event.logical_key {
        Key::Named(named) => named.to_text().is_some(),
        Key::Character(_) => text.chars().count() == 1,
        _ => false,
    }
}

/// Decide whether a key should be emitted as an escape sequence or as raw text.
fn should_build_sequence(event: &KeyEvent, text: &str, mode: TermMode, mods: Mods) -> bool {
    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        return true;
    }

    // Only shift (no ctrl/alt/super) pressed.
    let only_shift = mods.shift && !mods.ctrl && !mods.alt && !mods.super_;

    let disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES)
        && (matches!(event.logical_key, Key::Named(NamedKey::Escape))
            || event.location == KeyLocation::Numpad
            || (!mods.is_empty()
                && (!only_shift
                    || matches!(
                        event.logical_key,
                        Key::Named(NamedKey::Tab | NamedKey::Enter | NamedKey::Backspace)
                    ))));

    match &event.logical_key {
        _ if disambiguate => true,
        Key::Named(named) => named.to_text().is_none(),
        _ => text.is_empty(),
    }
}

/// Build a kitty keyboard protocol (CSI `u`) sequence for `event`.
fn build_kitty_sequence(event: &KeyEvent, mods: Mods, mode: TermMode) -> Vec<u8> {
    use std::fmt::Write;

    let mut modifiers = kitty_modifiers(mods);
    let kitty_encode_all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let kitty_event_type = mode.contains(TermMode::REPORT_EVENT_TYPES)
        && (event.repeat || event.state == ElementState::Released);

    let associated_text = event.text.as_deref().filter(|text| {
        mode.contains(TermMode::REPORT_ASSOCIATED_TEXT)
            && event.state != ElementState::Released
            && !text.is_empty()
            && !is_control_character(text)
    });

    let base = kitty_numpad(event)
        .or_else(|| kitty_named_key(event))
        .or_else(|| kitty_named_legacy(event, modifiers, kitty_event_type, associated_text.is_some()))
        .or_else(|| kitty_control_or_modifier(event, &mut modifiers, mode))
        .or_else(|| kitty_textual(event, mode, modifiers, kitty_encode_all, associated_text));

    let Some((payload, terminator)) = base else { return Vec::new() };

    let mut out = format!("\x1b[{payload}");
    if kitty_event_type || modifiers != 0 || associated_text.is_some() {
        let _ = write!(out, ";{}", modifiers + 1);
    }
    if kitty_event_type {
        let _ = write!(out, ":{}", match event.state {
            _ if event.repeat => '2',
            ElementState::Pressed => '1',
            ElementState::Released => '3',
        });
    }
    if let Some(text) = associated_text {
        let mut codepoints = text.chars().map(u32::from);
        if let Some(codepoint) = codepoints.next() {
            let _ = write!(out, ";{codepoint}");
        }
        for codepoint in codepoints {
            let _ = write!(out, ":{codepoint}");
        }
    }
    out.push(terminator);
    out.into_bytes()
}

type SequenceBase = (String, char);

/// Map a numpad key to its kitty base key code.
fn kitty_numpad(event: &KeyEvent) -> Option<SequenceBase> {
    if event.location != KeyLocation::Numpad {
        return None;
    }
    let base = match &event.logical_key {
        Key::Character(ch) => match ch.as_str() {
            "0" => "57399",
            "1" => "57400",
            "2" => "57401",
            "3" => "57402",
            "4" => "57403",
            "5" => "57404",
            "6" => "57405",
            "7" => "57406",
            "8" => "57407",
            "9" => "57408",
            "." => "57409",
            "/" => "57410",
            "*" => "57411",
            "-" => "57412",
            "+" => "57413",
            "=" => "57415",
            _ => return None,
        },
        Key::Named(named) => match *named {
            NamedKey::Enter => "57414",
            NamedKey::ArrowLeft => "57417",
            NamedKey::ArrowRight => "57418",
            NamedKey::ArrowUp => "57419",
            NamedKey::ArrowDown => "57420",
            NamedKey::PageUp => "57421",
            NamedKey::PageDown => "57422",
            NamedKey::Home => "57423",
            NamedKey::End => "57424",
            NamedKey::Insert => "57425",
            NamedKey::Delete => "57426",
            _ => return None,
        },
        _ => return None,
    };
    Some((base.to_string(), 'u'))
}

/// Kitky protocol key codes for keys that do not map to a classic sequence.
fn kitty_named_key(event: &KeyEvent) -> Option<SequenceBase> {
    let named = match &event.logical_key {
        Key::Named(named) => *named,
        _ => return None,
    };
    let base = match named {
        // F3 diverges from the classic `CSI R` to avoid colliding with DA.
        NamedKey::F3 => "13",
        NamedKey::F13 => "57376",
        NamedKey::F14 => "57377",
        NamedKey::F15 => "57378",
        NamedKey::F16 => "57379",
        NamedKey::F17 => "57380",
        NamedKey::F18 => "57381",
        NamedKey::F19 => "57382",
        NamedKey::F20 => "57383",
        NamedKey::F21 => "57384",
        NamedKey::F22 => "57385",
        NamedKey::F23 => "57386",
        NamedKey::F24 => "57387",
        NamedKey::F25 => "57388",
        NamedKey::F26 => "57389",
        NamedKey::F27 => "57390",
        NamedKey::F28 => "57391",
        NamedKey::F29 => "57392",
        NamedKey::F30 => "57393",
        NamedKey::F31 => "57394",
        NamedKey::F32 => "57395",
        NamedKey::F33 => "57396",
        NamedKey::F34 => "57397",
        NamedKey::F35 => "57398",
        NamedKey::ScrollLock => "57359",
        NamedKey::PrintScreen => "57361",
        NamedKey::Pause => "57362",
        NamedKey::ContextMenu => "57363",
        _ => return None,
    };
    Some((base.to_string(), 'u'))
}

/// Classic sequences reused by the kitty protocol (e.g. `CSI 5~`, `CSI 1;5A`).
fn kitty_named_legacy(
    event: &KeyEvent,
    modifiers: u8,
    kitty_event_type: bool,
    has_associated_text: bool,
) -> Option<SequenceBase> {
    let named = match &event.logical_key {
        Key::Named(named) => *named,
        _ => return None,
    };

    // The kitty protocol requires the base parameter to be explicit whenever
    // modifiers or an event type are attached (`CSI 1;5A` instead of `CSI A`).
    let one_based = if modifiers == 0 && !kitty_event_type && !has_associated_text {
        ""
    } else {
        "1"
    };

    let (base, terminator) = match named {
        NamedKey::PageUp => ("5", '~'),
        NamedKey::PageDown => ("6", '~'),
        NamedKey::Insert => ("2", '~'),
        NamedKey::Delete => ("3", '~'),
        NamedKey::Home => (one_based, 'H'),
        NamedKey::End => (one_based, 'F'),
        NamedKey::ArrowLeft => (one_based, 'D'),
        NamedKey::ArrowRight => (one_based, 'C'),
        NamedKey::ArrowUp => (one_based, 'A'),
        NamedKey::ArrowDown => (one_based, 'B'),
        NamedKey::F1 => (one_based, 'P'),
        NamedKey::F2 => (one_based, 'Q'),
        NamedKey::F3 => (one_based, 'R'),
        NamedKey::F4 => (one_based, 'S'),
        NamedKey::F5 => ("15", '~'),
        NamedKey::F6 => ("17", '~'),
        NamedKey::F7 => ("18", '~'),
        NamedKey::F8 => ("19", '~'),
        NamedKey::F9 => ("20", '~'),
        NamedKey::F10 => ("21", '~'),
        NamedKey::F11 => ("23", '~'),
        NamedKey::F12 => ("24", '~'),
        _ => return None,
    };

    Some((base.to_string(), terminator))
}

/// Control keys and modifier keys under the kitty protocol.
fn kitty_control_or_modifier(
    event: &KeyEvent,
    modifiers: &mut u8,
    mode: TermMode,
) -> Option<SequenceBase> {
    let kitty_encode_all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    if !kitty_encode_all && !kitty_active(mode) {
        return None;
    }

    let named = match &event.logical_key {
        Key::Named(named) => *named,
        _ => return None,
    };

    let base = match named {
        NamedKey::Tab => "9",
        NamedKey::Enter => "13",
        NamedKey::Escape => "27",
        NamedKey::Space => "32",
        NamedKey::Backspace => "127",
        _ => "",
    };

    // Only control characters are encoded unless every key is reported.
    if !kitty_encode_all && base.is_empty() {
        return None;
    }

    let base = match (named, event.location) {
        (NamedKey::Shift, KeyLocation::Left) => "57441",
        (NamedKey::Control, KeyLocation::Left) => "57442",
        (NamedKey::Alt, KeyLocation::Left) => "57443",
        (NamedKey::Super, KeyLocation::Left) => "57444",
        (NamedKey::Hyper, KeyLocation::Left) => "57445",
        (NamedKey::Meta, KeyLocation::Left) => "57446",
        (NamedKey::Shift, _) => "57447",
        (NamedKey::Control, _) => "57448",
        (NamedKey::Alt, _) => "57449",
        (NamedKey::Super, _) => "57450",
        (NamedKey::Hyper, _) => "57451",
        (NamedKey::Meta, _) => "57452",
        (NamedKey::CapsLock, _) => "57358",
        (NamedKey::NumLock, _) => "57360",
        _ => base,
    };

    // A modifier key's press state applies before the key itself, so reflect
    // it in the modifier bits (kitty protocol recommendation).
    let press = event.state.is_pressed();
    match named {
        NamedKey::Shift => set_modifier(modifiers, MOD_SHIFT, press),
        NamedKey::Control => set_modifier(modifiers, MOD_CONTROL, press),
        NamedKey::Alt => set_modifier(modifiers, MOD_ALT, press),
        NamedKey::Super => set_modifier(modifiers, MOD_SUPER, press),
        NamedKey::Hyper => set_modifier(modifiers, MOD_SUPER, press),
        NamedKey::Meta => set_modifier(modifiers, MOD_SUPER, press),
        _ => {},
    }

    if base.is_empty() {
        None
    } else {
        Some((base.to_string(), 'u'))
    }
}

/// Printable text keys under the kitty protocol.
fn kitty_textual(
    event: &KeyEvent,
    mode: TermMode,
    modifiers: u8,
    kitty_encode_all: bool,
    associated_text: Option<&str>,
) -> Option<SequenceBase> {
    let character = match &event.logical_key {
        Key::Character(ch) => ch,
        _ => return None,
    };

    if character.chars().count() != 1 {
        // No key code is available for multi-codepoint text; only report it
        // when every key is being encoded.
        return if kitty_encode_all && associated_text.is_some() {
            Some(("0".to_string(), 'u'))
        } else {
            None
        };
    }

    let ch = character.chars().next().unwrap();
    let shift = modifiers & MOD_SHIFT != 0;
    let unshifted = if shift { ch.to_lowercase().next().unwrap() } else { ch };

    let alternate_key_code = u32::from(ch);
    let mut unicode_key_code = u32::from(unshifted);

    // For shifted symbols (e.g. `!`), the key code is the unshifted base (`1`),
    // while the alternate key is the shifted character.
    if shift && alternate_key_code == unicode_key_code {
        if let Key::Character(unmodded) = event.key_without_modifiers() {
            unicode_key_code = u32::from(unmodded.chars().next().unwrap_or(unshifted));
        }
    }

    let payload = if mode.contains(TermMode::REPORT_ALTERNATE_KEYS)
        && alternate_key_code != unicode_key_code
    {
        format!("{unicode_key_code}:{alternate_key_code}")
    } else {
        unicode_key_code.to_string()
    };

    Some((payload, 'u'))
}

const MOD_SHIFT: u8 = 0b0000_0001;
const MOD_ALT: u8 = 0b0000_0010;
const MOD_CONTROL: u8 = 0b0000_0100;
const MOD_SUPER: u8 = 0b0000_1000;

fn kitty_modifiers(mods: Mods) -> u8 {
    let mut bits = 0;
    if mods.shift {
        bits |= MOD_SHIFT;
    }
    if mods.alt {
        bits |= MOD_ALT;
    }
    if mods.ctrl {
        bits |= MOD_CONTROL;
    }
    if mods.super_ {
        bits |= MOD_SUPER;
    }
    bits
}

fn set_modifier(bits: &mut u8, modifier: u8, on: bool) {
    if on {
        *bits |= modifier;
    } else {
        *bits &= !modifier;
    }
}

fn kitty_active(mode: TermMode) -> bool {
    mode.intersects(
        TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_EVENT_TYPES,
    )
}

fn is_control_character(text: &str) -> bool {
    let codepoint = text.bytes().next().unwrap();
    text.len() == 1 && (codepoint < 0x20 || (0x7f..=0x9f).contains(&codepoint))
}

fn named_sequence(named: NamedKey, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    use std::fmt::Write;

    let mut out = Vec::new();
    let modifier = match (mods.ctrl, mods.alt, mods.shift) {
        (false, false, false) => None,
        (true, false, false) => Some(5),
        (false, true, false) => Some(3),
        (false, false, true) => Some(2),
        (true, false, true) => Some(6),
        (true, true, false) => Some(7),
        (false, true, true) => Some(4),
        (true, true, true) => Some(8),
    };

    let sgr = |param: &str| -> Vec<u8> {
        match modifier {
            Some(m) => {
                let mut buf = String::new();
                let _ = write!(buf, "\x1b[{param};{m}");
                buf.into_bytes()
            },
            None => format!("\x1b[{param}").into_bytes(),
        }
    };

    match named {
        NamedKey::Enter => out.extend_from_slice(b"\r"),
        NamedKey::Tab => {
            if mods.shift {
                out.extend_from_slice(b"\x1b[Z");
            } else {
                out.extend_from_slice(b"\t");
            }
        },
        NamedKey::Escape => out.push(0x1b),
        NamedKey::Backspace => out.push(0x7f),
        NamedKey::Space => out.push(b' '),
        NamedKey::ArrowUp => {
            if modifier.is_some() {
                out.extend(sgr("A"));
            } else if app_cursor {
                out.extend_from_slice(b"\x1bOA");
            } else {
                out.extend_from_slice(b"\x1b[A");
            }
        },
        NamedKey::ArrowDown => {
            if modifier.is_some() {
                out.extend(sgr("B"));
            } else if app_cursor {
                out.extend_from_slice(b"\x1bOB");
            } else {
                out.extend_from_slice(b"\x1b[B");
            }
        },
        NamedKey::ArrowRight => {
            if modifier.is_some() {
                out.extend(sgr("C"));
            } else if app_cursor {
                out.extend_from_slice(b"\x1bOC");
            } else {
                out.extend_from_slice(b"\x1b[C");
            }
        },
        NamedKey::ArrowLeft => {
            if modifier.is_some() {
                out.extend(sgr("D"));
            } else if app_cursor {
                out.extend_from_slice(b"\x1bOD");
            } else {
                out.extend_from_slice(b"\x1b[D");
            }
        },
        NamedKey::PageUp => out.extend(sgr("5~")),
        NamedKey::PageDown => out.extend(sgr("6~")),
        NamedKey::Home => out.extend(sgr("H")),
        NamedKey::End => out.extend(sgr("F")),
        NamedKey::Insert => out.extend(sgr("2~")),
        NamedKey::Delete => out.extend(sgr("3~")),
        NamedKey::F1 => out.extend_from_slice(b"\x1bOP"),
        NamedKey::F2 => out.extend_from_slice(b"\x1bOQ"),
        NamedKey::F3 => out.extend_from_slice(b"\x1bOR"),
        NamedKey::F4 => out.extend_from_slice(b"\x1bOS"),
        NamedKey::F5 => out.extend(sgr("15~")),
        NamedKey::F6 => out.extend(sgr("17~")),
        NamedKey::F7 => out.extend(sgr("18~")),
        NamedKey::F8 => out.extend(sgr("19~")),
        NamedKey::F9 => out.extend(sgr("20~")),
        NamedKey::F10 => out.extend(sgr("21~")),
        NamedKey::F11 => out.extend(sgr("23~")),
        NamedKey::F12 => out.extend(sgr("24~")),
        _ => return None,
    }

    let _ = app_cursor;
    Some(out)
}

/// Xterm modifier flags folded into the mouse report button code.
fn mouse_modifier_flags(mods: Mods) -> u16 {
    let mut bits = 0;
    // Per the SGR/X10 mouse protocols: shift +4, meta/alt +8, ctrl +16.
    if mods.shift {
        bits += 4;
    }
    if mods.alt {
        bits += 8;
    }
    if mods.ctrl {
        bits += 16;
    }
    bits
}

/// SGR mouse button codes. `3` marks a button release (any button).
const MOUSE_RELEASE: u16 = 3;
/// SGR code for motion with no button held.
const MOUSE_MOTION: u16 = 32;
/// SGR code for drag motion with a button held.
const MOUSE_DRAG: u16 = 35;

/// Encode one mouse event as SGR or X10 bytes for the PTY.
///
/// * `button` — SGR button code (0 = left, 1 = middle, 2 = right, 3 = release,
///   64/65/66/67 = wheel up/down/left/right, 32/35 = motion/drag).
/// * `col`/`row` — 0-based grid position; converted to 1-based for reporting.
/// * `mods` — modifier state folded into the report's button code.
/// * `sgr` — use SGR (`CSI < Cb ; Cx ; Cy M`) instead of legacy X10 (`CSI M b x y`).
pub fn encode_mouse(button: u16, col: usize, row: usize, mods: Mods, sgr: bool) -> Vec<u8> {
    let code = button + mouse_modifier_flags(mods);
    if sgr {
        format!("\x1b[<{code};{};{}M", col + 1, row + 1).into_bytes()
    } else {
        let mut out = Vec::with_capacity(6);
        out.extend_from_slice(b"\x1b[M");
        out.push((code + 32) as u8);
        out.push((col + 1 + 32) as u8);
        out.push((row + 1 + 32) as u8);
        out
    }
}

/// Encode a wheel scroll as mouse-report bytes.
///
/// `delta_lines` is the number of lines to scroll (positive = toward older
/// content / wheel up). Line-delta devices report whole notches; pixel-delta
/// devices (touchpads) accumulate fractional lines, so this rounds and emits
/// one report per whole line crossed so apps see discrete wheel steps.
pub fn encode_mouse_wheel(delta_lines: f64, col: usize, row: usize, mods: Mods, sgr: bool) -> Vec<u8> {
    let steps = delta_lines.abs().round() as usize;
    if steps == 0 {
        return Vec::new();
    }
    let button = if delta_lines > 0.0 { 64 } else { 65 };
    let mut out = Vec::with_capacity(steps * 12);
    for _ in 0..steps {
        out.extend_from_slice(&encode_mouse(button, col, row, mods, sgr));
    }
    out
}

/// Whether the terminal is in any mouse-reporting mode (app wants mouse events).
pub fn mouse_reporting_active(mode: TermMode) -> bool {
    mode.intersects(
        TermMode::MOUSE_REPORT_CLICK
            | TermMode::MOUSE_MOTION
            | TermMode::MOUSE_DRAG
            | TermMode::SGR_MOUSE
            | TermMode::UTF8_MOUSE,
    )
}

#[cfg(test)]
mod mouse_tests {
    use super::*;

    #[test]
    fn sgr_wheel_up_is_reported() {
        // Wheel up at column 4, row 7 -> `CSI < 64 ; 5 ; 8 M`.
        let bytes = encode_mouse_wheel(1.0, 4, 7, Mods::default(), true);
        assert_eq!(bytes, b"\x1b[<64;5;8M");
    }

    #[test]
    fn sgr_wheel_down_with_ctrl() {
        // ctrl (+16) + wheel down (65) = 81.
        let mods = Mods { ctrl: true, ..Mods::default() };
        let bytes = encode_mouse_wheel(-1.0, 0, 0, mods, true);
        assert_eq!(bytes, b"\x1b[<81;1;1M");
    }

    #[test]
    fn sgr_click_press_and_release() {
        let mods = Mods::default();
        assert_eq!(encode_mouse(0, 1, 2, mods, true), b"\x1b[<0;2;3M");
        assert_eq!(encode_mouse(MOUSE_RELEASE, 1, 2, mods, true), b"\x1b[<3;2;3M");
    }

    #[test]
    fn x10_encoding_offsets_by_32() {
        // Legacy X10: `ESC [ M` then each byte + 32 (button first).
        let bytes = encode_mouse(0, 0, 0, Mods::default(), false);
        assert_eq!(bytes, b"\x1b[M" .iter().copied().chain([32, 33, 33]).collect::<Vec<u8>>());
    }

    #[test]
    fn fractional_wheel_rounds_to_steps() {
        let mods = Mods::default();
        // 2.6 lines -> 3 wheel reports (one per whole line).
        let bytes = encode_mouse_wheel(2.6, 0, 0, mods, true);
        assert_eq!(bytes, b"\x1b[<64;1;1M\x1b[<64;1;1M\x1b[<64;1;1M");
        // 0.3 lines below a step -> nothing yet (avoids wheel jitter).
        assert!(encode_mouse_wheel(0.3, 0, 0, mods, true).is_empty());
    }

    #[test]
    fn mouse_reporting_detection() {
        assert!(mouse_reporting_active(TermMode::SGR_MOUSE));
        assert!(mouse_reporting_active(TermMode::MOUSE_REPORT_CLICK));
        assert!(!mouse_reporting_active(TermMode::empty()));
    }
}
