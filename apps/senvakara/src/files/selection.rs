//! Selecting rows of a file list with the mouse, and the keys of a file list.

use std::collections::BTreeSet;
use std::rc::Rc;

use slint::platform::Key;

use crate::App;

impl App {
    pub(super) fn files_select(
        &self,
        id: i32,
        side: usize,
        row: usize,
        control: bool,
        shift: bool,
    ) {
        self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            if row >= state.entries.len() {
                return;
            }
            let mut selected = state.selected.clone();
            if shift {
                let from = state.anchor.unwrap_or(row);
                selected = (from.min(row)..=from.max(row)).collect();
            } else if control {
                if !selected.remove(&row) {
                    selected.insert(row);
                }
                state.anchor = Some(row);
            } else {
                selected = BTreeSet::from([row]);
                state.anchor = Some(row);
            }
            state.cursor = Some(row);
            state.set_selected(selected);
        });
        self.files_publish(id);
    }

    /// Keys of a file list: arrows, Enter, Backspace, Ctrl+A, F5.
    pub(super) fn files_key(
        self: &Rc<Self>,
        id: i32,
        side: usize,
        text: &str,
        control: bool,
        shift: bool,
    ) -> bool {
        let is = |key: Key| text.chars().eq([char::from(key)]);
        let (count, cursor) = self
            .with_files(id, |pane| {
                let state = pane.sides[side].borrow();
                (state.entries.len(), state.cursor)
            })
            .unwrap_or((0, None));
        let step = |delta: i64| -> usize {
            let last = count.saturating_sub(1) as i64;
            match cursor {
                Some(row) => (row as i64 + delta).clamp(0, last) as usize,
                None if delta > 0 => 0,
                None => last as usize,
            }
        };
        let target = if is(Key::DownArrow) {
            Some(step(1))
        } else if is(Key::UpArrow) {
            Some(step(-1))
        } else if is(Key::PageDown) {
            Some(step(10))
        } else if is(Key::PageUp) {
            Some(step(-10))
        } else if is(Key::Home) {
            Some(0)
        } else if is(Key::End) {
            Some(count.saturating_sub(1))
        } else {
            None
        };
        if let Some(row) = target {
            if count > 0 {
                self.files_select(id, side, row, false, shift);
            }
            return true;
        }
        if is(Key::Return) {
            if let Some(row) = cursor {
                self.files_open(id, side, row);
            }
            return true;
        }
        if is(Key::Backspace) {
            self.files_up(id, side);
            return true;
        }
        if is(Key::F5) {
            self.files_refresh(id, side);
            return true;
        }
        if control && matches!(text, "a" | "A" | "\u{1}") {
            self.with_files(id, |pane| {
                let mut state = pane.sides[side].borrow_mut();
                let all = (0..state.entries.len()).collect();
                state.set_selected(all);
            });
            self.files_publish(id);
            return true;
        }
        false
    }
}
