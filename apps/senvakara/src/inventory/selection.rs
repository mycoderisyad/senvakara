//! The selected host or phone and the inspector that shows it.

use slint::{ComponentHandle, Model, SharedString};

use crate::{App, Inventory};

impl App {
    /// Show a host or a phone in the inspector ("" for neither).
    pub fn select_item(&self, kind: &str, id: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let inventory = ui.global::<Inventory>();
        inventory.set_selected_kind(kind.into());
        inventory.set_selected_id(id.into());
        self.publish_selection();
    }

    /// Keep the inspector showing the current version of the selected item.
    pub fn publish_selection(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let inventory = ui.global::<Inventory>();
        let id = inventory.get_selected_id();
        let kind = inventory.get_selected_kind();
        self.tools_sync(if kind == "device" { id.as_str() } else { "" });
        self.paths_sync(if kind == "host" { id.as_str() } else { "" });
        match kind.as_str() {
            "host" => match self.inv.host_model.iter().find(|host| host.id == id) {
                Some(host) => inventory.set_selected_host(host),
                None => inventory.set_selected_kind(SharedString::new()),
            },
            "device" => match self.inv.device_model.iter().find(|device| device.id == id) {
                Some(device) => inventory.set_selected_device(device),
                None => inventory.set_selected_kind(SharedString::new()),
            },
            _ => {}
        }
    }
}
