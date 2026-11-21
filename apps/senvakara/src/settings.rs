//! Settings, Diagnostics and About. The values live in `settings.json` next to the database
//! (not in it), so a missing or broken file simply means the defaults.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use slint::ComponentHandle;

use crate::{App, Settings, Term};

pub const FILE_NAME: &str = "settings.json";
/// Seconds between health readings the user can choose from.
pub const INTERVALS: [u64; 4] = [2, 5, 10, 30];
/// Terminal font sizes the user can choose from, in px.
pub const FONT_SIZES: [u32; 7] = [11, 12, 13, 14, 15, 16, 18];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Values {
    pub metrics_seconds: u64,
    pub font_size: u32,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            metrics_seconds: 5,
            font_size: 13,
        }
    }
}

impl Values {
    /// Read the file's text. Anything missing, of the wrong type or not on offer is the
    /// default for that value; text that is not JSON at all is all defaults.
    pub fn parse(text: &str) -> Self {
        let json: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let defaults = Self::default();
        Self {
            metrics_seconds: json
                .get("metrics_interval_seconds")
                .and_then(Value::as_u64)
                .filter(|seconds| INTERVALS.contains(seconds))
                .unwrap_or(defaults.metrics_seconds),
            font_size: json
                .get("terminal_font_size")
                .and_then(Value::as_u64)
                .map(|size| size as u32)
                .filter(|size| FONT_SIZES.contains(size))
                .unwrap_or(defaults.font_size),
        }
    }

    pub fn to_text(self) -> String {
        let json = json!({
            "metrics_interval_seconds": self.metrics_seconds,
            "terminal_font_size": self.font_size,
        });
        serde_json::to_string_pretty(&json).unwrap_or_default() + "\n"
    }

    /// The stored values, or the defaults when there is no usable file.
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(FILE_NAME))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Write the file through a temporary one, so a crash never leaves half a file.
    pub fn save(self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temporary = dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&temporary, self.to_text())?;
        std::fs::rename(&temporary, dir.join(FILE_NAME))
    }
}

/// The result of the decoder self-test.
#[derive(Clone, Debug, PartialEq)]
pub struct DecoderReport {
    pub width: usize,
    pub height: usize,
    pub encoded: usize,
    pub decoded: usize,
    pub per_frame: Duration,
}

impl DecoderReport {
    /// Every frame came back, at the size that went in.
    pub fn passed(&self) -> bool {
        self.encoded > 0 && self.decoded == self.encoded
    }

    pub fn text(&self) -> String {
        let per_frame = self.per_frame.as_secs_f64() * 1000.0;
        if self.passed() {
            format!(
                "Decoded {} of {} frames at {}x{}, {per_frame:.1} ms per frame. A 30 fps phone screen leaves 33 ms per frame.",
                self.decoded, self.encoded, self.width, self.height
            )
        } else {
            format!(
                "Only {} of {} frames at {}x{} were decoded.",
                self.decoded, self.encoded, self.width, self.height
            )
        }
    }
}

/// One frame of a scrolling list: flat areas with sharp edges that shift every frame, which is
/// what a phone screen stream looks like to the codec.
fn test_frame(width: usize, height: usize, index: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];
    let scroll = index * 8;
    for y in 0..height {
        let row = (y + scroll) / 48;
        let in_row = (y + scroll) % 48;
        for x in 0..width {
            let mut pixel = if row.is_multiple_of(2) {
                [24u8, 28, 34]
            } else {
                [30, 35, 43]
            };
            if (14..22).contains(&in_row) && x > 48 && !(x / 4 + row * 7).is_multiple_of(5) {
                pixel = [228, 231, 236];
            }
            if (10..38).contains(&in_row) && (12..40).contains(&x) {
                pixel = [
                    (row * 53 % 160) as u8 + 30,
                    (row * 97 % 160) as u8 + 30,
                    180,
                ];
            }
            let at = (y * width + x) * 3;
            rgb[at..at + 3].copy_from_slice(&pixel);
        }
    }
    rgb
}

