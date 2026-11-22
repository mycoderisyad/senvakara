//! Input from the view: keys, paste, the mouse wheel, pointer reports and text selection.

use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{viewport_to_point, TermMode};

use super::mouse::{self, Action, Modifiers};
use super::{keys, Cmd, Session};

/// Presses on the same cell closer together than this count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// A pointer event from the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Down,
    Up,
    Move,
    /// The toolkit recognised a double click: select the word under the pointer.
    Word,
}

impl Session {
    /// Handle a key press. Returns false when the key sends nothing.
    pub fn key(&self, text: &str, control: bool, alt: bool, shift: bool) -> bool {
        let mut core = self.core.lock().unwrap();
        let app_cursor = core.emulator.mode().contains(TermMode::APP_CURSOR);
        let Some(bytes) = keys::encode(text, control, alt, shift, app_cursor) else {
            return false;
        };
        // Typing returns to the live screen and drops the selection.
        core.emulator.scroll_display(Scroll::Bottom);
        if core.emulator.selection.take().is_some() {
            self.redraw_all.set(true);
            self.shared.mark();
        }
        drop(core);
        let _ = self.tx.send(Cmd::Input(bytes));
        true
    }

    /// Paste text, bracketed when the program asked for it.
    pub fn paste(&self, text: &str) {
        let bracketed = self
            .core
            .lock()
            .unwrap()
            .emulator
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed {
            // Strip the end marker so pasted text cannot break out of the paste.
            format!("\x1b[200~{}\x1b[201~", body.replace("\x1b[201~", "")).into_bytes()
        } else {
            body.into_bytes()
        };
        let _ = self.tx.send(Cmd::Input(bytes));
    }

    /// Mouse wheel: report it to a program that tracks the mouse, send arrow keys to any other
    /// full-screen program, and scroll the history otherwise.
    pub fn scroll(&self, lines: i32, col: usize, row: usize, modifiers: Modifiers) {
        let mut core = self.core.lock().unwrap();
        let mode = *core.emulator.mode();
        if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
            let action = if lines > 0 {
                Action::WheelUp
            } else {
                Action::WheelDown
            };
            let sgr = mode.contains(TermMode::SGR_MOUSE);
            if let Some(bytes) = mouse::encode(action, col, row, modifiers, sgr) {
                let _ = self.tx.send(Cmd::Input(bytes));
            }
        } else if mode.contains(TermMode::ALT_SCREEN) {
            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            let key = if lines > 0 { "A" } else { "B" };
            let sequence = if app_cursor {
                format!("\x1bO{key}")
            } else {
                format!("\x1b[{key}")
            };
            let bytes = sequence.repeat(lines.unsigned_abs() as usize).into_bytes();
            let _ = self.tx.send(Cmd::Input(bytes));
        } else {
            core.emulator.scroll_display(Scroll::Delta(lines));
            drop(core);
            self.shared.mark();
        }
    }

    /// A pointer event on the cell at `col`, `row` of the viewport. Programs that track the
    /// mouse get a report; otherwise the left button selects text. Holding Shift always selects.
    pub fn pointer(
        &self,
        event: Pointer,
        button: u8,
        col: usize,
        row: usize,
        right_half: bool,
        modifiers: Modifiers,
    ) {
        let mut core = self.core.lock().unwrap();
        let emulator = &mut core.emulator;
        let mode = *emulator.mode();
        let col = col.min(emulator.columns().saturating_sub(1));
        let row = row.min(emulator.screen_lines().saturating_sub(1));

        if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
            let action = match event {
                Pointer::Down => {
                    self.held.set(Some(button));
                    Action::Press(button)
                }
                Pointer::Up => {
                    self.held.set(None);
                    Action::Release(button)
                }
                Pointer::Word => return,
                Pointer::Move => {
                    let held = self.held.get();
                    let wanted = mode.contains(TermMode::MOUSE_MOTION)
                        || (held.is_some() && mode.contains(TermMode::MOUSE_DRAG));
                    if !wanted {
                        return;
                    }
                    Action::Motion(held)
                }
            };
            if let Some(bytes) = mouse::encode(
                action,
                col,
                row,
                modifiers,
                mode.contains(TermMode::SGR_MOUSE),
            ) {
                let _ = self.tx.send(Cmd::Input(bytes));
            }
            return;
        }

        let offset = emulator.grid().display_offset();
        let point = viewport_to_point(offset, Point::new(row, Column(col)));
        let side = if right_half { Side::Right } else { Side::Left };
        match event {
            Pointer::Down if button == 0 => {
                self.held.set(Some(0));
                // A second press on the same cell selects the word, a third the line.
                let now = Instant::now();
                let clicks = match self.last_press.get() {
                    Some((at, c, r, count))
                        if (c, r) == (col, row) && now.duration_since(at) < MULTI_CLICK =>
                    {
                        count % 3 + 1
                    }
                    _ => 1,
                };
                self.last_press.set(Some((now, col, row, clicks)));
                let kind = match clicks {
                    2 => SelectionType::Semantic,
                    3 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                emulator.selection = Some(Selection::new(kind, point, side));
            }
            Pointer::Word => {
                emulator.selection = Some(Selection::new(SelectionType::Semantic, point, side));
            }
            Pointer::Move if self.held.get() == Some(0) => match &mut emulator.selection {
                Some(selection) => selection.update(point, side),
                None => return,
            },
            Pointer::Up if button == 0 => {
                self.held.set(None);
                if emulator
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.is_empty())
                {
                    emulator.selection = None;
                }
            }
            _ => return,
        }
        drop(core);
        self.redraw_all.set(true);
        self.shared.mark();
    }

    /// The selected text, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.core
            .lock()
            .unwrap()
            .emulator
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }
}
