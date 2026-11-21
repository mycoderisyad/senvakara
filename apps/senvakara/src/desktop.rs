//! One remote desktop pane: the host's X11 session through x11vnc, reached over the SSH
//! connection (`desktop_open` gives the tunnel and a one-time password) and shown by a native
//! VNC client. Nothing listens beyond the host's loopback, and closing the pane stops x11vnc.
//!
//! One framebuffer is kept per pane. The screen is copied out for the UI at most about thirty
//! times a second, only when something changed and only while the tab is on screen.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use senvakara_app::VncTarget;
use slint::platform::Key;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use tokio::sync::mpsc;
use vnc::{PixelFormat, VncConnector, VncEncoding, VncEvent, X11Event};

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

// --------------------------------------------------------------- keysyms

const SHIFT_L: u32 = 0xffe1;
const CONTROL_L: u32 = 0xffe3;
const ALT_L: u32 = 0xffe9;
const DELETE: u32 = 0xffff;

/// The X11 keysym for a key as Slint reports it, or None for a key X11 has no use for.
pub fn keysym(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    let (first, None) = (chars.next()?, chars.next()) else {
        return None;
    };
    const NAMED: [(Key, u32); 40] = [
        (Key::Backspace, 0xff08),
        (Key::Tab, 0xff09),
        (Key::Backtab, 0xfe20),
        (Key::Return, 0xff0d),
        (Key::Escape, 0xff1b),
        (Key::Delete, DELETE),
        (Key::Home, 0xff50),
        (Key::LeftArrow, 0xff51),
        (Key::UpArrow, 0xff52),
        (Key::RightArrow, 0xff53),
        (Key::DownArrow, 0xff54),
        (Key::PageUp, 0xff55),
        (Key::PageDown, 0xff56),
        (Key::End, 0xff57),
        (Key::Insert, 0xff63),
        (Key::Menu, 0xff67),
        (Key::Pause, 0xff13),
        (Key::ScrollLock, 0xff14),
        (Key::SysReq, 0xff15),
        (Key::Shift, SHIFT_L),
        (Key::ShiftR, 0xffe2),
        (Key::Control, CONTROL_L),
        (Key::ControlR, 0xffe4),
        (Key::CapsLock, 0xffe5),
        (Key::Alt, ALT_L),
        (Key::AltGr, 0xfe03),
        (Key::Meta, 0xffeb),
        (Key::MetaR, 0xffec),
        (Key::F1, 0xffbe),
        (Key::F2, 0xffbf),
        (Key::F3, 0xffc0),
        (Key::F4, 0xffc1),
        (Key::F5, 0xffc2),
        (Key::F6, 0xffc3),
        (Key::F7, 0xffc4),
        (Key::F8, 0xffc5),
        (Key::F9, 0xffc6),
        (Key::F10, 0xffc7),
        (Key::F11, 0xffc8),
        (Key::F12, 0xffc9),
    ];
    if let Some((_, sym)) = NAMED.iter().find(|(key, _)| char::from(*key) == first) {
        return Some(*sym);
    }
    let code = first as u32;
    match code {
        // A letter typed with Control held can arrive as its control character.
        0x01..=0x1a => Some(code + 0x60),
        // Printable Latin-1 has the same number as a keysym.
        0x20..=0x7e | 0xa0..=0xff => Some(code),
        // Other control codes, and Slint's own codes for keys it has no text for.
        0x00 | 0x1b..=0x1f | 0x7f..=0x9f | 0xe000..=0xf8ff => None,
        // Any other character: the Unicode keysym range.
        _ => Some(0x0100_0000 + code),
    }
}

/// The same letter key in the other case: a key released after Shift arrives lowercase.
fn other_case(sym: u32) -> Option<u32> {
    let letter = char::from_u32(sym).filter(char::is_ascii_alphabetic)?;
    Some(if letter.is_ascii_lowercase() {
        letter.to_ascii_uppercase()
    } else {
        letter.to_ascii_lowercase()
    } as u32)
}

