//! One remote desktop pane: the host's X11 session through x11vnc, reached over the SSH
//! connection (`desktop_open` gives the tunnel and a one-time password) and shown by a native
//! VNC client. Nothing listens beyond the host's loopback, and closing the pane stops x11vnc.
//!
//! One framebuffer is kept per pane. The screen is copied out for the UI at most about thirty
//! times a second, only when something changed and only while the tab is on screen.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use slint::{Rgba8Pixel, SharedPixelBuffer};

mod framebuffer;
mod input;
mod pane;
mod session;

pub use pane::DesktopPane;

/// How often the screen is handed to the UI at most, and how often the host is asked for news.
const FRAME_GAP: Duration = Duration::from_millis(33);

/// Desktop sessions that still have to stop their x11vnc. The app waits for them on exit.
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// Wait until every closed pane has stopped its x11vnc, for at most `limit`.
pub fn wait_closed(limit: Duration) {
    let start = Instant::now();
    while OPEN.load(Ordering::Acquire) > 0 && start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct OpenGuard;

impl OpenGuard {
    fn new() -> Self {
        OPEN.fetch_add(1, Ordering::AcqRel);
        OpenGuard
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::AcqRel);
    }
}

// ------------------------------------------------------------------ pane

/// Input for the host, in the order the user made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    Pointer { x: u16, y: u16, mask: u8 },
    Key { sym: u32, down: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    /// What is shown: the desktop's name and its X display.
    Connected,
    /// Why it ended, in words ("" when the user closed it).
    Ended(String),
}

struct Shared {
    frame: Mutex<Option<SharedPixelBuffer<Rgba8Pixel>>>,
    /// A phase change the UI has not seen yet, and the header detail once it is known.
    phase: Mutex<Option<Phase>>,
    detail: Mutex<Option<String>>,
    dirty: AtomicBool,
    /// The tab is on screen: only then is the screen asked for and copied out.
    visible: AtomicBool,
    /// The tab just came on screen: hand over the screen even if nothing changed.
    want_frame: AtomicBool,
    closed: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn mark(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }

    fn set_phase(&self, phase: Phase) {
        *self.phase.lock().unwrap() = Some(phase);
        self.mark();
    }
}
