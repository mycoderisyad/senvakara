//! The phone side of Home's state: lookups over the device list and the card built for each phone.

use slint::{ModelRc, VecModel};

use crate::{Action, DeviceItem};

use super::text::{action, blocked, fact, transport_label};
use super::{DeviceView, State};

impl State {
    /// Every phone, as the backend last reported them.
    pub fn devices(&self) -> Vec<DeviceView> {
        self.devices.borrow().clone()
    }

    /// The card of every phone, whatever the search field says.
    pub fn device_items(&self) -> Vec<DeviceItem> {
        self.devices
            .borrow()
            .iter()
            .map(|device| self.device_item(device))
            .collect()
    }

    /// Route ids of the phones that can be used right now, in list order.
    pub fn available_devices(&self) -> Vec<String> {
        self.devices
            .borrow()
            .iter()
            .filter(|device| device.record.state == "available")
            .map(|device| device.record.route_id.clone())
            .collect()
    }

    /// The phone reached through `route_id`, as the backend last reported it.
    pub fn device(&self, route_id: &str) -> Option<DeviceView> {
        self.devices
            .borrow()
            .iter()
            .find(|device| device.record.route_id == route_id)
            .cloned()
    }

    pub(super) fn device_item(&self, device: &DeviceView) -> DeviceItem {
        let record = &device.record;
        let transport = transport_label(&record.connection_type);
        let (tone, state_text, hint) = match record.state.as_str() {
            "available" => ("live", "Available", ""),
            "unauthorized" => (
                "attention",
                "Unauthorized",
                "Unlock the phone and allow USB debugging from this computer.",
            ),
            "offline" => (
                "idle",
                "Offline",
                "ADB sees the phone but it is not responding yet.",
            ),
            _ => ("idle", "Not connected", ""),
        };
        let available = record.state == "available";
        let needs_phone = "The phone must be connected and authorized first.";
        let when_available = |id: &str, label: &str| {
            if available {
                action(id, label)
            } else {
                blocked(id, label, needs_phone)
            }
        };
        let primary = when_available("mirror", "Open screen");
        let connected = record.state != "unavailable";
        let actions = vec![
            primary.clone(),
            when_available("logcat", "Open logcat"),
            when_available("files", "Open files"),
            when_available("info", "Read device info"),
            action("rename", "Rename"),
            if transport != "Wi-Fi" {
                blocked(
                    "disconnect",
                    "Disconnect",
                    "Only Wi-Fi connections can be disconnected from here. Unplug the cable instead.",
                )
            } else if !connected {
                blocked("disconnect", "Disconnect", "The phone is not connected.")
            } else {
                action("disconnect", "Disconnect")
            },
            Action {
                danger: true,
                ..if connected {
                    blocked(
                        "forget",
                        "Forget",
                        "Only a phone that is not connected can be forgotten. Unplug or disconnect it first.",
                    )
                } else {
                    action("forget", "Forget")
                }
            },
        ];
        let mut details = vec![fact("Serial", record.serial.as_str(), true)];
        if let Some(model) = &record.model {
            details.push(fact("Model", model.as_str(), false));
        }
        details.push(fact(
            "Reached through",
            format!("{} over {transport}", record.host_name),
            false,
        ));
        if let Some(id) = device.transport_id {
            details.push(fact("ADB transport", id.to_string(), true));
        }
        DeviceItem {
            id: record.route_id.as_str().into(),
            name: record.display_name.as_str().into(),
            route: format!("{}  >  {transport}", record.host_name).into(),
            host_hue: self.hue(&record.host_id),
            tone: tone.into(),
            state_text: state_text.into(),
            primary: primary.clone(),
            actions: ModelRc::new(VecModel::from(actions)),
            details: ModelRc::new(VecModel::from(details)),
            hint: hint.into(),
        }
    }
}
