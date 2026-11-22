//! The emulator's event listener: answers the queries a program makes and records its title.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};

use super::{palette, Cmd, Shared};

pub(super) struct Listener {
    pub(super) tx: mpsc::Sender<Cmd>,
    pub(super) shared: Arc<Shared>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Replies the program is waiting for. ConPTY blocks at startup until its
            // cursor-position query is answered (TESTING.md, POC-03).
            Event::PtyWrite(text) => {
                let _ = self.tx.send(Cmd::Input(text.into_bytes()));
            }
            Event::TextAreaSizeRequest(format) => {
                let grid = self.shared.grid.load(Ordering::Relaxed);
                let size = WindowSize {
                    num_cols: (grid >> 16) as u16,
                    num_lines: grid as u16,
                    cell_width: 8,
                    cell_height: 18,
                };
                let _ = self.tx.send(Cmd::Input(format(size).into_bytes()));
            }
            Event::ColorRequest(index, format) => {
                let colors = alacritty_terminal::term::color::Colors::default();
                let rgb = palette::query(index, &colors);
                let _ = self.tx.send(Cmd::Input(format(rgb).into_bytes()));
            }
            Event::Title(title) => {
                *self.shared.title.lock().unwrap() = Some(title);
                self.shared.mark();
            }
            _ => {}
        }
    }
}
