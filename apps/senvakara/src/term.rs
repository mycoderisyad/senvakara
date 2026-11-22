//! One terminal session: a local PTY, an `alacritty_terminal` emulator and the row model the
//! Slint view draws.
//!
//! Threads: a reader thread feeds PTY output into the emulator and asks the UI thread for one
//! refresh at a time; a writer thread owns the PTY and applies input and resizes in order. The
//! row model is only ever touched on the UI thread.

mod input;
mod keys;
mod lifecycle;
mod listener;
mod mouse;
mod palette;
mod view;

pub use input::Pointer;
pub use mouse::Modifiers;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term as Emulator;
use alacritty_terminal::vte::ansi::Processor;
use slint::VecModel;

use crate::{TermLive, TermRow};

use listener::Listener;

/// What the session asks of whatever carries its bytes, in order.
pub enum Cmd {
    Input(Vec<u8>),
    Resize(u16, u16),
    Close,
}

/// State shared between the session threads and the UI thread.
struct Shared {
    dirty: AtomicBool,
    exited: AtomicBool,
    title: Mutex<Option<String>>,
    /// Packed `cols << 16 | lines`, for size queries from programs.
    grid: AtomicU32,
    /// Asks the UI thread to call [`Session::sync`]. Must only queue work.
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    /// Request a refresh unless one is already pending.
    fn mark(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

struct Core {
    emulator: Emulator<Listener>,
    parser: Processor,
}

struct Grid {
    cols: usize,
    lines: usize,
}

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Session {
    pub id: i32,
    core: Arc<Mutex<Core>>,
    shared: Arc<Shared>,
    tx: mpsc::Sender<Cmd>,
    rows: Rc<VecModel<TermRow>>,
    /// One row: the cursor and the other things that change with every keystroke.
    live: Rc<VecModel<TermLive>>,
    size: Cell<(u16, u16)>,
    /// The mouse button currently held down in the view.
    held: Cell<Option<u8>>,
    /// Set when the selection changed: the next sync redraws every line.
    redraw_all: Cell<bool>,
    /// When and where the left button last went down, and the click count so far.
    last_press: Cell<Option<(Instant, usize, usize, u8)>>,
}
