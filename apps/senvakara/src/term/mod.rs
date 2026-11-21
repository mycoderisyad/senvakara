//! One terminal session: a local PTY, an `alacritty_terminal` emulator and the row model the
//! Slint view draws.
//!
//! Threads: a reader thread feeds PTY output into the emulator and asks the UI thread for one
//! refresh at a time; a writer thread owns the PTY and applies input and resizes in order. The
//! row model is only ever touched on the UI thread.

mod keys;
mod mouse;
mod palette;

use mouse::Action;
pub use mouse::Modifiers;

use std::cell::Cell;
use std::io::Read;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{viewport_to_point, Config, Term as Emulator, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use anyhow::{Context, Result};
use senvakara_adapters_pty::{PtySession, ShellKind};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{TermLive, TermRow, TermRun};

const SCROLLBACK: usize = 5000;
/// Presses on the same cell closer together than this count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

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

struct Listener {
    tx: mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
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

/// What the view needs besides the rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub col: i32,
    pub row: i32,
    pub visible: bool,
    pub scrollback: i32,
}

/// A pointer event from the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Down,
    Up,
    Move,
    /// The toolkit recognised a double click: select the word under the pointer.
    Word,
}

/// Feeds output into a [`Session`] from any thread.
#[derive(Clone)]
pub struct Feeder {
    core: Arc<Mutex<Core>>,
    shared: Arc<Shared>,
}

impl Feeder {
    pub fn feed(&self, bytes: &[u8]) {
        let mut core = self.core.lock().unwrap();
        let Core { emulator, parser } = &mut *core;
        parser.advance(emulator, bytes);
        drop(core);
        self.shared.mark();
    }

    /// The far end is gone.
    pub fn end(&self) {
        self.shared.exited.store(true, Ordering::Release);
        self.shared.dirty.store(true, Ordering::Release);
        (self.shared.notify)();
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

impl Session {
    /// The emulator and row model, not yet attached to anything that produces output.
    fn build(
        id: i32,
        cols: u16,
        lines: u16,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> (Self, mpsc::Receiver<Cmd>) {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            dirty: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            title: Mutex::new(None),
            grid: AtomicU32::new((cols as u32) << 16 | lines as u32),
            notify: Box::new(notify),
        });
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        let grid = Grid {
            cols: cols as usize,
            lines: lines as usize,
        };
        let listener = Listener {
            tx: tx.clone(),
            shared: shared.clone(),
        };
        let core = Arc::new(Mutex::new(Core {
            emulator: Emulator::new(config, &grid, listener),
            parser: Processor::new(),
        }));
        let session = Self {
            id,
            core,
            shared,
            tx,
            rows: Rc::new(VecModel::default()),
            live: Rc::new(VecModel::from(vec![TermLive::default()])),
            size: Cell::new((cols, lines)),
            held: Cell::new(None),
            redraw_all: Cell::new(false),
            last_press: Cell::new(None),
        };
        (session, rx)
    }

    /// A session whose bytes come from elsewhere (an SSH channel). The caller feeds output
    /// through the [`Feeder`] and applies the commands arriving on the receiver, in order.
    pub fn remote(
        id: i32,
        cols: u16,
        lines: u16,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> (Self, Feeder, mpsc::Receiver<Cmd>) {
        let (session, rx) = Self::build(id, cols.max(2), lines.max(1), notify);
        let feeder = Feeder {
            core: session.core.clone(),
            shared: session.shared.clone(),
        };
        (session, feeder, rx)
    }

    /// Start `kind` in a new PTY. `notify` is called from background threads and must only
    /// queue a call to [`Session::sync`] on the UI thread.
    pub fn spawn(
        id: i32,
        kind: &ShellKind,
        cols: u16,
        lines: u16,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        let cols = cols.max(2);
        let lines = lines.max(1);
        let pty = PtySession::spawn(kind, &[], cols, lines)?;
        let mut reader = pty.reader()?;
        let (session, rx) = Self::build(id, cols, lines, notify);
        let core = session.core.clone();
        let shared = session.shared.clone();

        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn({
                let core = core.clone();
                let shared = shared.clone();
                move || {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let mut core = core.lock().unwrap();
                                let Core { emulator, parser } = &mut *core;
                                parser.advance(emulator, &buf[..n]);
                                drop(core);
                                shared.mark();
                            }
                        }
                    }
                    shared.exited.store(true, Ordering::Release);
                    shared.dirty.store(true, Ordering::Release);
                    (shared.notify)();
                }
            })
            .context("start pty reader")?;

        std::thread::Builder::new()
            .name(format!("pty-writer-{id}"))
            .spawn({
                let shared = shared.clone();
                move || {
                    let mut pty = pty;
                    loop {
                        match rx.recv_timeout(Duration::from_millis(750)) {
                            Ok(Cmd::Input(bytes)) => {
                                if pty.write_input(&bytes).is_err() {
                                    break;
                                }
                            }
                            Ok(Cmd::Resize(cols, lines)) => {
                                let _ = pty.resize(cols, lines);
                            }
                            Ok(Cmd::Close) | Err(RecvTimeoutError::Disconnected) => break,
                            // ConPTY keeps the pipe open after the shell exits, so the reader
                            // never sees EOF on its own: check the child here.
                            Err(RecvTimeoutError::Timeout) => {
                                if matches!(pty.try_exit_code(), Ok(Some(_))) {
                                    break;
                                }
                            }
                        }
                    }
                    shared.exited.store(true, Ordering::Release);
                    shared.dirty.store(true, Ordering::Release);
                    (shared.notify)();
                    // Dropping the PTY kills a child that is still running and ends the reader.
                }
            })
            .context("start pty writer")?;

        Ok(session)
    }

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

    pub fn resize(&self, cols: u16, lines: u16) {
        let cols = cols.max(2);
        let lines = lines.max(1);
        if self.size.replace((cols, lines)) == (cols, lines) {
            return;
        }
        self.shared
            .grid
            .store((cols as u32) << 16 | lines as u32, Ordering::Relaxed);
        self.core.lock().unwrap().emulator.resize(Grid {
            cols: cols as usize,
            lines: lines as usize,
        });
        let _ = self.tx.send(Cmd::Resize(cols, lines));
    }

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

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Close);
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
