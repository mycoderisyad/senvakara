//! One Android screen pane: the scrcpy stream from the backend, decoded with OpenH264 (ADR-024)
//! on its own thread, and the input sent back.
//!
//! Only the newest decoded frame is kept: when the UI is behind, older frames are dropped.

mod held;

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use openh264::decoder::{DecodeOptions, Decoder, Flush};
use openh264::formats::YUVSource;
use senvakara_app::{Input, MirrorEvent, Sink};
use slint::platform::Key;
use slint::{Rgba8Pixel, SharedPixelBuffer};

use held::Held;

/// The touch pointer id the server sees (one finger).
const FINGER: u64 = 1;

struct Shared {
    frame: Mutex<Option<SharedPixelBuffer<Rgba8Pixel>>>,
    ended: Mutex<Option<String>>,
    dirty: AtomicBool,
    /// Decoded frames since the pane started.
    frames: AtomicU32,
    /// The backend's mirror id once the stream has started, -1 before.
    remote: AtomicI64,
    /// The pane was closed: stop as soon as possible.
    closed: AtomicBool,
    /// The pane is on screen. A hidden pane still decodes, but makes no RGBA pictures.
    visible: AtomicBool,
    /// The newest picture decoded while hidden.
    held: Mutex<Option<Held>>,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn mark(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

/// What the view should show now.
pub struct Update {
    pub frame: Option<SharedPixelBuffer<Rgba8Pixel>>,
    pub ended: Option<String>,
    pub fps: Option<u32>,
}

pub struct MirrorPane {
    pub id: i32,
    pub title: String,
    pub route: String,
    pub hue: i32,
    backend: Arc<senvakara_app::App>,
    shared: Arc<Shared>,
    /// Size of the last frame: touches must carry it or the server ignores them.
    size: Cell<(u16, u16)>,
    fps_mark: Cell<(Instant, u32)>,
    pub fps: Cell<u32>,
    pub error: std::cell::RefCell<String>,
    pub ended: Cell<bool>,
    /// The newest frame, for a view that starts showing this pane (a still screen sends none).
    pub last: std::cell::RefCell<Option<SharedPixelBuffer<Rgba8Pixel>>>,
}

impl MirrorPane {
    /// Start mirroring `route_id`. `notify` must only queue a refresh on the UI thread.
    pub fn start(
        id: i32,
        backend: Arc<senvakara_app::App>,
        route_id: String,
        title: String,
        route: String,
        hue: i32,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            ended: Mutex::new(None),
            dirty: AtomicBool::new(false),
            frames: AtomicU32::new(0),
            remote: AtomicI64::new(-1),
            closed: AtomicBool::new(false),
            visible: AtomicBool::new(true),
            held: Mutex::new(None),
            notify: Box::new(notify),
        });

        let (packets, inbox) = mpsc::channel::<MirrorEvent>();
        std::thread::Builder::new()
            .name(format!("mirror-decode-{id}"))
            .spawn({
                let shared = shared.clone();
                move || decode_loop(inbox, &shared)
            })
            .ok();

        let sink: Sink<MirrorEvent> = {
            let shared = shared.clone();
            Arc::new(move |event| {
                !shared.closed.load(Ordering::Acquire) && packets.send(event).is_ok()
            })
        };
        backend.rt.spawn({
            let backend = backend.clone();
            let shared = shared.clone();
            async move {
                match backend.mirror_start(route_id, sink).await {
                    Ok(started) => {
                        shared.remote.store(started.id as i64, Ordering::Release);
                        if shared.closed.load(Ordering::Acquire) {
                            backend.mirror_stop(started.id);
                        }
                    }
                    Err(error) => {
                        *shared.ended.lock().unwrap() = Some(crate::inventory::friendly(&error));
                        shared.mark();
                    }
                }
            }
        });

        Self {
            id,
            title,
            route,
            hue,
            backend,
            shared,
            size: Cell::new((0, 0)),
            fps_mark: Cell::new((Instant::now(), 0)),
            fps: Cell::new(0),
            error: Default::default(),
            ended: Cell::new(false),
            last: Default::default(),
        }
    }

    /// The pane came on screen or left it. While it is hidden no picture is converted or
    /// uploaded; coming back shows the newest picture the phone sent in the meantime.
    pub fn set_visible(&self, visible: bool) {
        if self.shared.visible.swap(visible, Ordering::AcqRel) == visible || !visible {
            return;
        }
        let shared = self.shared.clone();
        // The conversion is the slow part and must not hold up the UI thread.
        std::thread::Builder::new()
            .name(format!("mirror-resume-{}", self.id))
            .spawn(move || {
                let Some(held) = shared.held.lock().unwrap().take() else {
                    return;
                };
                let picture = held.to_rgba();
                let mut frame = shared.frame.lock().unwrap();
                // A picture decoded since the pane came back is newer: keep it.
                if frame.is_none() {
                    *frame = Some(picture);
                    drop(frame);
                    shared.mark();
                }
            })
            .ok();
    }

    pub fn size(&self) -> (u16, u16) {
        self.size.get()
    }

    /// Take what changed since the last call. UI thread only.
    pub fn take(&self) -> Update {
        self.shared.dirty.store(false, Ordering::Release);
        let frame = self.shared.frame.lock().unwrap().take();
        if let Some(frame) = &frame {
            self.size.set((frame.width() as u16, frame.height() as u16));
            *self.last.borrow_mut() = Some(frame.clone());
        }
        let ended = self.shared.ended.lock().unwrap().take();
        if let Some(reason) = &ended {
            self.ended.set(true);
            *self.error.borrow_mut() = reason.clone();
        }
        let (since, count) = self.fps_mark.get();
        let now = self.shared.frames.load(Ordering::Relaxed);
        let fps = (since.elapsed().as_millis() >= 1000).then(|| {
            let fps = ((now - count) as f64 / since.elapsed().as_secs_f64()).round() as u32;
            self.fps_mark.set((Instant::now(), now));
            self.fps.set(fps);
            fps
        });
        Update { frame, ended, fps }
    }

