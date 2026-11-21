//! The tools of the device inspector: shell command, device info, apps, screenshot, rename,
//! forget and disconnect. Everything here acts on the phone selected on Home, and every action
//! that changes the phone asks first, naming the phone, its route and what will happen.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use senvakara_app::CoreError;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::inventory::{fact, friendly, friendly_with, transport_label};
use crate::jobs::{file_name, JobSpec};
use crate::{App, DeviceTools, Inventory, KeyValue, Pending, Question};

type DeviceView = senvakara_app::android::DeviceView;

/// Shell output longer than this is cut in the inspector; Copy output still copies all of it.
const OUTPUT_LINES: usize = 200;
const OUTPUT_CHARS: usize = 16_000;

#[derive(Default)]
pub struct State {
    /// The route whose results the tools are showing; results for any other route are dropped.
    route: RefCell<String>,
    packages: Rc<VecModel<SharedString>>,
    /// The whole output of the last shell command, for Copy output.
    output: RefCell<String>,
    /// A native file dialog is open.
    picking: Cell<bool>,
}

/// The part of a long output the inspector shows, and whether anything was left out.
pub fn shown_output(output: &str) -> (String, bool) {
    let mut shown = String::new();
    for (index, line) in output.lines().enumerate() {
        if index == OUTPUT_LINES || shown.len() + line.len() > OUTPUT_CHARS {
            if shown.is_empty() {
                // One enormous line: show its beginning.
                shown = line.chars().take(OUTPUT_CHARS).collect();
            }
            return (shown, true);
        }
        if index > 0 {
            shown.push('\n');
        }
        shown.push_str(line);
    }
    (shown, false)
}

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

/// Android property names in the words the inspector uses.
fn prop_label(name: &str) -> &str {
    match name {
        "ro.product.manufacturer" => "Manufacturer",
        "ro.product.model" => "Model",
        "ro.product.device" => "Device",
        "ro.build.version.release" => "Android",
        "ro.build.version.sdk" => "API level",
        "ro.build.display.id" => "Build",
        "ro.product.cpu.abi" => "CPU",
        other => other,
    }
}

