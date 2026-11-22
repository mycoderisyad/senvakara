//! Keeping Home current: wiring it to the backend, reloading, rebuilding lists, polling health.

use std::rc::Rc;

use senvakara_app::AppEvent;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{App, DeviceItem, HostItem, Inventory, KeyValue};

use super::text::{fact, friendly};

impl App {
    /// Wire Home to the backend. Called once at startup, on the UI thread.
    pub fn init_inventory(self: &Rc<Self>, ui: &crate::MainWindow) {
        let inventory = ui.global::<Inventory>();
        inventory.set_hosts(self.inv.host_model.clone().into());
        inventory.set_devices(self.inv.device_model.clone().into());
        *self.inv.wsl.borrow_mut() = crate::wsl_distros();

        inventory.on_select({
            let app = self.clone();
            move |kind, id| {
                // A form the user typed in is not dropped by a click elsewhere.
                let next = crate::hostform::Next::Select(kind.to_string(), id.to_string());
                if app.host_form_leave(next) {
                    app.select_item(&kind, &id);
                }
            }
        });
        inventory.on_search({
            let app = self.clone();
            move |text| {
                *app.inv.query.borrow_mut() = text.to_lowercase();
                app.rebuild();
            }
        });
        inventory.on_refresh({
            let app = self.clone();
            move || app.reload()
        });
        inventory.on_host_action({
            let app = self.clone();
            move |host, action| app.host_action(&host, &action)
        });
        inventory.on_device_action({
            let app = self.clone();
            move |route, action| app.device_action(&route, &action)
        });

        // Backend events are refresh hints: reload on any of them.
        let mut events = self.backend.subscribe();
        self.backend.rt.spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => Some(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                    Err(_) => break,
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = crate::app() {
                        if let Some(AppEvent::HostLost { host_id, reason }) = &event {
                            let message = format!("The connection was lost. Senvakara is trying to reconnect. ({reason})");
                            app.inv.errors.borrow_mut().insert(host_id.clone(), message);
                        }
                        if let Some(AppEvent::HostReconnected { host_id, .. }) = &event {
                            app.inv.errors.borrow_mut().remove(host_id);
                        }
                        app.reload();
                    }
                });
            }
        });

        // Health is read only while Home is visible. Settings may change the interval later.
        self.set_metrics_interval(std::time::Duration::from_secs(5));
        self.reload();
        self.sample_metrics();
    }

    /// How often host health is read (Settings, General).
    pub fn set_metrics_interval(self: &Rc<Self>, every: std::time::Duration) {
        let app = self.clone();
        self.inv
            .metrics_timer
            .start(slint::TimerMode::Repeated, every, move || {
                app.sample_metrics()
            });
    }

    /// Read hosts and devices again and redraw.
    pub fn reload(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let inventory = ui.global::<Inventory>();
        match (self.backend.hosts_list(), self.backend.devices_list()) {
            (Ok(hosts), Ok(devices)) => {
                *self.inv.hues.borrow_mut() = hosts
                    .iter()
                    .enumerate()
                    .map(|(index, host)| (host.id.clone(), (index % 8) as i32))
                    .collect();
                *self.inv.hosts.borrow_mut() = hosts;
                *self.inv.devices.borrow_mut() = devices;
                inventory.set_error(SharedString::new());
            }
            (Err(error), _) | (_, Err(error)) => inventory.set_error(friendly(&error).into()),
        }
        let status = self.backend.adb_status();
        *self.inv.local_adb.borrow_mut() = status.local_adb_path;
        *self.inv.watching.borrow_mut() = status
            .hosts
            .into_iter()
            .map(|(id, watch)| (id, (watch.watching, watch.detail)))
            .collect();
        self.rebuild();
    }

    /// Rebuild the view models from the cached backend data.
    pub(super) fn rebuild(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let query = self.inv.query.borrow().clone();
        let hosts = self.inv.hosts.borrow();
        let devices = self.inv.devices.borrow();

        let matches = |fields: &[&str]| {
            query.is_empty()
                || fields
                    .iter()
                    .any(|field| field.to_lowercase().contains(&query))
        };
        let host_items: Vec<HostItem> = hosts
            .iter()
            .filter(|host| {
                let address = host.ssh.as_ref().map(|s| s.address.as_str()).unwrap_or("");
                matches(&[&host.display_name, address])
            })
            .map(|host| self.inv.host_item(host))
            .collect();
        let device_items: Vec<DeviceItem> = devices
            .iter()
            .filter(|device| {
                matches(&[
                    &device.record.display_name,
                    &device.record.serial,
                    &device.record.host_name,
                ])
            })
            .map(|device| self.inv.device_item(device))
            .collect();
        self.inv.host_model.set_vec(host_items);
        self.inv.device_model.set_vec(device_items);

        let count = |state: &str| {
            devices
                .iter()
                .filter(|device| device.record.state == state)
                .count()
        };
        let connected = hosts
            .iter()
            .filter(|host| host.is_local || host.connected)
            .count();
        let inventory = ui.global::<Inventory>();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let (available, unauthorized) = (count("available"), count("unauthorized"));
        let mut phones = vec![format!(
            "{} available",
            plural(available, "phone", "phones")
        )];
        if unauthorized > 0 {
            phones.push(format!("{unauthorized} unauthorized"));
        }
        let away = devices.len() - available - unauthorized;
        if away > 0 {
            phones.push(format!("{away} not connected"));
        }
        inventory.set_summary(
            format!(
                "{connected} of {} reachable  ·  {}",
                plural(hosts.len(), "host", "hosts"),
                phones.join(", ")
            )
            .into(),
        );

        let watching = self.inv.watching.borrow();
        let sources: Vec<KeyValue> = hosts
            .iter()
            .map(|host| {
                let value = match watching.get(&host.id) {
                    Some((true, _)) => "Watching for phones".to_string(),
                    Some((false, detail)) if !detail.is_empty() => detail.clone(),
                    _ if host.is_local => "Not running".to_string(),
                    _ if !host.connected => "Connect the host to see its phones".to_string(),
                    _ => "Not watching".to_string(),
                };
                fact(&host.display_name, value, false)
            })
            .collect();
        inventory.set_adb_sources(ModelRc::new(VecModel::from(sources)));
        drop((hosts, devices, watching));
        self.publish_selection();
        self.files_inventory_changed();
        self.workspaces_inventory_changed();
        self.automations_inventory_changed();
    }

    /// Redraw one host card after its health changed. A full `rebuild` here
    /// recreated every list (and the other screens' host lists) every 5 s, which
    /// was most of the idle CPU on Home.
    fn refresh_host_row(&self, host_id: &str) {
        let Some(index) = self
            .inv
            .host_model
            .iter()
            .position(|host| host.id == host_id)
        else {
            return;
        };
        let Some(item) = self
            .inv
            .hosts
            .borrow()
            .iter()
            .find(|host| host.id == host_id)
            .map(|host| self.inv.host_item(host))
        else {
            return;
        };
        let Some(ui) = self.ui.upgrade() else { return };
        let inventory = ui.global::<Inventory>();
        if inventory.get_selected_kind() == "host" && inventory.get_selected_id() == host_id {
            inventory.set_selected_host(item.clone());
        }
        self.inv.host_model.set_row_data(index, item);
    }

    pub(super) fn sample_metrics(self: &Rc<Self>) {
        // Only while Home is on screen, and only for hosts that can answer.
        if self.active.get() >= 0 {
            return;
        }
        let ids: Vec<String> = self
            .inv
            .hosts
            .borrow()
            .iter()
            .filter(|host| host.is_local || host.connected)
            .map(|host| host.id.clone())
            .collect();
        for id in ids {
            let backend = self.backend.clone();
            let host = id.clone();
            self.run(
                async move { backend.host_metrics(host).await },
                move |app, result| {
                    if let Ok(metrics) = result {
                        app.inv.metrics.borrow_mut().insert(id.clone(), metrics);
                        app.refresh_host_row(&id);
                    }
                },
            );
        }
        // The first CPU sample has nothing to compare with: take a second one right away.
        if !self.inv.primed.replace(true) {
            let app = self.clone();
            slint::Timer::single_shot(std::time::Duration::from_secs(1), move || {
                app.sample_metrics()
            });
        }
    }
}
