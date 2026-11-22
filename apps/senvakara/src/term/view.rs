//! What the UI thread reads from a session: the row model, the cursor, the title and exit state.

use std::sync::atomic::Ordering;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term as Emulator, TermDamage};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{TermLive, TermRow, TermRun};

use super::listener::Listener;
use super::{palette, Session};

/// What the view needs besides the rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub col: i32,
    pub row: i32,
    pub visible: bool,
    pub scrollback: i32,
}

impl Session {
    pub fn rows(&self) -> ModelRc<TermRow> {
        self.rows.clone().into()
    }

    /// The one-row model the view reads the cursor from.
    pub fn live(&self) -> ModelRc<TermLive> {
        self.live.clone().into()
    }

    /// Tell the view where the cursor is now.
    pub fn set_live(&self, live: TermLive) {
        if self.live.row_data(0).as_ref() != Some(&live) {
            self.live.set_row_data(0, live);
        }
    }

    pub fn exited(&self) -> bool {
        self.shared.exited.load(Ordering::Acquire)
    }

    /// A title the program set since the last call.
    pub fn take_title(&self) -> Option<String> {
        self.shared.title.lock().unwrap().take()
    }

    /// Clear the pending-refresh flag without drawing (for a tab that is not visible).
    pub fn acknowledge(&self) {
        self.shared.dirty.store(false, Ordering::Release);
    }

    /// Bring the row model up to date. UI thread only. `full` redraws every line.
    pub fn sync(&self, full: bool) -> Cursor {
        self.shared.dirty.store(false, Ordering::Release);
        let full = full || self.redraw_all.take();
        let mut core = self.core.lock().unwrap();
        let emulator = &mut core.emulator;
        let lines = emulator.screen_lines();
        let cols = emulator.columns();

        let damaged: Option<Vec<usize>> = match emulator.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(iter) => Some(iter.map(|bounds| bounds.line).collect()),
        };
        emulator.reset_damage();

        match damaged {
            Some(damaged) if !full && self.rows.row_count() == lines => {
                for line in damaged.into_iter().filter(|line| *line < lines) {
                    self.rows
                        .set_row_data(line, build_row(emulator, line, cols));
                }
            }
            _ => {
                let rows: Vec<TermRow> = (0..lines)
                    .map(|line| build_row(emulator, line, cols))
                    .collect();
                self.rows.set_vec(rows);
            }
        }

        let offset = emulator.grid().display_offset() as i32;
        let cursor = emulator.renderable_content().cursor;
        Cursor {
            col: cursor.point.column.0 as i32,
            row: cursor.point.line.0 + offset,
            visible: cursor.shape != CursorShape::Hidden,
            scrollback: offset,
        }
    }
}

fn to_color(rgb: Rgb) -> slint::Color {
    slint::Color::from_rgb_u8(rgb.r, rgb.g, rgb.b)
}

/// Characters that keep the cell width in the bundled monospace face, so a run of them can be
/// drawn as one text element. Anything else gets a run of its own to stay on the grid.
fn groupable(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{a0}'..='\u{24f}' | '\u{2500}'..='\u{259f}')
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Rgb,
    bg: Rgb,
    has_bg: bool,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// Build the styled runs for one line of the viewport.
fn build_row(emulator: &Emulator<Listener>, viewport_line: usize, cols: usize) -> TermRow {
    let offset = emulator.grid().display_offset() as i32;
    let row = &emulator.grid()[Line(viewport_line as i32 - offset)];
    let colors = emulator.colors();
    let line = Line(viewport_line as i32 - offset);
    let selection: Option<SelectionRange> = emulator
        .selection
        .as_ref()
        .and_then(|selection| selection.to_range(emulator));

    let mut runs: Vec<TermRun> = Vec::new();
    // The run being built: start column, width in cells, style, text.
    let mut open: Option<(usize, usize, Style, String)> = None;
    let mut flush = |open: &mut Option<(usize, usize, Style, String)>| {
        if let Some((col, width, style, text)) = open.take() {
            runs.push(TermRun {
                text: SharedString::from(text),
                col: col as i32,
                cols: width as i32,
                fg: to_color(style.fg),
                bg: to_color(style.bg),
                has_bg: style.has_bg,
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
            });
        }
    };

    for col in 0..cols {
        let cell = &row[Column(col)];
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }

        let mut fg = palette::resolve(cell.fg, colors);
        let mut bg = palette::resolve(cell.bg, colors);
        let mut has_bg = cell.bg != Color::Named(NamedColor::Background);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            has_bg = true;
        }
        if flags.contains(Flags::DIM) {
            fg = Rgb {
                r: (fg.r as u16 * 2 / 3) as u8,
                g: (fg.g as u16 * 2 / 3) as u8,
                b: (fg.b as u16 * 2 / 3) as u8,
            };
        }
        if selection.is_some_and(|range| range.contains(Point::new(line, Column(col)))) {
            bg = palette::SELECTION;
            has_bg = true;
        }
        let c = if flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        let style = Style {
            fg,
            bg,
            has_bg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
        };

        // Blank cells on the default background draw nothing.
        if c == ' ' && !has_bg && !style.underline {
            flush(&mut open);
            continue;
        }

        let wide = flags.contains(Flags::WIDE_CHAR);
        if wide || !groupable(c) {
            flush(&mut open);
            open = Some((col, if wide { 2 } else { 1 }, style, c.to_string()));
            flush(&mut open);
            continue;
        }

        match &mut open {
            Some((start, width, open_style, text))
                if *open_style == style && *start + *width == col =>
            {
                text.push(c);
                *width += 1;
            }
            _ => {
                flush(&mut open);
                open = Some((col, 1, style, c.to_string()));
            }
        }
    }
    flush(&mut open);

    TermRow {
        runs: ModelRc::new(VecModel::from(runs)),
    }
}