/// Key presses and releases for Ctrl+Alt+Del, in order.
pub fn ctrl_alt_del() -> [(u32, bool); 6] {
    [
        (CONTROL_L, true),
        (ALT_L, true),
        (DELETE, true),
        (DELETE, false),
        (ALT_L, false),
        (CONTROL_L, false),
    ]
}

/// A desktop name as the host reports it ("xfce"), with a capital to start the header line.
fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The VNC button mask bit of a mouse button (0 left, 1 middle, 2 right).
fn button_bit(button: i32) -> u8 {
    match button {
        1 => 2,
        2 => 4,
        _ => 1,
    }
}

// ----------------------------------------------------------- framebuffer

/// The host's screen as RGBA, kept up to date from the rectangles the server sends.
#[derive(Default)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Framebuffer {
    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.rgba.clear();
        // Opaque black until the server has painted it.
        self.rgba.resize(width * height * 4, 0);
        for pixel in self.rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }

    /// Paint `data` (RGBX rows) at `rect`. Returns false when it does not fit the screen.
    pub fn draw(&mut self, rect: vnc::Rect, data: &[u8]) -> bool {
        let (x, y, w, h) = (
            rect.x as usize,
            rect.y as usize,
            rect.width as usize,
            rect.height as usize,
        );
        if x + w > self.width || y + h > self.height || data.len() < w * h * 4 {
            return false;
        }
        for row in 0..h {
            let from = row * w * 4;
            let to = ((y + row) * self.width + x) * 4;
            let line = &mut self.rgba[to..to + w * 4];
            line.copy_from_slice(&data[from..from + w * 4]);
            // The fourth byte is padding on the wire; the picture is opaque.
            for pixel in line.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        true
    }

    /// Move the pixels of `src` to `dst` (same size), as CopyRect asks.
    pub fn copy(&mut self, dst: vnc::Rect, src: vnc::Rect) -> bool {
        let (w, h) = (src.width as usize, src.height as usize);
        let (sx, sy, dx, dy) = (
            src.x as usize,
            src.y as usize,
            dst.x as usize,
            dst.y as usize,
        );
        if sx + w > self.width
            || sy + h > self.height
            || dx + w > self.width
            || dy + h > self.height
        {
            return false;
        }
        // Rows are moved in the order that never overwrites a row still to be read.
        let rows: Box<dyn Iterator<Item = usize>> = if dy > sy {
            Box::new((0..h).rev())
        } else {
            Box::new(0..h)
        };
        for row in rows {
            let from = ((sy + row) * self.width + sx) * 4;
            let to = ((dy + row) * self.width + dx) * 4;
            self.rgba.copy_within(from..from + w * 4, to);
        }
        true
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

/// What the view should show now.
pub struct Update {
    pub frame: Option<SharedPixelBuffer<Rgba8Pixel>>,
    pub changed: bool,
}

pub struct DesktopPane {
    pub id: i32,
    /// The host's name.
    pub title: String,
    pub hue: i32,
    shared: Arc<Shared>,
    input: mpsc::UnboundedSender<Cmd>,
    pub phase: RefCell<Phase>,
    /// "Xfce on display :0, through the SSH tunnel, one-time password"
    pub detail: RefCell<String>,
    pub view_only: Cell<bool>,
    size: Cell<(u16, u16)>,
    /// The picture last handed to the view, for when the tab is shown again.
    pub last: RefCell<Option<SharedPixelBuffer<Rgba8Pixel>>>,
    /// Mouse buttons and keys held down on the host.
    buttons: Cell<u8>,
    pointer: Cell<(u16, u16)>,
    pressed: RefCell<Vec<u32>>,
}

impl DesktopPane {
    /// Start a desktop session on `host_id`. `notify` must only queue a refresh on the UI thread.
    pub fn start(
        id: i32,
        backend: Arc<senvakara_app::App>,
        host_id: String,
        title: String,
        hue: i32,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            phase: Mutex::new(None),
            detail: Mutex::new(None),
            dirty: AtomicBool::new(false),
            visible: AtomicBool::new(false),
            want_frame: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            notify: Box::new(notify),
        });
        let (input, commands) = mpsc::unbounded_channel();
        let guard = OpenGuard::new();
        backend.rt.spawn({
            let backend = backend.clone();
            let shared = shared.clone();
            async move {
                let _guard = guard;
                let target = match backend.desktop_open(host_id).await {
                    Ok(target) => target,
                    Err(error) => {
                        shared.set_phase(Phase::Ended(crate::inventory::friendly(&error)));
                        return;
                    }
                };
                *shared.detail.lock().unwrap() = Some(format!(
                    "{} on display {}, through the SSH tunnel, one-time password",
                    capitalised(target.desktop.as_deref().unwrap_or("Desktop")),
                    target.display
                ));
                shared.mark();
                let outcome = if shared.closed.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    serve(&target, &shared, commands).await
                };
                // Whatever happened, stop x11vnc: nothing may keep serving the screen.
                let _ = backend.desktop_close(target.id).await;
                if !shared.closed.load(Ordering::Acquire) {
                    shared.set_phase(Phase::Ended(match outcome {
                        Ok(()) => String::new(),
                        Err(reason) => reason,
                    }));
                }
            }
        });
        Self {
            id,
            title,
            hue,
            shared,
            input,
            phase: RefCell::new(Phase::Starting),
            detail: RefCell::new("Through the SSH tunnel, one-time password".into()),
            view_only: Cell::new(false),
            size: Cell::new((0, 0)),
            last: RefCell::new(None),
            buttons: Cell::new(0),
            pointer: Cell::new((0, 0)),
            pressed: RefCell::new(Vec::new()),
        }
    }

    pub fn ended(&self) -> bool {
        matches!(*self.phase.borrow(), Phase::Ended(_))
    }

    /// The tab came on screen or left it.
    pub fn set_visible(&self, visible: bool) {
        if self.shared.visible.swap(visible, Ordering::AcqRel) == visible {
            return;
        }
        if visible {
            self.shared.want_frame.store(true, Ordering::Release);
        } else {
            // Keys held while the tab goes away would stay down on the host.
            self.release_all();
        }
    }

    /// Take what changed since the last call. UI thread only.
    pub fn take(&self) -> Update {
        self.shared.dirty.store(false, Ordering::Release);
        let frame = self.shared.frame.lock().unwrap().take();
        if let Some(frame) = &frame {
            self.size.set((frame.width() as u16, frame.height() as u16));
            *self.last.borrow_mut() = Some(frame.clone());
        }
        let mut changed = false;
        if let Some(detail) = self.shared.detail.lock().unwrap().take() {
            *self.detail.borrow_mut() = detail;
            changed = true;
        }
        if let Some(phase) = self.shared.phase.lock().unwrap().take() {
            *self.phase.borrow_mut() = phase;
            changed = true;
        }
        Update { frame, changed }
    }

    fn live(&self) -> bool {
        *self.phase.borrow() == Phase::Connected && !self.view_only.get()
    }

    fn send(&self, command: Cmd) {
        let _ = self.input.send(command);
    }

    fn at(&self, x: f32, y: f32) -> (u16, u16) {
        let (width, height) = self.size.get();
        let place = |fraction: f32, size: u16| {
            ((fraction.clamp(0.0, 1.0) * size as f32) as u16).min(size.saturating_sub(1))
        };
        (place(x, width), place(y, height))
    }

    /// kind: 0 down, 1 up, 2 move. button: 0 left, 1 middle, 2 right. `x`, `y` are fractions
    /// of the screen.
    pub fn pointer(&self, kind: i32, button: i32, x: f32, y: f32) {
        if !self.live() || self.size.get().0 == 0 {
            return;
        }
        let mut mask = self.buttons.get();
        match kind {
            0 => mask |= button_bit(button),
            1 => mask &= !button_bit(button),
            _ => {}
        }
        self.buttons.set(mask);
        let (x, y) = self.at(x, y);
        self.pointer.set((x, y));
        self.send(Cmd::Pointer { x, y, mask });
    }

    pub fn wheel(&self, x: f32, y: f32, up: bool) {
        if !self.live() || self.size.get().0 == 0 {
            return;
        }
        let (x, y) = self.at(x, y);
        let mask = self.buttons.get();
        // Wheel steps are buttons 4 (up) and 5 (down), pressed and released.
        let step = if up { 8 } else { 16 };
        self.send(Cmd::Pointer {
            x,
            y,
            mask: mask | step,
        });
        self.send(Cmd::Pointer { x, y, mask });
    }

    /// A key went down or up while the screen has focus. Returns false when it is not sent.
    pub fn key(&self, text: &str, down: bool) -> bool {
        if !self.live() {
            return false;
        }
        let Some(sym) = keysym(text) else {
            return false;
        };
        let mut pressed = self.pressed.borrow_mut();
        if down {
            if !pressed.contains(&sym) {
                pressed.push(sym);
            }
            self.send(Cmd::Key { sym, down: true });
            return true;
        }
        // Release the key that went down, whatever Shift did to its letter since.
        let held = [Some(sym), other_case(sym)]
            .into_iter()
            .flatten()
            .find(|sym| pressed.contains(sym));
        if let Some(sym) = held {
            pressed.retain(|key| *key != sym);
            self.send(Cmd::Key { sym, down: false });
        }
        true
    }

    /// Let go of every key and mouse button held on the host.
    pub fn release_all(&self) {
        for sym in self.pressed.borrow_mut().drain(..).rev() {
            self.send(Cmd::Key { sym, down: false });
        }
        if self.buttons.replace(0) != 0 {
            let (x, y) = self.pointer.get();
            self.send(Cmd::Pointer { x, y, mask: 0 });
        }
    }

    /// Stop or resume sending the user's input. The picture keeps updating either way.
    pub fn set_view_only(&self, on: bool) {
        if on {
            self.release_all();
        }
        self.view_only.set(on);
    }

    pub fn send_ctrl_alt_del(&self) {
        if !self.live() {
            return;
        }
        self.release_all();
        for (sym, down) in ctrl_alt_del() {
            self.send(Cmd::Key { sym, down });
        }
    }
}