/// The lines every confirmation about a phone starts with: which phone, through which host.
pub fn target_facts(device: &DeviceView) -> Vec<KeyValue> {
    let record = &device.record;
    vec![
        fact("Phone", record.display_name.as_str(), false),
        fact(
            "Reached through",
            format!(
                "{} over {}",
                record.host_name,
                transport_label(&record.connection_type)
            ),
            false,
        ),
        fact("Serial", record.serial.as_str(), true),
    ]
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
    pub fn init_device_tools(self: &Rc<Self>, ui: &crate::MainWindow) {
        let tools = ui.global::<DeviceTools>();
        tools.set_packages(self.dev.packages.clone().into());
        tools.on_show_tab({
            let app = self.clone();
            move |tab| app.tools_show(&tab)
        });
        tools.on_run_shell({
            let app = self.clone();
            move || app.shell_ask()
        });
        tools.on_copy_output({
            let app = self.clone();
            move || {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(app.dev.output.borrow().clone());
                }
            }
        });
        tools.on_load_apps({
            let app = self.clone();
            move || app.apps_load()
        });
        tools.on_install({
            let app = self.clone();
            move || app.install_pick()
        });
        tools.on_uninstall({
            let app = self.clone();
            move |package| app.uninstall_ask(&package)
        });
        tools.on_take_shot({
            let app = self.clone();
            move || app.shot_take()
        });
        tools.on_save_shot({
            let app = self.clone();
            move || app.shot_save()
        });
        tools.on_rename_save({
            let app = self.clone();
            move || app.rename_save()
        });
        tools.on_rename_cancel({
            let app = self.clone();
            move || {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<DeviceTools>().set_renaming(false);
                }
            }
        });
    }

    /// The phone selected on Home, as the backend last reported it.
    fn selected_device(&self) -> Option<DeviceView> {
        let ui = self.ui.upgrade()?;
        let inventory = ui.global::<Inventory>();
        if inventory.get_selected_kind() != "device" {
            return None;
        }
        self.inv.device(&inventory.get_selected_id())
    }

    /// The tools global, but only while `route_id` is still the phone on screen.
    fn tools_of(&self, route_id: &str) -> Option<crate::MainWindow> {
        (*self.dev.route.borrow() == route_id)
            .then(|| self.ui.upgrade())
            .flatten()
    }

    /// Called whenever the selection is (re)published: a different phone starts with empty
    /// tools; the same phone keeps its results.
    pub fn tools_sync(&self, route_id: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let tools = ui.global::<DeviceTools>();
        let available = self
            .inv
            .device(route_id)
            .is_some_and(|device| device.record.state == "available");
        tools.set_blocked(
            if available || route_id.is_empty() {
                ""
            } else {
                "The phone must be connected and authorized first."
            }
            .into(),
        );
        if *self.dev.route.borrow() == route_id {
            return;
        }
        *self.dev.route.borrow_mut() = route_id.to_string();
        self.dev.packages.set_vec(Vec::new());
        self.dev.output.borrow_mut().clear();
        tools.set_tab("overview".into());
        tools.set_message(SharedString::new());
        tools.set_renaming(false);
        tools.set_info_busy(false);
        tools.set_props(ModelRc::default());
        tools.set_command(SharedString::new());
        tools.set_shell_busy(false);
        tools.set_shell_ran(false);
        tools.set_shell_note(SharedString::new());
        tools.set_shell_output(SharedString::new());
        tools.set_shell_error(SharedString::new());
        tools.set_apps_busy(false);
        tools.set_apps_loaded(false);
        tools.set_apps_note(SharedString::new());
        tools.set_apps_error(SharedString::new());
        tools.set_shot(slint::Image::default());
        tools.set_shot_width(0);
        tools.set_shot_height(0);
        tools.set_shot_busy(false);
        tools.set_shot_note(SharedString::new());
        tools.set_shot_path(SharedString::new());
        tools.set_shot_error(SharedString::new());
    }

    fn tools_show(self: &Rc<Self>, tab: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let tools = ui.global::<DeviceTools>();
        tools.set_tab(tab.into());
        if tab == "apps" && !tools.get_apps_loaded() && !tools.get_apps_busy() {
            self.apps_load();
        }
    }

    /// Inspector and card-menu actions that are not sessions. `mirror` and `logcat` are
    /// handled by the caller.
    pub fn device_tool_action(self: &Rc<Self>, device: &DeviceView, action: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let record = &device.record;
        // The tools live in the inspector: make sure it shows this phone.
        let inventory = ui.global::<Inventory>();
        if inventory.get_selected_kind() != "device"
            || inventory.get_selected_id() != record.route_id.as_str()
        {
            inventory.set_selected_kind("device".into());
            inventory.set_selected_id(record.route_id.as_str().into());
            self.publish_selection();
        }
        let tools = ui.global::<DeviceTools>();
        match action {
            "info" => self.info_load(device),
            "rename" => {
                tools.set_tab("overview".into());
                tools.set_rename_text(record.display_name.as_str().into());
                tools.set_message(SharedString::new());
                tools.set_renaming(true);
            }
            "disconnect" => {
                let mut facts = target_facts(device);
                facts.push(fact("Address", record.serial.as_str(), true));
                self.ask_with(
                    Question {
                        title: format!("Disconnect {}?", record.display_name),
                        body: format!(
                            "Ends the Wi-Fi debugging connection through {}. The pairing is kept, so the phone can be connected again from Add over Wi-Fi.",
                            record.host_name
                        ),
                        confirm: "Disconnect".into(),
                        danger: false,
                    },
                    facts,
                    "",
                    Pending::DisconnectWifi(record.route_id.clone()),
                );
            }
            "forget" => self.ask_with(
                Question {
                    title: format!("Forget {}?", record.display_name),
                    body: "Removes its name and history from Senvakara. It appears again as a new device if it is plugged in later.".into(),
                    confirm: "Forget phone".into(),
                    danger: true,
                },
                target_facts(device),
                "",
                Pending::ForgetDevice(record.route_id.clone()),
            ),
            _ => {}
        }
    }

    fn info_load(self: &Rc<Self>, device: &DeviceView) {
        let Some(ui) = self.ui.upgrade() else { return };
        let tools = ui.global::<DeviceTools>();
        if tools.get_info_busy() {
            return;
        }
        tools.set_tab("overview".into());
        tools.set_info_busy(true);
        tools.set_message(SharedString::new());
        let backend = self.backend.clone();
        let route = device.record.route_id.clone();
        let id = route.clone();
        self.run(
            async move { backend.device_info(id).await },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_info_busy(false);
                match result {
                    Ok(info) => {
                        let mut props: Vec<KeyValue> = info
                            .props
                            .iter()
                            .map(|(name, value)| {
                                let value = if value.is_empty() {
                                    "Not reported"
                                } else {
                                    value
                                };
                                fact(prop_label(name), value, false)
                            })
                            .collect();
                        props.push(fact("ADB transport", info.transport_id.to_string(), true));
                        tools.set_props(ModelRc::new(VecModel::from(props)));
                    }
                    Err(error) => tools.set_message(friendly(&error).into()),
                }
            },
        );
    }

    // ------------------------------------------------------------------ shell

    fn shell_ask(self: &Rc<Self>) {
        let (Some(ui), Some(device)) = (self.ui.upgrade(), self.selected_device()) else {
            return;
        };
        let command = ui.global::<DeviceTools>().get_command().trim().to_string();
        if command.is_empty() || device.record.state != "available" {
            return;
        }
        let mut facts = target_facts(&device);
        facts.push(fact("Command", command.as_str(), true));
        self.ask_with(
            Question {
                title: "Run this command on the phone?".into(),
                body: "The command runs with ADB shell permissions, on this phone only.".into(),
                confirm: "Run command".into(),
                danger: false,
            },
            facts,
            "",
            Pending::RunShell {
                route_id: device.record.route_id.clone(),
                serial: device.record.serial.clone(),
                command,
            },
        );
    }

    /// The user confirmed the command for exactly this route and serial.
    pub fn shell_run(self: &Rc<Self>, route_id: String, serial: String, command: String) {
        let Some(ui) = self.tools_of(&route_id) else {
            return;
        };
        let tools = ui.global::<DeviceTools>();
        tools.set_shell_busy(true);
        tools.set_shell_error(SharedString::new());
        let backend = self.backend.clone();
        let route = route_id.clone();
        self.run(
            async move { backend.device_shell(route_id, serial, command).await },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_shell_busy(false);
                match result {
                    Ok(done) => {
                        let (shown, cut) = shown_output(&done.output);
                        let mut note = format!("Ran on {}, serial {}.", done.host_name, done.serial);
                        if done.truncated {
                            note.push_str(" The output was truncated: the command printed more than Senvakara keeps.");
                        }
                        if cut {
                            note.push_str(" Only the beginning is shown here. Copy output copies everything.");
                        }
                        *app.dev.output.borrow_mut() = done.output;
                        tools.set_shell_note(note.into());
                        tools.set_shell_output(shown.into());
                        tools.set_shell_ran(true);
                    }
                    Err(error) => tools.set_shell_error(friendly(&error).into()),
                }
            },
        );
    }

    // ------------------------------------------------------------------- apps

    fn apps_load(self: &Rc<Self>) {
        let (Some(ui), Some(device)) = (self.ui.upgrade(), self.selected_device()) else {
            return;
        };
        if device.record.state != "available" {
            return;
        }
        let tools = ui.global::<DeviceTools>();
        tools.set_apps_busy(true);
        tools.set_apps_error(SharedString::new());
        let backend = self.backend.clone();
        let route = device.record.route_id.clone();
        let id = route.clone();
        self.run(
            async move { backend.device_packages(id).await },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_apps_busy(false);
                match result {
                    Ok(packages) => {
                        app.dev.packages.set_vec(
                            packages
                                .into_iter()
                                .map(SharedString::from)
                                .collect::<Vec<_>>(),
                        );
                        tools.set_apps_loaded(true);
                    }
                    Err(error) => tools.set_apps_error(friendly(&error).into()),
                }
            },
        );
    }

    /// Show a native file dialog without stopping the app, then continue on the UI thread.
    pub fn pick_path(
        self: &Rc<Self>,
        dialog: rfd::FileDialog,
        save: bool,
        done: impl FnOnce(&Rc<App>, PathBuf) + Send + 'static,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.dev.picking.replace(true) {
            return;
        }
        // Owned by the main window: the dialog stays in front of it and blocks it meanwhile.
        let dialog = dialog.set_parent(&ui.window().window_handle());
        std::thread::spawn(move || {
            let picked = if save {
                dialog.save_file()
            } else {
                dialog.pick_file()
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    app.dev.picking.set(false);
                    if let Some(path) = picked {
                        done(&app, path);
                    }
                }
            });
        });
    }

    /// Like [`App::pick_path`], for choosing a folder. `done` gets the folder as text.
    pub fn pick_folder(
        self: &Rc<Self>,
        dialog: rfd::FileDialog,
        done: impl FnOnce(&Rc<App>, String) + Send + 'static,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.dev.picking.replace(true) {
            return;
        }
        let dialog = dialog.set_parent(&ui.window().window_handle());
        std::thread::spawn(move || {
            let picked = dialog.pick_folder();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    app.dev.picking.set(false);
                    if let Some(path) = picked {
                        done(&app, path.display().to_string());
                    }
                }
            });
        });
    }

    fn install_pick(self: &Rc<Self>) {
        let Some(device) = self.selected_device() else {
            return;
        };
        if device.record.state != "available" {
            return;
        }
        let dialog = rfd::FileDialog::new()
            .set_title("Choose an APK to install")
            .add_filter("Android package", &["apk"]);
        let route = device.record.route_id.clone();
        self.pick_path(dialog, false, move |app, path| {
            // Ask about the phone as it is now, not as it was when the dialog opened.
            let Some(device) = app.inv.device(&route) else {
                return;
            };
            let path = path.display().to_string();
            let mut facts = target_facts(&device);
            facts.push(fact("File", path.as_str(), true));
            app.ask_with(
                Question {
                    title: format!("Install {} on this phone?", file_name(&path)),
                    body: "The file is copied to the phone and installed. An app with the same package name is updated and keeps its data.".into(),
                    confirm: "Install".into(),
                    danger: false,
                },
                facts,
                "",
                Pending::InstallApk {
                    label: format!("Install {} on {}", file_name(&path), device.record.display_name),
                    route_id: device.record.route_id.clone(),
                    serial: device.record.serial.clone(),
                    local_path: path,
                },
            );
        });
    }

    fn uninstall_ask(self: &Rc<Self>, package: &str) {
        let Some(device) = self.selected_device() else {
            return;
        };
        let mut facts = target_facts(&device);
        facts.push(fact("App", package, true));
        self.ask_with(
            Question {
                title: "Uninstall this app?".into(),
                body: "The app and its data on the phone are removed. This cannot be undone."
                    .into(),
                confirm: "Uninstall".into(),
                danger: true,
            },
            facts,
            "",
            Pending::Uninstall {
                route_id: device.record.route_id.clone(),
                serial: device.record.serial.clone(),
                package: package.to_string(),
            },
        );
    }

    pub fn uninstall(self: &Rc<Self>, route_id: String, serial: String, package: String) {
        let Some(ui) = self.tools_of(&route_id) else {
            return;
        };
        let tools = ui.global::<DeviceTools>();
        tools.set_apps_busy(true);
        tools.set_apps_error(SharedString::new());
        tools.set_apps_note(SharedString::new());
        let backend = self.backend.clone();
        let (route, name) = (route_id.clone(), package.clone());
        self.run(
            async move { backend.device_uninstall(route_id, serial, package).await },
            move |app, result| {
                let Some(ui) = app.tools_of(&route) else {
                    return;
                };
                let tools = ui.global::<DeviceTools>();
                tools.set_apps_busy(false);
                match result {
                    Ok(_) => {
                        tools.set_apps_note(format!("Removed {name}.").into());
                        app.apps_load();
                    }
                    Err(error) => tools.set_apps_error(
                        friendly_with(&error, "The phone did not remove the app.").into(),
                    ),
                }
            },
        );
    }

    /// A job ended well: refresh what it changed, if that is still on screen.
    pub fn job_finished(self: &Rc<Self>, spec: &JobSpec) {
        self.files_job_finished(spec);
        if let JobSpec::InstallApk { route_id, .. } = spec {
            let loaded = self
                .tools_of(route_id)
                .is_some_and(|ui| ui.global::<DeviceTools>().get_apps_loaded());
            if loaded {
                self.apps_load();
            }
        }
    }

    // ------------------------------------------------------------- screenshot

    fn shot_take(self: &Rc<Self>) {
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

    fn shot_save(self: &Rc<Self>) {
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

    // ------------------------------------------------- rename, forget, disconnect

    fn rename_save(self: &Rc<Self>) {
        let (Some(ui), Some(device)) = (self.ui.upgrade(), self.selected_device()) else {
            return;
        };
        let tools = ui.global::<DeviceTools>();
        let name = tools.get_rename_text().trim().to_string();
        if name.is_empty() {
            tools.set_message("Enter a name for the phone.".into());
            return;
        }
        match self
            .backend
            .device_rename(device.record.device_id.clone(), name)
        {
            Ok(()) => {
                tools.set_renaming(false);
                tools.set_message(SharedString::new());
                self.reload();
            }
            Err(error) => tools.set_message(friendly(&error).into()),
        }
    }

    fn tools_message(&self, route_id: &str, error: &CoreError, conflict: &str) {
        if let Some(ui) = self.tools_of(route_id) {
            let tools = ui.global::<DeviceTools>();
            tools.set_tab("overview".into());
            tools.set_message(friendly_with(error, conflict).into());
        }
    }

    pub fn forget(self: &Rc<Self>, route_id: String) {
        if let Err(error) = self.backend.device_forget(route_id.clone()) {
            let conflict = "The phone is still connected. Unplug it or disconnect it first.";
            self.tools_message(&route_id, &error, conflict);
        }
        self.reload();
    }

    pub fn disconnect_wifi(self: &Rc<Self>, route_id: String) {
        let backend = self.backend.clone();
        let route = route_id.clone();
        self.run(
            async move { backend.wireless_disconnect(route_id).await },
            move |app, result| {
                if let Err(error) = result {
                    app.tools_message(&route, &error, "This is not possible right now.");
                }
                app.reload();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_shown_whole() {
        assert_eq!(shown_output("a\nb\n"), ("a\nb".to_string(), false));
        assert_eq!(shown_output(""), (String::new(), false));
    }

    #[test]
    fn long_output_is_cut_at_a_line() {
        let output: String = (0..OUTPUT_LINES + 50)
            .map(|n| format!("line {n}\n"))
            .collect();
        let (shown, cut) = shown_output(&output);
        assert!(cut);
        assert_eq!(shown.lines().count(), OUTPUT_LINES);
        assert!(shown.ends_with(&format!("line {}", OUTPUT_LINES - 1)));
    }

    #[test]
    fn one_huge_line_is_cut_by_length() {
        let (shown, cut) = shown_output(&"x".repeat(OUTPUT_CHARS * 2));
        assert!(cut);
        assert_eq!(shown.len(), OUTPUT_CHARS);
    }

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

    #[test]
    fn property_labels() {
        assert_eq!(prop_label("ro.build.version.sdk"), "API level");
        assert_eq!(prop_label("ro.other"), "ro.other");
    }
}