    fn send(&self, input: Input) {
        let remote = self.shared.remote.load(Ordering::Acquire);
        if remote >= 0 && !self.ended.get() {
            let _ = self.backend.mirror_input(remote as u32, input);
        }
    }

    /// action: 0 down, 1 up, 2 move. `x`, `y` are fractions of the frame.
    pub fn touch(&self, action: u8, x: f32, y: f32) {
        let (width, height) = self.size.get();
        if width == 0 {
            return;
        }
        self.send(Input::Touch {
            action,
            pointer_id: FINGER,
            x: (x.clamp(0.0, 1.0) * width as f32).round() as i32,
            y: (y.clamp(0.0, 1.0) * height as f32).round() as i32,
            width,
            height,
            pressure: if action == 1 { 0.0 } else { 1.0 },
        });
    }

    pub fn wheel(&self, x: f32, y: f32, up: bool) {
        let (width, height) = self.size.get();
        if width == 0 {
            return;
        }
        self.send(Input::Scroll {
            x: (x.clamp(0.0, 1.0) * width as f32).round() as i32,
            y: (y.clamp(0.0, 1.0) * height as f32).round() as i32,
            width,
            height,
            h: 0.0,
            v: if up { 1.0 } else { -1.0 },
        });
    }

    fn press(&self, keycode: i32) {
        for action in [0, 1] {
            self.send(Input::Key {
                action,
                keycode,
                meta_state: 0,
            });
        }
    }

    /// A key typed while the screen has focus. Returns false when it is not forwarded.
    pub fn key(&self, text: &str, control: bool, alt: bool) -> bool {
        if control || alt {
            return false;
        }
        // Android key codes for the keys that are not text.
        const KEYS: [(Key, i32); 13] = [
            (Key::Backspace, 67),
            (Key::Return, 66),
            (Key::Tab, 61),
            (Key::Escape, 111),
            (Key::Delete, 112),
            (Key::UpArrow, 19),
            (Key::DownArrow, 20),
            (Key::LeftArrow, 21),
            (Key::RightArrow, 22),
            (Key::Home, 122),
            (Key::End, 123),
            (Key::PageUp, 92),
            (Key::PageDown, 93),
        ];
        let mut chars = text.chars();
        let (Some(first), None) = (chars.next(), chars.next()) else {
            return false;
        };
        if let Some((_, code)) = KEYS.iter().find(|(key, _)| char::from(*key) == first) {
            self.press(*code);
            return true;
        }
        if first.is_control() || ('\u{e000}'..='\u{f8ff}').contains(&first) {
            return false;
        }
        self.send(Input::Text {
            text: text.to_string(),
        });
        true
    }

    /// One of the phone's own buttons.
    pub fn tool(&self, id: &str) {
        match id {
            "back" => self.press(4),
            "home" => self.press(3),
            "recents" => self.press(187),
            "power" => self.press(26),
            "volume-down" => self.press(25),
            "volume-up" => self.press(24),
            "notifications" => self.send(Input::ExpandNotifications),
            "rotate" => self.send(Input::Rotate),
            _ => {}
        }
    }
}

impl Drop for MirrorPane {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        let remote = self.shared.remote.load(Ordering::Acquire);
        if remote >= 0 {
            self.backend.mirror_stop(remote as u32);
        }
    }
}

fn decode_loop(inbox: mpsc::Receiver<MirrorEvent>, shared: &Shared) {
    let mut decoder = match Decoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            *shared.ended.lock().unwrap() =
                Some(format!("The video decoder could not start. ({error})"));
            shared.mark();
            return;
        }
    };
    for event in inbox {
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        match event {
            MirrorEvent::Config(_) => {}
            MirrorEvent::Packet { data, .. } => {
                // One packet is one access unit (parameter sets are already prepended to key
                // frames by the adapter): decode it whole, without the flush that makes the
                // decoder lose its place between frames.
                let result = decoder.decode_with_options(
                    &data,
                    DecodeOptions::new().flush_after_decode(Flush::NoFlush),
                );
                let mut produced = None;
                // Errors before the first key frame are expected: keep going.
                if let Ok(Some(picture)) = result {
                    // Checked under the lock `set_visible` takes the held picture with, so a
                    // picture is either kept for it or converted here.
                    let mut held = shared.held.lock().unwrap();
                    if shared.visible.load(Ordering::Acquire) {
                        drop(held);
                        let (width, height) = picture.dimensions();
                        let mut buffer =
                            SharedPixelBuffer::<Rgba8Pixel>::new(width as u32, height as u32);
                        picture.write_rgba8(buffer.make_mut_bytes());
                        produced = Some(buffer);
                    } else {
                        Held::keep(&mut held, &picture);
                    }
                }
                if let Some(buffer) = produced {
                    *shared.frame.lock().unwrap() = Some(buffer);
                    shared.frames.fetch_add(1, Ordering::Relaxed);
                    shared.mark();
                }
            }
            MirrorEvent::Ended(reason) => {
                *shared.ended.lock().unwrap() = Some(reason);
                shared.mark();
                break;
            }
        }
    }
}