impl Drop for DesktopPane {
    fn drop(&mut self) {
        // The session task sees this (and the closed input channel), leaves its loop and
        // calls `desktop_close`.
        self.shared.closed.store(true, Ordering::Release);
    }
}

/// Runs the VNC session until the pane closes (Ok) or the connection ends (Err: why, in words).
async fn serve(
    target: &VncTarget,
    shared: &Shared,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
) -> Result<(), String> {
    let lost = |error: &dyn std::fmt::Display| {
        format!("The connection to the desktop was lost. ({error})")
    };
    // The session is over on the other side: x11vnc stopped or the SSH connection went away.
    let ended = |error: vnc::VncError| match error {
        vnc::VncError::ClientNotRunning => {
            "The desktop session ended: the host closed it or the SSH connection was lost."
                .to_string()
        }
        other => lost(&other),
    };
    let stream = target
        .conn
        .open_tcp("127.0.0.1", target.vnc_port)
        .await
        .map_err(|error| format!("The SSH tunnel to the desktop could not be opened. ({error})"))?;
    let password = target.password.clone();
    let start =
        |error: vnc::VncError| format!("The desktop did not accept the connection. ({error})");
    let vnc = VncConnector::new(stream)
        .set_auth_method(async move { Ok(password) })
        .add_encoding(VncEncoding::Tight)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::Raw)
        .allow_shared(true)
        .set_pixel_format(PixelFormat::rgba())
        .build()
        .map_err(start)?
        .try_start()
        .await
        .map_err(start)?
        .finish()
        .map_err(start)?;
    shared.set_phase(Phase::Connected);

    let mut screen = Framebuffer::default();
    let mut changed = false;
    let mut asked = Instant::now() - FRAME_GAP;
    let mut copied = asked;
    let outcome = loop {
        if shared.closed.load(Ordering::Acquire) {
            break Ok(());
        }
        // Apply what the server sent, without letting a flood starve the input.
        let burst = Instant::now();
        let failed = loop {
            match vnc.poll_event().await {
                Ok(Some(event)) => match event {
                    VncEvent::SetResolution(size) => {
                        screen.resize(size.width as usize, size.height as usize);
                        changed = true;
                    }
                    VncEvent::RawImage(rect, data) => changed |= screen.draw(rect, &data),
                    VncEvent::Copy(dst, src) => changed |= screen.copy(dst, src),
                    VncEvent::Error(message) => break Some(lost(&message)),
                    _ => {}
                },
                Ok(None) => break None,
                Err(error) => break Some(ended(error)),
            }
            if burst.elapsed() > Duration::from_millis(12) {
                break None;
            }
        };
        if let Some(reason) = failed {
            break Err(reason);
        }

        let visible = shared.visible.load(Ordering::Acquire);
        if visible && asked.elapsed() >= FRAME_GAP {
            if let Err(error) = vnc.input(X11Event::Refresh).await {
                break Err(ended(error));
            }
            asked = Instant::now();
        }
        if visible && screen.width > 0 && copied.elapsed() >= FRAME_GAP {
            let wanted = shared.want_frame.load(Ordering::Acquire);
            let mut slot = shared.frame.lock().unwrap();
            // One copy waits at most: while the UI has not taken it, the screen stays here.
            if (changed || wanted) && slot.is_none() {
                *slot = Some(SharedPixelBuffer::clone_from_slice(
                    &screen.rgba,
                    screen.width as u32,
                    screen.height as u32,
                ));
                drop(slot);
                shared.want_frame.store(false, Ordering::Release);
                changed = false;
                copied = Instant::now();
                shared.mark();
            }
        }

        // Sleep until the user does something, but not for long: the server is polled.
        let nap = Duration::from_millis(if visible { 8 } else { 100 });
        let first = match tokio::time::timeout(nap, commands.recv()).await {
            Ok(Some(command)) => command,
            Ok(None) => break Ok(()),
            Err(_) => continue,
        };
        let mut batch = vec![first];
        while let Ok(command) = commands.try_recv() {
            // Of several pointer moves in a row only the last position matters.
            match (batch.last_mut(), command) {
                (
                    Some(Cmd::Pointer { x, y, mask }),
                    Cmd::Pointer {
                        x: new_x,
                        y: new_y,
                        mask: new_mask,
                    },
                ) if *mask == new_mask => (*x, *y) = (new_x, new_y),
                (_, command) => batch.push(command),
            }
        }
        let mut failed = None;
        for command in batch {
            let event = match command {
                Cmd::Pointer { x, y, mask } => X11Event::PointerEvent((x, y, mask).into()),
                Cmd::Key { sym, down } => X11Event::KeyEvent((sym, down).into()),
            };
            if let Err(error) = vnc.input(event).await {
                failed = Some(ended(error));
                break;
            }
        }
        if let Some(reason) = failed {
            break Err(reason);
        }
    };
    let _ = vnc.close().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key) -> String {
        char::from(key).to_string()
    }

    #[test]
    fn printable_text_maps_to_its_own_keysym() {
        assert_eq!(keysym("a"), Some(0x61));
        assert_eq!(keysym("A"), Some(0x41));
        assert_eq!(keysym(" "), Some(0x20));
        assert_eq!(keysym("~"), Some(0x7e));
        assert_eq!(keysym("é"), Some(0xe9));
        // Beyond Latin-1: Unicode keysyms.
        assert_eq!(keysym("€"), Some(0x0100_20ac));
        assert_eq!(keysym("ж"), Some(0x0100_0436));
        assert_eq!(keysym(""), None);
        assert_eq!(keysym("ab"), None);
    }

    #[test]
    fn named_keys_map_to_x11_function_keysyms() {
        assert_eq!(keysym(&key(Key::Return)), Some(0xff0d));
        assert_eq!(keysym(&key(Key::Backspace)), Some(0xff08));
        assert_eq!(keysym(&key(Key::Tab)), Some(0xff09));
        assert_eq!(keysym(&key(Key::Escape)), Some(0xff1b));
        assert_eq!(keysym(&key(Key::Delete)), Some(0xffff));
        assert_eq!(keysym(&key(Key::LeftArrow)), Some(0xff51));
        assert_eq!(keysym(&key(Key::DownArrow)), Some(0xff54));
        assert_eq!(keysym(&key(Key::PageUp)), Some(0xff55));
        assert_eq!(keysym(&key(Key::F1)), Some(0xffbe));
        assert_eq!(keysym(&key(Key::F12)), Some(0xffc9));
        assert_eq!(keysym(&key(Key::Shift)), Some(0xffe1));
        assert_eq!(keysym(&key(Key::Control)), Some(0xffe3));
        assert_eq!(keysym(&key(Key::Alt)), Some(0xffe9));
        assert_eq!(keysym(&key(Key::Meta)), Some(0xffeb));
    }

    #[test]
    fn control_characters_and_private_codes() {
        // Ctrl+C reported as its control character is still the letter c.
        assert_eq!(keysym("\u{3}"), Some(0x63));
        // A key Slint has no text for (media keys and the like).
        assert_eq!(keysym("\u{f8ff}"), None);
        assert_eq!(other_case(0x61), Some(0x41));
        assert_eq!(other_case(0x5a), Some(0x7a));
        assert_eq!(other_case(0xff0d), None);
    }

    #[test]
    fn ctrl_alt_del_presses_then_releases_in_reverse() {
        let keys = ctrl_alt_del();
        assert_eq!(keys[..3], [(0xffe3, true), (0xffe9, true), (0xffff, true)]);
        assert_eq!(
            keys[3..],
            [(0xffff, false), (0xffe9, false), (0xffe3, false)]
        );
        assert_eq!((button_bit(0), button_bit(1), button_bit(2)), (1, 2, 4));
        assert_eq!(capitalised("xfce"), "Xfce");
        assert_eq!(capitalised(""), "");
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> vnc::Rect {
        vnc::Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn rectangles_are_painted_opaque() {
        let mut screen = Framebuffer::default();
        screen.resize(4, 3);
        assert_eq!(screen.rgba.len(), 48);
        assert!(screen.rgba.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        // Two pixels with a zero padding byte, at (1, 1).
        assert!(screen.draw(rect(1, 1, 2, 1), &[10, 20, 30, 0, 40, 50, 60, 0]));
        let at = |screen: &Framebuffer, x: usize, y: usize| {
            let start = (y * screen.width + x) * 4;
            screen.rgba[start..start + 4].to_vec()
        };
        assert_eq!(at(&screen, 1, 1), [10, 20, 30, 255]);
        assert_eq!(at(&screen, 2, 1), [40, 50, 60, 255]);
        assert_eq!(at(&screen, 0, 1), [0, 0, 0, 255]);
        // Out of bounds or short data is refused, not painted.
        assert!(!screen.draw(rect(3, 0, 2, 1), &[0; 8]));
        assert!(!screen.draw(rect(0, 0, 2, 1), &[0; 4]));
        // CopyRect, overlapping downwards.
        assert!(screen.copy(rect(1, 2, 2, 1), rect(1, 1, 2, 1)));
        assert_eq!(at(&screen, 2, 2), [40, 50, 60, 255]);
        assert!(!screen.copy(rect(3, 2, 2, 1), rect(1, 1, 2, 1)));
        // The same size keeps the picture.
        screen.resize(4, 3);
        assert_eq!(at(&screen, 1, 1), [10, 20, 30, 255]);
    }
}
