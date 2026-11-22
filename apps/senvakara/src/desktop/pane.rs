//! The desktop pane the UI thread owns: starting the session, taking updates, sending input.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use slint::{Rgba8Pixel, SharedPixelBuffer};
use tokio::sync::mpsc;

use super::input::{button_bit, capitalised, ctrl_alt_del, keysym, other_case};
use super::session::serve;
use super::{Cmd, OpenGuard, Phase, Shared};

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
