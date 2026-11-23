//! The Overview tools: device info, rename, forget and disconnect.

use std::rc::Rc;

use senvakara_app::CoreError;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::inventory::{fact, friendly, friendly_with};
use crate::{App, DeviceTools, Inventory, KeyValue, Pending, Question};

use super::{target_facts, DeviceView};

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

impl App {
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
            // A host form the user typed in is not dropped by this: it asks first, and the
            // action is not run then.
            let next = crate::hostform::Next::Select("device".into(), record.route_id.clone());
            if !self.host_form_leave(next) {
                return;
            }
            self.select_item("device", &record.route_id);
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

    pub(super) fn rename_save(self: &Rc<Self>) {
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
    fn property_labels() {
        assert_eq!(prop_label("ro.build.version.sdk"), "API level");
        assert_eq!(prop_label("ro.other"), "ro.other");
    }
}
