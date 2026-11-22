//! Keyboard and pointer input as X11 keysyms and VNC button bits, and the header name helper.

use slint::platform::Key;

const SHIFT_L: u32 = 0xffe1;
const CONTROL_L: u32 = 0xffe3;
const ALT_L: u32 = 0xffe9;
const DELETE: u32 = 0xffff;

/// The X11 keysym for a key as Slint reports it, or None for a key X11 has no use for.
pub fn keysym(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    let (first, None) = (chars.next()?, chars.next()) else {
        return None;
    };
    const NAMED: [(Key, u32); 40] = [
        (Key::Backspace, 0xff08),
        (Key::Tab, 0xff09),
        (Key::Backtab, 0xfe20),
        (Key::Return, 0xff0d),
        (Key::Escape, 0xff1b),
        (Key::Delete, DELETE),
        (Key::Home, 0xff50),
        (Key::LeftArrow, 0xff51),
        (Key::UpArrow, 0xff52),
        (Key::RightArrow, 0xff53),
        (Key::DownArrow, 0xff54),
        (Key::PageUp, 0xff55),
        (Key::PageDown, 0xff56),
        (Key::End, 0xff57),
        (Key::Insert, 0xff63),
        (Key::Menu, 0xff67),
        (Key::Pause, 0xff13),
        (Key::ScrollLock, 0xff14),
        (Key::SysReq, 0xff15),
        (Key::Shift, SHIFT_L),
        (Key::ShiftR, 0xffe2),
        (Key::Control, CONTROL_L),
        (Key::ControlR, 0xffe4),
        (Key::CapsLock, 0xffe5),
        (Key::Alt, ALT_L),
        (Key::AltGr, 0xfe03),
        (Key::Meta, 0xffeb),
        (Key::MetaR, 0xffec),
        (Key::F1, 0xffbe),
        (Key::F2, 0xffbf),
        (Key::F3, 0xffc0),
        (Key::F4, 0xffc1),
        (Key::F5, 0xffc2),
        (Key::F6, 0xffc3),
        (Key::F7, 0xffc4),
        (Key::F8, 0xffc5),
        (Key::F9, 0xffc6),
        (Key::F10, 0xffc7),
        (Key::F11, 0xffc8),
        (Key::F12, 0xffc9),
    ];
    if let Some((_, sym)) = NAMED.iter().find(|(key, _)| char::from(*key) == first) {
        return Some(*sym);
    }
    let code = first as u32;
    match code {
        // A letter typed with Control held can arrive as its control character.
        0x01..=0x1a => Some(code + 0x60),
        // Printable Latin-1 has the same number as a keysym.
        0x20..=0x7e | 0xa0..=0xff => Some(code),
        // Other control codes, and Slint's own codes for keys it has no text for.
        0x00 | 0x1b..=0x1f | 0x7f..=0x9f | 0xe000..=0xf8ff => None,
        // Any other character: the Unicode keysym range.
        _ => Some(0x0100_0000 + code),
    }
}

/// The same letter key in the other case: a key released after Shift arrives lowercase.
pub(super) fn other_case(sym: u32) -> Option<u32> {
    let letter = char::from_u32(sym).filter(char::is_ascii_alphabetic)?;
    Some(if letter.is_ascii_lowercase() {
        letter.to_ascii_uppercase()
    } else {
        letter.to_ascii_lowercase()
    } as u32)
}

/// Key presses and releases for Ctrl+Alt+Del, in order.
pub fn ctrl_alt_del() -> [(u32, bool); 6] {
    [
        (CONTROL_L, true),
        (ALT_L, true),
        (DELETE, true),
        (DELETE, false),
        (ALT_L, false),
        (CONTROL_L, false),
    ]
}

/// A desktop name as the host reports it ("xfce"), with a capital to start the header line.
pub(super) fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The VNC button mask bit of a mouse button (0 left, 1 middle, 2 right).
pub(super) fn button_bit(button: i32) -> u8 {
    match button {
        1 => 2,
        2 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key) -> String {
        char::from(key).to_string()
    }

    #[test]
    fn printable_text_maps_to_its_own_keysym() {
        assert_eq!(keysym("a"), Some(0x61));
        assert_eq!(keysym("A"), Some(0x41));
        assert_eq!(keysym(" "), Some(0x20));
        assert_eq!(keysym("~"), Some(0x7e));
        assert_eq!(keysym("é"), Some(0xe9));
        // Beyond Latin-1: Unicode keysyms.
        assert_eq!(keysym("€"), Some(0x0100_20ac));
        assert_eq!(keysym("ж"), Some(0x0100_0436));
        assert_eq!(keysym(""), None);
        assert_eq!(keysym("ab"), None);
    }

    #[test]
    fn named_keys_map_to_x11_function_keysyms() {
        assert_eq!(keysym(&key(Key::Return)), Some(0xff0d));
        assert_eq!(keysym(&key(Key::Backspace)), Some(0xff08));
        assert_eq!(keysym(&key(Key::Tab)), Some(0xff09));
        assert_eq!(keysym(&key(Key::Escape)), Some(0xff1b));
        assert_eq!(keysym(&key(Key::Delete)), Some(0xffff));
        assert_eq!(keysym(&key(Key::LeftArrow)), Some(0xff51));
        assert_eq!(keysym(&key(Key::DownArrow)), Some(0xff54));
        assert_eq!(keysym(&key(Key::PageUp)), Some(0xff55));
        assert_eq!(keysym(&key(Key::F1)), Some(0xffbe));
        assert_eq!(keysym(&key(Key::F12)), Some(0xffc9));
        assert_eq!(keysym(&key(Key::Shift)), Some(0xffe1));
        assert_eq!(keysym(&key(Key::Control)), Some(0xffe3));
        assert_eq!(keysym(&key(Key::Alt)), Some(0xffe9));
        assert_eq!(keysym(&key(Key::Meta)), Some(0xffeb));
    }

    #[test]
    fn control_characters_and_private_codes() {
        // Ctrl+C reported as its control character is still the letter c.
        assert_eq!(keysym("\u{3}"), Some(0x63));
        // A key Slint has no text for (media keys and the like).
        assert_eq!(keysym("\u{f8ff}"), None);
        assert_eq!(other_case(0x61), Some(0x41));
        assert_eq!(other_case(0x5a), Some(0x7a));
        assert_eq!(other_case(0xff0d), None);
    }

    #[test]
    fn ctrl_alt_del_presses_then_releases_in_reverse() {
        let keys = ctrl_alt_del();
        assert_eq!(keys[..3], [(0xffe3, true), (0xffe9, true), (0xffff, true)]);
        assert_eq!(
            keys[3..],
            [(0xffff, false), (0xffe9, false), (0xffe3, false)]
        );
        assert_eq!((button_bit(0), button_bit(1), button_bit(2)), (1, 2, 4));
        assert_eq!(capitalised("xfce"), "Xfce");
        assert_eq!(capitalised(""), "");
    }
}
