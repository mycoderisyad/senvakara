//! Starting, feeding, resizing and closing a session, with the PTY reader and writer threads.

use std::cell::Cell;
use std::io::Read;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alacritty_terminal::term::{Config, Term as Emulator};
use alacritty_terminal::vte::ansi::Processor;
use anyhow::{Context, Result};
use senvakara_adapters_pty::{PtySession, ShellKind};
use slint::VecModel;

use crate::TermLive;

use super::listener::Listener;
use super::{Cmd, Core, Grid, Session, Shared};

const SCROLLBACK: usize = 5000;

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
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Close);
    }
}
