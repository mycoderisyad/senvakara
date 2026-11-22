//! The Screen tool: taking a screenshot of the phone, showing it and saving it.

use std::rc::Rc;

use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer, SharedString};

use crate::inventory::{friendly, friendly_with};
use crate::{App, DeviceTools};

/// A file name made from a device name: letters, digits, dot, dash and underscore only.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "phone".to_string()
    } else {
        trimmed.to_string()
    }
}

fn decode_png(png: &[u8]) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let picture = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("The screenshot could not be read. ({error})"))?
        .into_rgba8();
    Ok(SharedPixelBuffer::clone_from_slice(
        picture.as_raw(),
        picture.width(),
        picture.height(),
    ))
}

impl App {
    pub(super) fn shot_take(self: &Rc<Self>) {
        let (Some(ui), Some(device)) = (self.ui.upgrade(), self.selected_device()) else {
            return;
        };
        if device.record.state != "available" {
            return;
        }
        let tools = ui.global::<DeviceTools>();
        tools.set_shot_busy(true);
        tools.set_shot_error(SharedString::new());
        tools.set_shot_note(SharedString::new());
        tools.set_shot_path(SharedString::new());
        let backend = self.backend.clone();
        let route = device.record.route_id.clone();
        let id = route.clone();
        self.run(
            async move {
                let png = backend
                    .device_screenshot(id)
                    .await
                    .map_err(|e| friendly(&e))?;
                decode_png(&png)
            },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_shot_busy(false);
                match result {
                    Ok(pixels) => {
                        tools.set_shot_width(pixels.width() as i32);
                        tools.set_shot_height(pixels.height() as i32);
                        tools.set_shot(slint::Image::from_rgba8(pixels));
                    }
                    Err(message) => tools.set_shot_error(message.into()),
                }
            },
        );
    }

    pub(super) fn shot_save(self: &Rc<Self>) {
        let Some(device) = self.selected_device() else {
            return;
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dialog = rfd::FileDialog::new()
            .set_title("Save screenshot")
            .set_file_name(format!(
                "{}-{stamp}.png",
                safe_name(&device.record.display_name)
            ))
            .add_filter("PNG image", &["png"]);
        let route = device.record.route_id.clone();
        self.pick_path(dialog, true, move |app, path| {
            let backend = app.backend.clone();
            let path = path.display().to_string();
            let (id, saved) = (route.clone(), path.clone());
            app.run(
                async move { backend.device_screenshot_save(id, path).await },
                move |app, result| {
                    let Some(ui) = app.tools_of(&route) else {
                        return;
                    };
                    let tools = ui.global::<DeviceTools>();
                    match result {
                        Ok(()) => {
                            tools.set_shot_error(SharedString::new());
                            tools.set_shot_note("The screenshot was saved.".into());
                            tools.set_shot_path(saved.into());
                        }
                        Err(error) => tools.set_shot_error(
                            friendly_with(&error, "Take a screenshot first.").into(),
                        ),
                    }
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_from_device_names() {
        assert_eq!(safe_name("Redmi Note 11"), "Redmi_Note_11");
        assert_eq!(safe_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(safe_name("  ?? "), "phone");
        assert_eq!(safe_name("pixel-8.pro"), "pixel-8.pro");
    }

    #[test]
    fn decodes_a_png_into_rgba_pixels() {
        let mut picture = image::RgbaImage::new(3, 2);
        picture.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        picture.put_pixel(2, 1, image::Rgba([0, 0, 255, 255]));
        let mut png = Vec::new();
        picture
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let pixels = decode_png(&png).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (3, 2));
        let bytes = pixels.as_bytes();
        assert_eq!(&bytes[..4], [255, 0, 0, 255]);
        assert_eq!(&bytes[20..], [0, 0, 255, 255]);
        assert!(decode_png(b"not a png").is_err());
    }
}