/// Encode a short synthetic clip with OpenH264 and decode it the way the mirror pane does
/// (decode, then convert to RGBA). Blocking: run it off the UI thread.
pub fn decoder_self_test(
    width: usize,
    height: usize,
    frames: usize,
) -> Result<DecoderReport, String> {
    use openh264::decoder::Decoder;
    use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate};
    use openh264::formats::{RgbSliceU8, YUVBuffer, YUVSource};
    use openh264::OpenH264API;

    let config = EncoderConfig::new()
        .bitrate(BitRate::from_bps(2_000_000))
        .max_frame_rate(FrameRate::from_hz(30.0))
        .skip_frames(false);
    let mut encoder =
        Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|error| {
            format!("The encoder for the test clip could not be started ({error}).")
        })?;
    let mut packets = Vec::with_capacity(frames);
    for index in 0..frames {
        let rgb = test_frame(width, height, index);
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (width, height)));
        let packet = encoder
            .encode(&yuv)
            .map_err(|error| format!("The test clip could not be encoded ({error})."))?
            .to_vec();
        packets.push(packet);
    }

    let mut decoder = Decoder::new()
        .map_err(|error| format!("The H.264 decoder could not be started ({error})."))?;
    let mut rgba = vec![0u8; width * height * 4];
    let mut decoded = 0;
    let started = Instant::now();
    for packet in &packets {
        let frame = decoder
            .decode(packet)
            .map_err(|error| format!("A frame could not be decoded ({error})."))?;
        if let Some(yuv) = frame {
            if yuv.dimensions() != (width, height) {
                return Err(format!(
                    "A frame came back as {}x{} instead of {width}x{height}.",
                    yuv.dimensions().0,
                    yuv.dimensions().1
                ));
            }
            yuv.write_rgba8(&mut rgba);
            decoded += 1;
        }
    }
    let elapsed = started.elapsed();
    Ok(DecoderReport {
        width,
        height,
        encoded: packets.len(),
        decoded,
        per_frame: elapsed / decoded.max(1) as u32,
    })
}

