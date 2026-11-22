//! The tools of the device inspector: shell command, device info, apps, screenshot, rename,
//! forget and disconnect. Everything here acts on the phone selected on Home, and every action
//! that changes the phone asks first, naming the phone, its route and what will happen.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::inventory::{fact, transport_label};
use crate::{App, DeviceTools, Inventory, KeyValue};

mod apps;
mod file_dialog;
mod overview;
mod screen;
mod shell;

type DeviceView = senvakara_app::android::DeviceView;

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
}
