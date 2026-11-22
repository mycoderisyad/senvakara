//! The Apps tool: listing packages, installing an APK and uninstalling an app.

use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::inventory::{fact, friendly, friendly_with};
use crate::jobs::{file_name, JobSpec};
use crate::{App, DeviceTools, Pending, Question};

use super::target_facts;

impl App {
    // ------------------------------------------------------------------- apps

    pub(super) fn apps_load(self: &Rc<Self>) {
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

    pub(super) fn install_pick(self: &Rc<Self>) {
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

    pub(super) fn uninstall_ask(self: &Rc<Self>, package: &str) {
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
}