/// Where the third-party licence notices are, when this build ships them.
fn notices_file() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [
        folder.join("THIRD_PARTY_NOTICES.md"),
        folder.join("resources").join("THIRD_PARTY_NOTICES.md"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

#[derive(Default)]
pub struct State {
    values: Cell<Values>,
    testing: Cell<bool>,
    /// What draws the window, once the first frame has said so.
    renderer: RefCell<String>,
}

impl App {
    pub fn init_settings(self: &Rc<Self>, ui: &crate::MainWindow) {
        let values = Values::load(&self.backend.paths.data_dir);
        self.settings.values.set(values);
        let view = ui.global::<Settings>();
        view.set_version(env!("CARGO_PKG_VERSION").into());
        view.set_data_dir(self.backend.paths.data_dir.display().to_string().into());
        view.set_log_dir(self.backend.paths.log_dir.display().to_string().into());
        view.set_settings_file(
            self.backend
                .paths
                .data_dir
                .join(FILE_NAME)
                .display()
                .to_string()
                .into(),
        );
        match notices_file() {
            Some(path) => {
                view.set_notices(path.display().to_string().into());
                view.set_has_notices(true);
            }
            None => view.set_notices(
                "Not included in this development build. Releases ship THIRD_PARTY_NOTICES.md next to the program."
                    .into(),
            ),
        }
        view.set_renderer("Not known until the first frame is drawn".into());

        view.on_set_metrics_interval({
            let app = self.clone();
            move |text| {
                if let Some(seconds) = text.parse().ok().filter(|s| INTERVALS.contains(s)) {
                    app.settings_change(|values| values.metrics_seconds = seconds);
                }
            }
        });
        view.on_set_font_size({
            let app = self.clone();
            move |text| {
                if let Some(size) = text.parse().ok().filter(|s| FONT_SIZES.contains(s)) {
                    app.settings_change(|values| values.font_size = size);
                }
            }
        });
        view.on_open_folder({
            let app = self.clone();
            move |which| app.settings_open(&which)
        });
        view.on_test_decoder({
            let app = self.clone();
            move || app.decoder_test()
        });

        // The renderer tells which graphics API it draws with when it sets up.
        let weak = ui.as_weak();
        let noted = Cell::new(false);
        let notifier = ui.window().set_rendering_notifier(move |state, api| {
            if !matches!(state, slint::RenderingState::RenderingSetup) || noted.replace(true) {
                return;
            }
            let name = match api {
                slint::GraphicsAPI::NativeOpenGL { .. } => "OpenGL (Slint FemtoVG renderer)",
                slint::GraphicsAPI::WebGL { .. } => "WebGL",
                _ => "A GPU renderer Slint chose (not OpenGL)",
            }
            .to_string();
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    *app.settings.renderer.borrow_mut() = name.clone();
                }
                if let Some(ui) = weak.upgrade() {
                    ui.global::<Settings>().set_renderer(name.into());
                }
            });
        });
        // Only GPU renderers report a graphics API; the software renderer (the default) does not.
        if let Err(slint::SetRenderingNotifierError::Unsupported) = notifier {
            let name = "Software (Slint, drawn on the CPU)".to_string();
            *self.settings.renderer.borrow_mut() = name.clone();
            view.set_renderer(name.into());
        }

        self.settings_apply(ui);
    }

    /// Put the current values on screen and into the parts of the app that use them.
    fn settings_apply(self: &Rc<Self>, ui: &crate::MainWindow) {
        let values = self.settings.values.get();
        let view = ui.global::<Settings>();
        view.set_metrics_interval(values.metrics_seconds.to_string().into());
        view.set_font_size(values.font_size.to_string().into());
        ui.global::<Term>().set_font_size(values.font_size as f32);
        self.set_metrics_interval(Duration::from_secs(values.metrics_seconds));
    }

    fn settings_change(self: &Rc<Self>, change: impl FnOnce(&mut Values)) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut values = self.settings.values.get();
        change(&mut values);
        if values == self.settings.values.get() {
            return;
        }
        self.settings.values.set(values);
        self.settings_apply(&ui);
        let error = match values.save(&self.backend.paths.data_dir) {
            Ok(()) => String::new(),
            Err(error) => format!(
                "The change applies now but could not be saved for the next start: {error}."
            ),
        };
        ui.global::<Settings>().set_error(error.into());
    }

    /// The Settings section came on screen: read what may have changed since.
    pub fn settings_opened(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Settings>();
        match self.backend.adb_status().local_adb_path {
            Some(path) => {
                view.set_adb_tone("live".into());
                view.set_adb_text("Found".into());
                view.set_adb_detail(path.into());
            }
            None => {
                view.set_adb_tone("attention".into());
                view.set_adb_text("Not found".into());
                view.set_adb_detail(
                    "Install Android platform-tools and put adb on PATH to use phones on this PC."
                        .into(),
                );
            }
        }
    }

    fn settings_open(&self, which: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let path = match which {
            "data" => Some(self.backend.paths.data_dir.clone()),
            "logs" => Some(self.backend.paths.log_dir.clone()),
            // Show the file selected in its folder.
            "notices" => notices_file(),
            _ => None,
        };
        let Some(path) = path else { return };
        let mut command = std::process::Command::new("explorer.exe");
        if path.is_file() {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(&path);
        }
        // Explorer returns a non-zero code even when it opens the folder: only a failure to
        // start it is an error.
        if let Err(error) = command.spawn() {
            ui.global::<Settings>()
                .set_error(format!("The folder could not be opened: {error}.").into());
        }
    }

    fn decoder_test(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.settings.testing.replace(true) {
            return;
        }
        let view = ui.global::<Settings>();
        view.set_decoder_tone("busy".into());
        view.set_decoder_text("Encoding and decoding a short test clip.".into());
        view.set_decoder_reason("The test is running.".into());
        std::thread::spawn(|| {
            // A panic inside the codec must not take the app down or leave the test "running".
            let result = std::panic::catch_unwind(|| decoder_self_test(360, 640, 30))
                .unwrap_or_else(|_| Err("The decoder stopped unexpectedly.".into()));
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = crate::app() else { return };
                app.settings.testing.set(false);
                let Some(ui) = app.ui.upgrade() else { return };
                let view = ui.global::<Settings>();
                let (passed, text) = match result {
                    Ok(report) => (report.passed(), report.text()),
                    Err(reason) => (false, reason),
                };
                view.set_decoder_tone(if passed { "live" } else { "attention" }.into());
                view.set_decoder_text(text.into());
                view.set_decoder_reason(slint::SharedString::new());
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_broken_file_means_the_defaults() {
        assert_eq!(Values::parse(""), Values::default());
        assert_eq!(Values::parse("not json"), Values::default());
        assert_eq!(Values::parse("[1, 2]"), Values::default());
        assert_eq!(Values::parse("{}"), Values::default());
        assert_eq!(
            Values::default(),
            Values {
                metrics_seconds: 5,
                font_size: 13
            }
        );
    }

    #[test]
    fn values_that_are_not_on_offer_fall_back_one_by_one() {
        let values = Values::parse(r#"{"metrics_interval_seconds": 10, "terminal_font_size": 99}"#);
        assert_eq!(
            values,
            Values {
                metrics_seconds: 10,
                font_size: 13
            }
        );
        let values =
            Values::parse(r#"{"metrics_interval_seconds": "fast", "terminal_font_size": 16}"#);
        assert_eq!(
            values,
            Values {
                metrics_seconds: 5,
                font_size: 16
            }
        );
        assert_eq!(
            Values::parse(r#"{"metrics_interval_seconds": 7, "terminal_font_size": -3}"#),
            Values::default()
        );
    }

    #[test]
    fn what_is_written_reads_back() {
        for metrics_seconds in INTERVALS {
            for font_size in FONT_SIZES {
                let values = Values {
                    metrics_seconds,
                    font_size,
                };
                assert_eq!(Values::parse(&values.to_text()), values);
            }
        }
    }

    #[test]
    fn saves_and_loads_from_a_folder() {
        let dir = std::env::temp_dir().join(format!("senvakara-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // No folder yet: the defaults.
        assert_eq!(Values::load(&dir), Values::default());
        let values = Values {
            metrics_seconds: 30,
            font_size: 18,
        };
        values.save(&dir).unwrap();
        assert_eq!(Values::load(&dir), values);
        assert!(!dir.join(format!("{FILE_NAME}.tmp")).exists());
        std::fs::write(dir.join(FILE_NAME), "{ broken").unwrap();
        assert_eq!(Values::load(&dir), Values::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_report_says_pass_only_when_every_frame_came_back() {
        let report = DecoderReport {
            width: 360,
            height: 640,
            encoded: 30,
            decoded: 30,
            per_frame: Duration::from_micros(2500),
        };
        assert!(report.passed());
        assert!(report.text().contains("2.5 ms per frame"));
        let short = DecoderReport {
            decoded: 12,
            ..report.clone()
        };
        assert!(!short.passed());
        assert!(short.text().starts_with("Only 12 of 30"));
        let nothing = DecoderReport {
            encoded: 0,
            decoded: 0,
            ..report
        };
        assert!(!nothing.passed());
    }

    #[test]
    fn the_decoder_decodes_the_synthetic_clip() {
        let report = decoder_self_test(64, 96, 6).unwrap();
        assert_eq!((report.width, report.height), (64, 96));
        assert!(report.passed(), "{report:?}");
    }
}
