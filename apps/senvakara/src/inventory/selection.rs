//! The selected host or phone and the inspector that shows it.

use slint::{ComponentHandle, SharedString};

use crate::{App, Inventory};

/// The item with `id`, looked up in everything the backend reported, not in what the search box
/// leaves on screen: filtering the lists must not drop the selection.
fn find_selected<'a, T>(items: &'a [T], id: &str, id_of: impl Fn(&T) -> &str) -> Option<&'a T> {
    items.iter().find(|item| id_of(item) == id)
}

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
            "host" => {
                let hosts = self.inv.hosts.borrow();
                match find_selected(&hosts, &id, |host| host.id.as_str()) {
                    Some(host) => inventory.set_selected_host(self.inv.host_item(host)),
                    None => inventory.set_selected_kind(SharedString::new()),
                }
            }
            "device" => {
                let devices = self.inv.devices.borrow();
                match find_selected(&devices, &id, |device| device.record.route_id.as_str()) {
                    Some(device) => inventory.set_selected_device(self.inv.device_item(device)),
                    None => inventory.set_selected_kind(SharedString::new()),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_found_among_everything_not_among_what_the_search_shows() {
        let reported = [("a", "Pixel"), ("b", "Redmi")];
        // The search box shows only "Pixel"; the selected phone is "b".
        let on_screen = [("a", "Pixel")];
        assert!(find_selected(&on_screen, "b", |item| item.0).is_none());
        assert_eq!(
            find_selected(&reported, "b", |item| item.0),
            Some(&("b", "Redmi"))
        );
        assert!(find_selected(&reported, "gone", |item| item.0).is_none());
    }
}
