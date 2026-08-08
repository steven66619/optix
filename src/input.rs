/// Keyboard input handling: keybinding resolution and terminal key encoding.
use std::collections::HashMap;

use winit::event::KeyEvent;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

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

    fn is_empty(self) -> bool {
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
/// `app_cursor` reflects DECCKM (application cursor keys mode); `app_keypad`
/// reflects DECKPAM. Returns an empty vec when the key produces no output.
pub fn encode_key(event: &KeyEvent, mods: Mods, app_cursor: bool, _app_keypad: bool) -> Vec<u8> {
    use winit::event::ElementState;    if event.state != ElementState::Pressed {
        return Vec::new();
    }

    // Shift is tracked separately for alt handling; ctrl suppresses text.
    let ctrl = mods.ctrl;
    let alt = mods.alt;
    let _shift = mods.shift;

    // Named keys with dedicated sequences.
    if let Key::Named(named) = &event.logical_key {
        let seq = named_sequence(*named, mods, app_cursor);
        if let Some(seq) = seq {
            return seq;
        }
    }

    // Character keys.
    if let Key::Character(ch) = &event.logical_key {
        let ch = ch.as_str();
        if ch.len() == 1 {
            let c = ch.as_bytes()[0];

            // Control characters: ctrl+letter, ctrl+[ \ ] ^ _ @ space 2.
            if ctrl {
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
            if alt {
                return vec![0x1b, c];
            }

            // Plain typing.
            return ch.as_bytes().to_vec();
        }

        // Multi-char or special text (e.g. emoji): send as-is when not modified.
        if !ctrl && !alt {
            return ch.as_bytes().to_vec();
        }
    }

    Vec::new()
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
