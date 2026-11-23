//! Terminal colours: the 16 ANSI colours tuned to the app theme, plus the xterm 256-colour cube.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

pub const FOREGROUND: Rgb = rgb(0xe8, 0xea, 0xf0);
pub const BACKGROUND: Rgb = rgb(0x14, 0x17, 0x29);
/// Background of selected text: the brand accent, darkened so any foreground stays readable.
pub const SELECTION: Rgb = rgb(0x1f, 0x4d, 0x45);

const ANSI: [Rgb; 16] = [
    rgb(0x28, 0x2b, 0x3d),
    rgb(0xf0, 0x80, 0x7d),
    rgb(0x86, 0xce, 0x77),
    rgb(0xd9, 0xbe, 0x5a),
    rgb(0x6f, 0xa8, 0xf5),
    rgb(0xa5, 0x8b, 0xf0),
    rgb(0x5c, 0xc3, 0xdc),
    rgb(0xc5, 0xca, 0xd3),
    rgb(0x6a, 0x6e, 0x84),
    rgb(0xf5, 0x9a, 0x97),
    rgb(0xa3, 0xdd, 0x96),
    rgb(0xe8, 0xd2, 0x7c),
    rgb(0x91, 0xbd, 0xf8),
    rgb(0xbd, 0xa8, 0xf5),
    rgb(0x86, 0xd6, 0xe8),
    rgb(0xf2, 0xf4, 0xf7),
];

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

fn dim(c: Rgb) -> Rgb {
    rgb(
        (c.r as u16 * 2 / 3) as u8,
        (c.g as u16 * 2 / 3) as u8,
        (c.b as u16 * 2 / 3) as u8,
    )
}

/// Colour `index` of the xterm 256-colour palette.
pub fn indexed(index: u8) -> Rgb {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (index - 232) * 10;
            rgb(v, v, v)
        }
    }
}

fn named(color: NamedColor) -> Rgb {
    match color {
        NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => FOREGROUND,
        NamedColor::Background => BACKGROUND,
        NamedColor::DimForeground => dim(FOREGROUND),
        NamedColor::DimBlack => dim(ANSI[0]),
        NamedColor::DimRed => dim(ANSI[1]),
        NamedColor::DimGreen => dim(ANSI[2]),
        NamedColor::DimYellow => dim(ANSI[3]),
        NamedColor::DimBlue => dim(ANSI[4]),
        NamedColor::DimMagenta => dim(ANSI[5]),
        NamedColor::DimCyan => dim(ANSI[6]),
        NamedColor::DimWhite => dim(ANSI[7]),
        other => ANSI[(other as usize).min(15)],
    }
}

/// Resolve a cell colour, honouring colours a program changed at runtime (OSC 4/10/11).
pub fn resolve(color: Color, overrides: &Colors) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Named(name) => overrides[name].unwrap_or_else(|| named(name)),
        Color::Indexed(index) => overrides[index as usize].unwrap_or_else(|| indexed(index)),
    }
}

/// The colour reported for an OSC colour query.
pub fn query(index: usize, overrides: &Colors) -> Rgb {
    if let Some(rgb) = overrides[index] {
        return rgb;
    }
    match index {
        0..=255 => indexed(index as u8),
        256 => FOREGROUND,
        257 => BACKGROUND,
        _ => FOREGROUND,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grey_ramp_match_xterm() {
        assert_eq!(indexed(16), rgb(0, 0, 0));
        assert_eq!(indexed(21), rgb(0, 0, 255));
        assert_eq!(indexed(196), rgb(255, 0, 0));
        assert_eq!(indexed(231), rgb(255, 255, 255));
        assert_eq!(indexed(232), rgb(8, 8, 8));
        assert_eq!(indexed(255), rgb(238, 238, 238));
    }
}
