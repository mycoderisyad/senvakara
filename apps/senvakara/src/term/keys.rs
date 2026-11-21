//! Turns a key press reported by Slint into the bytes a terminal program expects.

use slint::platform::Key;

fn is(text: &str, key: Key) -> bool {
    let mut chars = text.chars();
    chars.next() == Some(char::from(key)) && chars.next().is_none()
}

/// xterm modifier parameter: 1 + shift(1) + alt(2) + control(4).
fn modifier(control: bool, alt: bool, shift: bool) -> u8 {
    1 + shift as u8 + 2 * alt as u8 + 4 * control as u8
}

fn csi_letter(letter: char, app_cursor: bool, m: u8) -> Vec<u8> {
    if m > 1 {
        format!("\x1b[1;{m}{letter}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn csi_tilde(code: u8, m: u8) -> Vec<u8> {
    if m > 1 {
        format!("\x1b[{code};{m}~").into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

/// Bytes for one key press, or `None` when the key sends nothing (a bare modifier, say).
pub fn encode(
    text: &str,
    control: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let m = modifier(control, alt, shift);

    // Slint reports a modifier pressed on its own as a C0 code point; it sends nothing.
    const BARE: [Key; 9] = [
        Key::Shift,
        Key::ShiftR,
        Key::Control,
        Key::ControlR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
        Key::CapsLock,
    ];
    if BARE.into_iter().any(|key| is(text, key)) {
        return None;
    }

    const LETTERS: [(Key, char); 6] = [
        (Key::UpArrow, 'A'),
        (Key::DownArrow, 'B'),
        (Key::RightArrow, 'C'),
        (Key::LeftArrow, 'D'),
        (Key::Home, 'H'),
        (Key::End, 'F'),
    ];
    for (key, letter) in LETTERS {
        if is(text, key) {
            return Some(csi_letter(letter, app_cursor, m));
        }
    }

    const TILDES: [(Key, u8); 12] = [
        (Key::Insert, 2),
        (Key::Delete, 3),
        (Key::PageUp, 5),
        (Key::PageDown, 6),
        (Key::F5, 15),
        (Key::F6, 17),
        (Key::F7, 18),
        (Key::F8, 19),
        (Key::F9, 20),
        (Key::F10, 21),
        (Key::F11, 23),
        (Key::F12, 24),
    ];
    for (key, code) in TILDES {
        if is(text, key) {
            return Some(csi_tilde(code, m));
        }
    }

    const SS3: [(Key, char); 4] = [
        (Key::F1, 'P'),
        (Key::F2, 'Q'),
        (Key::F3, 'R'),
        (Key::F4, 'S'),
    ];
    for (key, letter) in SS3 {
        if is(text, key) {
            return Some(if m > 1 {
                format!("\x1b[1;{m}{letter}").into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            });
        }
    }

    if is(text, Key::Backtab) || (is(text, Key::Tab) && shift) {
        return Some(b"\x1b[Z".to_vec());
    }

    let mut out = Vec::new();
    if alt {
        out.push(0x1b);
    }
    if is(text, Key::Return) {
        out.push(b'\r');
    } else if is(text, Key::Backspace) {
        out.push(if control { 0x08 } else { 0x7f });
    } else if is(text, Key::Tab) {
        out.push(b'\t');
    } else if is(text, Key::Escape) {
        out.push(0x1b);
    } else {
        let mut chars = text.chars();
        let first = chars.next()?;
        let single = chars.next().is_none();
        // Slint reports the remaining special keys (lock keys, the menu key, media keys) as
        // private-use code points: they send nothing.
        if single && ('\u{e000}'..='\u{f8ff}').contains(&first) {
            return None;
        }
        if control && single && first.is_ascii() {
            let byte = match first.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 & 0x1f,
                ' ' | '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '/' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                c => c as u8,
            };
            out.push(byte);
        } else {
            out.extend_from_slice(text.as_bytes());
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key) -> String {
        char::from(key).to_string()
    }

    #[test]
    fn plain_text_and_control_letters() {
        assert_eq!(encode("a", false, false, false, false), Some(b"a".to_vec()));
        assert_eq!(encode("c", true, false, false, false), Some(vec![0x03]));
        assert_eq!(encode("C", true, false, false, false), Some(vec![0x03]));
        assert_eq!(
            encode("x", false, true, false, false),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            encode("é", false, false, false, false),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn editing_keys() {
        assert_eq!(
            encode(&key(Key::Return), false, false, false, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode(&key(Key::Backspace), false, false, false, false),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode(&key(Key::Tab), false, false, true, false),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode(&key(Key::Delete), false, false, false, false),
            Some(b"\x1b[3~".to_vec())
        );
    }

    #[test]
    fn arrows_follow_application_cursor_mode_and_modifiers() {
        let up = key(Key::UpArrow);
        assert_eq!(
            encode(&up, false, false, false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode(&up, false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode(&up, true, false, false, true),
            Some(b"\x1b[1;5A".to_vec())
        );
        assert_eq!(
            encode(&key(Key::F5), false, false, true, false),
            Some(b"\x1b[15;2~".to_vec())
        );
    }

    #[test]
    fn bare_modifiers_send_nothing() {
        assert_eq!(encode(&key(Key::Shift), false, false, true, false), None);
        assert_eq!(encode(&key(Key::Control), true, false, false, false), None);
    }
}
