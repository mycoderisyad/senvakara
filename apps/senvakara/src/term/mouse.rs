//! Mouse reports for programs that asked for them (tmux, htop, vim with `mouse=a`).

/// What happened, in xterm's button numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Button 0 left, 1 middle, 2 right.
    Press(u8),
    Release(u8),
    /// Pointer moved; the button held down, if any.
    Motion(Option<u8>),
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// Encode one report for the cell at zero-based `col`, `row`. `sgr` selects the SGR (1006)
/// format; otherwise the legacy X10 format is used, which cannot address cells past 223.
pub fn encode(
    action: Action,
    col: usize,
    row: usize,
    modifiers: Modifiers,
    sgr: bool,
) -> Option<Vec<u8>> {
    let (mut code, release) = match action {
        Action::Press(button) => (button.min(2), false),
        Action::Release(button) => (button.min(2), true),
        Action::Motion(Some(button)) => (32 + button.min(2), false),
        Action::Motion(None) => (35, false),
        Action::WheelUp => (64, false),
        Action::WheelDown => (65, false),
    };
    code += 4 * modifiers.shift as u8 + 8 * modifiers.alt as u8 + 16 * modifiers.control as u8;

    if sgr {
        let end = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes());
    }
    if col >= 223 || row >= 223 {
        return None;
    }
    if release {
        // X10 cannot say which button was released.
        code = (code & !3) | 3;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        33 + col as u8,
        33 + row as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_reports() {
        let none = Modifiers::default();
        assert_eq!(
            encode(Action::Press(0), 0, 0, none, true),
            Some(b"\x1b[<0;1;1M".to_vec())
        );
        assert_eq!(
            encode(Action::Release(0), 9, 4, none, true),
            Some(b"\x1b[<0;10;5m".to_vec())
        );
        assert_eq!(
            encode(Action::Motion(Some(0)), 2, 3, none, true),
            Some(b"\x1b[<32;3;4M".to_vec())
        );
        assert_eq!(
            encode(Action::WheelDown, 300, 2, none, true),
            Some(b"\x1b[<65;301;3M".to_vec())
        );
        let control = Modifiers {
            control: true,
            ..none
        };
        assert_eq!(
            encode(Action::Press(2), 0, 0, control, true),
            Some(b"\x1b[<18;1;1M".to_vec())
        );
    }

    #[test]
    fn legacy_reports() {
        let none = Modifiers::default();
        assert_eq!(
            encode(Action::Press(0), 0, 0, none, false),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        assert_eq!(
            encode(Action::Release(1), 1, 2, none, false),
            Some(vec![0x1b, b'[', b'M', 35, 34, 35])
        );
        assert_eq!(encode(Action::Press(0), 250, 0, none, false), None);
    }
}
