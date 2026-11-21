//! Home: hosts and devices from the backend, turned into the view models the UI draws, and the
//! actions the user can take on them.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use senvakara_app::{
    AppEvent, CoreError, ErrorCode, HostView, Metrics, ShellRequest, Sink, TermEvent,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{
    term, Action, App, DeviceItem, HostItem, Inventory, KeyValue, Meter, PathItem, Pending,
    Question, TabInfo, VpnHint,
};

type DeviceView = senvakara_app::android::DeviceView;

/// Everything Home shows, kept so the lists can be rebuilt after any change.
#[derive(Default)]
pub struct State {
    hosts: RefCell<Vec<HostView>>,
    devices: RefCell<Vec<DeviceView>>,
    watching: RefCell<HashMap<String, (bool, String)>>,
    local_adb: RefCell<Option<String>>,
    metrics: RefCell<HashMap<String, Metrics>>,
    /// The last failure per host, already worded for the user.
    errors: RefCell<HashMap<String, String>>,
    /// Hosts with a connect or disconnect in flight.
    busy: RefCell<HashSet<String>>,
    wsl: RefCell<Vec<String>>,
    /// Colour index per host id: its position in the host list, so two hosts never share one
    /// until there are more than eight.
    hues: RefCell<HashMap<String, i32>>,
    query: RefCell<String>,
    host_model: Rc<VecModel<HostItem>>,
    device_model: Rc<VecModel<DeviceItem>>,
    metrics_timer: slint::Timer,
    primed: Cell<bool>,
}

/// A backend error in words a user can act on. The raw code and message stay in the log.
pub fn friendly(error: &CoreError) -> String {
    friendly_with(error, "A file with that name already exists.")
}

/// Like [`friendly`], for callers where a "conflict" is not about a file name: `conflict` says
/// what it means there.
pub fn friendly_with(error: &CoreError, conflict: &str) -> String {
    let lead = match error.code {
        ErrorCode::HostOffline => "The host did not answer. Check that it is on and reachable on this network.",
        ErrorCode::Unauthenticated => "The host refused the key. Check the username, the private key and its passphrase.",
        ErrorCode::Unauthorized => "This is not allowed.",
        ErrorCode::DeviceOffline => "The phone is not connected any more.",
        ErrorCode::AdbUnauthorized => "The phone has not allowed USB debugging from this computer. Unlock it and accept the prompt.",
        ErrorCode::RouteStale => "The phone was reconnected since this view was opened. Try again.",
        ErrorCode::UnsupportedCapability => "This is not available on that target.",
        ErrorCode::Timeout => "It took too long and was stopped.",
        ErrorCode::Cancelled => "Cancelled.",
        ErrorCode::Conflict => conflict,
        ErrorCode::InvalidInput => "Something in the request is not valid.",
        _ => "Something went wrong.",
    };
    if error.message.is_empty() {
        lead.to_string()
    } else {
        format!("{lead} ({})", error.message)
    }
}

pub fn action(id: &str, label: &str) -> Action {
    Action {
        id: id.into(),
        label: label.into(),
        reason: SharedString::new(),
        danger: false,
    }
}

pub fn blocked(id: &str, label: &str, reason: &str) -> Action {
    Action {
        reason: reason.into(),
        ..action(id, label)
    }
}

pub fn fact(key: &str, value: impl Into<SharedString>, mono: bool) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: value.into(),
        mono,
    }
}

/// "USB" or "Wi-Fi", from a route's connection type.
pub fn transport_label(connection_type: &str) -> &'static str {
    if connection_type.contains("wifi") || connection_type.contains("tcp") {
        "Wi-Fi"
    } else {
        "USB"
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

fn rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_048_576.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_048_576.0)
    } else {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    }
}

fn uptime(secs: f64) -> String {
    let minutes = (secs / 60.0) as u64;
    let (days, hours, minutes) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}

fn meters(metrics: &Metrics) -> (Vec<Meter>, String) {
    let fraction = |used: u64, total: u64| {
        if total == 0 {
            0.0
        } else {
            used as f32 / total as f32
        }
    };
    let cpu = metrics.cpu_percent.unwrap_or(0.0);
    let list = vec![
        Meter {
            label: format!("CPU ({} cores)", metrics.cpu_count).into(),
            value: match metrics.cpu_percent {
                Some(percent) => format!("{percent:.0} %").into(),
                None => "measuring".into(),
            },
            fraction: (cpu / 100.0) as f32,
        },
        Meter {
            label: "Memory".into(),
            value: format!("{} of {}", gib(metrics.mem_used), gib(metrics.mem_total)).into(),
            fraction: fraction(metrics.mem_used, metrics.mem_total),
        },
        Meter {
            label: "System disk".into(),
            value: format!("{} of {}", gib(metrics.disk_used), gib(metrics.disk_total)).into(),
            fraction: fraction(metrics.disk_used, metrics.disk_total),
        },
    ];
    let mut note = Vec::new();
    if let (Some(rx), Some(tx)) = (metrics.net_rx_per_sec, metrics.net_tx_per_sec) {
        note.push(format!("Network {} down, {} up", rate(rx), rate(tx)));
    }
    note.push(format!("Up {}", uptime(metrics.uptime_secs)));
    if let Some(load) = metrics.load1 {
        note.push(format!("Load {load:.2}"));
    }
    if metrics.swap_used > 0 {
        note.push(format!("Swap {}", gib(metrics.swap_used)));
    }
    (list, note.join("  ·  "))
}

/// Why the remote desktop of a host cannot be opened, or "" when it can. `session` is what the
/// host reported when it connected: "x11", "wayland" or "none".
pub fn desktop_problem(connected: bool, session: &str, has_x11vnc: bool) -> &'static str {
    if !connected {
        return "Connect the host first.";
    }
    match session {
        "x11" if has_x11vnc => "",
        "x11" => "x11vnc is not installed on the host (sudo dnf install x11vnc).",
        "wayland" => "The host runs a Wayland session. Remote desktop supports X11 sessions only.",
        _ => "The host is in text mode: no graphical session is running. Use the terminal.",
    }
}

impl State {
    /// Every host, as the backend last reported them.
    pub fn hosts(&self) -> Vec<HostView> {
        self.hosts.borrow().clone()
    }

    /// Every phone, as the backend last reported them.
    pub fn devices(&self) -> Vec<DeviceView> {
        self.devices.borrow().clone()
    }

    /// Installed WSL distributions, default first.
    pub fn wsl(&self) -> Vec<String> {
        self.wsl.borrow().clone()
    }

    /// This PC.
    pub fn local_host(&self) -> Option<HostView> {
        self.hosts
            .borrow()
            .iter()
            .find(|host| host.is_local)
            .cloned()
    }

    /// The card of every host, whatever the search field says.
    pub fn host_items(&self) -> Vec<HostItem> {
        self.hosts
            .borrow()
            .iter()
            .map(|host| self.host_item(host))
            .collect()
    }

    /// The card of every phone, whatever the search field says.
    pub fn device_items(&self) -> Vec<DeviceItem> {
        self.devices
            .borrow()
            .iter()
            .map(|device| self.device_item(device))
            .collect()
    }

    /// A host, as the backend last reported it.
    pub fn host(&self, id: &str) -> Option<HostView> {
        self.hosts
            .borrow()
            .iter()
            .find(|host| host.id == id)
            .cloned()
    }

    pub fn host_name(&self, id: &str) -> Option<String> {
        self.host(id).map(|host| host.display_name)
    }

    /// Ids of the remote hosts with a live connection, in list order.
    pub fn connected_hosts(&self) -> Vec<String> {
        self.hosts
            .borrow()
            .iter()
            .filter(|host| !host.is_local && host.connected)
            .map(|host| host.id.clone())
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

    /// (id, name) of every host whose ADB is being watched: the hosts a phone can be added
    /// through.
    pub fn adb_hosts(&self) -> Vec<(String, String)> {
        let watching = self.watching.borrow();
        self.hosts
            .borrow()
            .iter()
            .filter(|host| watching.get(&host.id).is_some_and(|(on, _)| *on))
            .map(|host| {
                let name = if host.is_local {
                    format!("{} (this PC)", host.display_name)
                } else {
                    host.display_name.clone()
                };
                (host.id.clone(), name)
            })
            .collect()
    }

    /// The colour index of a host, shared by its card, its tabs and its phones' route chips.
    pub fn hue(&self, host_id: &str) -> i32 {
        self.hues.borrow().get(host_id).copied().unwrap_or(0)
    }

    fn host_item(&self, host: &HostView) -> HostItem {
        let busy = self.busy.borrow().contains(&host.id);
        let error = self
            .errors
            .borrow()
            .get(&host.id)
            .cloned()
            .unwrap_or_default();
        let trusted = host.trust_state == "trusted";
        let caps = &host.capabilities;
        let text = |key: &str| caps.get(key).and_then(|v| v.as_str()).map(str::to_string);

        let mut actions = Vec::new();
        let mut details = Vec::new();
        let (subtitle, tone, state_text);

        if host.is_local {
            subtitle = "Windows, this computer".to_string();
            tone = "live";
            state_text = "This PC".to_string();
            actions.push(action("terminal", "Open PowerShell"));
            for distro in self.wsl.borrow().iter() {
                actions.push(action(
                    &format!("wsl:{distro}"),
                    &format!("Open WSL {distro}"),
                ));
            }
            let adb = self.local_adb.borrow().clone();
            details.push(fact(
                "ADB",
                adb.unwrap_or_else(|| "Not found. Install Android platform-tools.".into()),
                true,
            ));
        } else {
            let ssh = host.ssh.as_ref();
            let address = ssh
                .map(|s| format!("{}@{}", s.username, s.address))
                .unwrap_or_default();
            subtitle = match text("os_pretty_name") {
                Some(os) if host.connected => format!("{address}  ·  {os}"),
                _ => format!("ssh, {address}"),
            };
            let via_vpn = host
                .path
                .as_ref()
                .is_some_and(|path| path.transport == "vpn");
            (tone, state_text) = if busy {
                ("busy", "Working".to_string())
            } else if !trusted {
                ("attention", "Key not trusted".to_string())
            } else if let Some(attempt) = host.reconnecting {
                ("busy", format!("Reconnecting ({attempt})"))
            } else if host.connected {
                (
                    "live",
                    if via_vpn {
                        "Connected, VPN"
                    } else {
                        "Connected"
                    }
                    .to_string(),
                )
            } else if !error.is_empty() {
                ("attention", "Not connected".to_string())
            } else {
                ("idle", "Not connected".to_string())
            };

            if !trusted {
                actions.push(action("trust", "Verify host key"));
            } else {
                actions.push(action("terminal", "Open terminal"));
                if caps.get("tmux").and_then(|v| v.as_bool()).unwrap_or(false) {
                    actions.push(action("tmux", "Open terminal in tmux"));
                }
                if host.connected {
                    actions.push(action("disconnect", "Disconnect"));
                } else if busy {
                    actions.push(blocked(
                        "connect",
                        "Connect",
                        "A connection attempt is already running.",
                    ));
                } else {
                    actions.push(action("connect", "Connect"));
                }
                let connect_first = "Connect the host first.";
                actions.push(if host.connected {
                    action("files", "Open files")
                } else {
                    blocked("files", "Open files", connect_first)
                });
                let desktop = caps.get("desktop");
                let session = desktop
                    .and_then(|d| d.get("session"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");
                let has_x11vnc = desktop
                    .and_then(|d| d.get("x11vnc"))
                    .is_some_and(|v| v.as_str().is_some_and(|path| !path.is_empty()));
                let problem = desktop_problem(host.connected, session, has_x11vnc);
                actions.push(if problem.is_empty() {
                    action("desktop", "Open desktop")
                } else {
                    blocked("desktop", "Open desktop", problem)
                });
                actions.push(action("trust", "Check host key"));
            }
            actions.push(action("edit", "Edit"));
            actions.push(Action {
                danger: true,
                ..action("remove", "Remove host")
            });

            if let Some(ssh) = ssh {
                details.push(fact(
                    "Address",
                    format!("{}:{}", ssh.address, ssh.port),
                    true,
                ));
                details.push(fact("User", ssh.username.as_str(), true));
                if let Some(fingerprint) = &ssh.fingerprint {
                    details.push(fact("Host key", fingerprint.as_str(), true));
                }
            }
            if let Some(os) = text("os_pretty_name") {
                let arch = text("arch").map(|a| format!(", {a}")).unwrap_or_default();
                details.push(fact("System", format!("{os}{arch}"), false));
            }
            if host.connected {
                details.push(fact(
                    "ADB",
                    text("adb_path").unwrap_or_else(|| "Not installed".into()),
                    true,
                ));
                let desktop = caps.get("desktop");
                let session = desktop
                    .and_then(|d| d.get("session"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");
                let screen = match session {
                    "x11" => {
                        let name = desktop
                            .and_then(|d| d.get("desktop"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("X11");
                        format!("{name} on X11")
                    }
                    "wayland" => "Wayland (screen sharing not supported)".to_string(),
                    _ => "Text mode".to_string(),
                };
                details.push(fact("Screen", screen, false));
            }
        }

        let (meter_list, meters_note) = match self.metrics.borrow().get(&host.id) {
            Some(metrics) if host.is_local || host.connected => meters(metrics),
            _ => (Vec::new(), String::new()),
        };

        // Every address the host can be reached at: its main one first, then VPN addresses.
        let mut paths = Vec::new();
        let mut hints = Vec::new();
        let mut path_note = String::new();
        if let Some(ssh) = host.ssh.as_ref() {
            let active = host.path.as_ref().filter(|_| host.connected);
            let in_use = |transport: &str, address: &str, port: u16| {
                active.is_some_and(|p| {
                    p.transport == transport && p.address == address && p.port == port
                })
            };
            paths.push(PathItem {
                id: SharedString::new(),
                kind: "LAN".into(),
                address: format!("{}:{}", ssh.address, ssh.port).into(),
                in_use: in_use("lan", &ssh.address, ssh.port),
            });
            for endpoint in &host.vpn {
                paths.push(PathItem {
                    id: endpoint.endpoint_id.as_str().into(),
                    kind: "VPN".into(),
                    address: format!("{}:{}", endpoint.address, endpoint.port).into(),
                    in_use: in_use("vpn", &endpoint.address, endpoint.port),
                });
            }
            if let Some(note) = active.and_then(|p| p.note.as_deref()) {
                path_note = note.to_string();
            }
            // Addresses the host reported on VPN-looking interfaces: offered, never trusted.
            if let Some(found) = caps.get("vpn_interfaces").and_then(|v| v.as_array()) {
                for interface in found {
                    let get = |key: &str| interface.get(key).and_then(|v| v.as_str());
                    let (Some(name), Some(address)) = (get("name"), get("address")) else {
                        continue;
                    };
                    let known =
                        address == ssh.address || host.vpn.iter().any(|vpn| vpn.address == address);
                    if !known {
                        hints.push(VpnHint {
                            label: format!("Add {name} {address}").into(),
                            address: address.into(),
                        });
                    }
                }
            }
        }

        HostItem {
            id: host.id.as_str().into(),
            name: host.display_name.as_str().into(),
            subtitle: subtitle.into(),
            os: if host.os_family == "windows" {
                "windows"
            } else {
                "linux"
            }
            .into(),
            hue: self.hue(&host.id),
            tone: tone.into(),
            state_text: state_text.into(),
            primary: actions[0].clone(),
            actions: ModelRc::new(VecModel::from(actions)),
            details: ModelRc::new(VecModel::from(details)),
            meters: ModelRc::new(VecModel::from(meter_list)),
            meters_note: meters_note.into(),
            paths: ModelRc::new(VecModel::from(paths)),
            path_note: path_note.into(),
            vpn_hints: ModelRc::new(VecModel::from(hints)),
            error: error.into(),
        }
    }

    fn device_item(&self, device: &DeviceView) -> DeviceItem {
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
    fn rebuild(self: &Rc<Self>) {
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

    /// Run a backend call and handle its result back on the UI thread.
    pub fn run<T: Send + 'static>(
        self: &Rc<Self>,
        work: impl std::future::Future<Output = T> + Send + 'static,
        done: impl FnOnce(&Rc<App>, T) + Send + 'static,
    ) {
        self.backend.rt.spawn(async move {
            let output = work.await;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    done(&app, output);
                }
            });
        });
    }

    fn sample_metrics(self: &Rc<Self>) {
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

    pub fn host_action(self: &Rc<Self>, host_id: &str, action: &str) {
        let Some(host) = self
            .inv
            .hosts
            .borrow()
            .iter()
            .find(|host| host.id == host_id)
            .cloned()
        else {
            return;
        };
        match action {
            "terminal" if host.is_local => self.open_shell("powershell", ""),
            "terminal" => self.open_ssh(&host, None),
            "tmux" => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                self.open_ssh(&host, Some(format!("senvakara-{}", base36(stamp))));
            }
            "connect" | "disconnect" => {
                self.inv.busy.borrow_mut().insert(host.id.clone());
                self.inv.errors.borrow_mut().remove(&host.id);
                self.rebuild();
                let backend = self.backend.clone();
                let (id, connect) = (host.id.clone(), action == "connect");
                self.run(
                    async move {
                        if connect {
                            backend.host_connect(id).await
                        } else {
                            backend.host_disconnect(id).await
                        }
                    },
                    move |app, result| {
                        app.inv.busy.borrow_mut().remove(&host.id);
                        if let Err(error) = result {
                            app.inv
                                .errors
                                .borrow_mut()
                                .insert(host.id.clone(), friendly(&error));
                        }
                        app.reload();
                        app.sample_metrics();
                    },
                );
            }
            "trust" => {
                let backend = self.backend.clone();
                let id = host.id.clone();
                self.run(async move { backend.host_check_key(id).await }, move |app, result| match result {
                    Ok(check) => {
                        let mut facts = vec![fact("Host", host.display_name.as_str(), false), fact("Presented key", check.presented.as_str(), true)];
                        if let Some(pinned) = &check.pinned {
                            facts.push(fact("Trusted key", pinned.as_str(), true));
                        }
                        let (title, body, warning, confirm) = if check.matches {
                            ("The host key matches", "The host presented the key you already trust. Nothing needs to change.", "", "")
                        } else if check.pinned.is_some() {
                            (
                                "The host key has changed",
                                "Only continue if you know why the key changed, for example after reinstalling the host.",
                                "A different key can mean another machine is answering at this address.",
                                "Trust new key",
                            )
                        } else {
                            (
                                "Trust this host key?",
                                "Compare the key with the one shown on the host itself (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub).",
                                "",
                                "Trust this key",
                            )
                        };
                        let question = Question {
                            title: title.into(),
                            body: body.into(),
                            confirm: confirm.into(),
                            danger: check.pinned.is_some() && !check.matches,
                        };
                        app.ask_with(question, facts, warning, Pending::TrustKey(host.id.clone(), check.presented));
                    }
                    Err(error) => {
                        app.inv.errors.borrow_mut().insert(host.id.clone(), friendly(&error));
                        app.rebuild();
                    }
                });
            }
            "files" if host.connected => self.open_files(crate::files::Source::Host(host.id)),
            "desktop" => self.open_desktop(&host),
            "edit" if !host.is_local => {
                if self.host_form_leave(crate::hostform::Next::Open(Some(host.id.clone()))) {
                    self.host_form_open(Some(&host.id));
                }
            }
            "remove" if !host.is_local => self.host_remove_ask(&host),
            wsl if wsl.starts_with("wsl:") => self.open_shell("wsl", &wsl[4..]),
            _ => {}
        }
    }

    pub fn device_action(self: &Rc<Self>, route_id: &str, action: &str) {
        let Some(device) = self.inv.device(route_id) else {
            return;
        };
        let record = &device.record;
        if !matches!(action, "mirror" | "logcat" | "files") {
            self.device_tool_action(&device, action);
            return;
        }
        if record.state != "available" {
            return;
        }
        if action == "files" {
            self.open_files(crate::files::Source::Device(record.route_id.clone()));
            return;
        }
        let id = self.take_id();
        let route = format!(
            "{}  >  {}",
            record.host_name,
            transport_label(&record.connection_type)
        );
        let hue = self.inv.hue(&record.host_id);
        let pane = if action == "mirror" {
            crate::Pane::Mirror(crate::mirror::MirrorPane::start(
                id,
                self.backend.clone(),
                record.route_id.clone(),
                record.display_name.clone(),
                route,
                hue,
                crate::notifier(id),
            ))
        } else {
            crate::Pane::Logcat(crate::logcat::LogcatPane::start(
                id,
                self.backend.clone(),
                record.route_id.clone(),
                record.display_name.clone(),
                route,
                hue,
                crate::notifier(id),
            ))
        };
        self.tabs.push(TabInfo {
            id,
            kind: pane.kind().into(),
            title: if action == "mirror" {
                record.display_name.as_str().into()
            } else {
                format!("{} logcat", record.display_name).into()
            },
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.sessions.borrow_mut().push(pane);
        self.select(self.tabs.row_count() as i32 - 1);
    }

    /// The user accepted a host key in the dialog.
    pub fn trust_key(self: &Rc<Self>, host_id: String, fingerprint: String) {
        let backend = self.backend.clone();
        let id = host_id.clone();
        self.run(
            async move { backend.host_trust_key(id, fingerprint).await },
            move |app, result| {
                if let Err(error) = result {
                    app.inv
                        .errors
                        .borrow_mut()
                        .insert(host_id, friendly(&error));
                }
                app.reload();
            },
        );
    }

    /// Open a terminal tab on a remote host. The tab appears at once and shows progress.
    fn open_ssh(self: &Rc<Self>, host: &HostView, tmux: Option<String>) {
        let id = self.take_id();
        let session = self.start_ssh(id, host, tmux);
        self.sessions.borrow_mut().push(crate::Pane::Term(session));
        self.tabs.push(TabInfo {
            id,
            kind: "terminal".into(),
            title: host.display_name.as_str().into(),
            detail: SharedString::new(),
            hue: self.inv.hue(&host.id),
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }

    /// Start a terminal on a remote host, for a tab or a workspace panel. The session exists
    /// at once and shows progress; `tmux` names the tmux session to attach to or create.
    pub fn start_ssh(
        self: &Rc<Self>,
        id: i32,
        host: &HostView,
        tmux: Option<String>,
    ) -> term::Session {
        let (cols, lines) = self.grid.get();
        let (session, feeder, commands) =
            term::Session::remote(id, cols, lines, crate::notifier(id));
        feeder.feed(format!("Connecting to {}...\r\n", host.display_name).as_bytes());

        let backend = self.backend.clone();
        let request = ShellRequest::Ssh {
            host_id: host.id.clone(),
            tmux,
        };
        let sink: Sink<TermEvent> = {
            let feeder = feeder.clone();
            Arc::new(move |event| {
                match event {
                    TermEvent::Data(bytes) => feeder.feed(&bytes),
                    TermEvent::Exit(_) => feeder.end(),
                }
                true
            })
        };
        self.backend.rt.spawn(async move {
            match backend.term_open(request, cols, lines, sink).await {
                Ok(remote) => {
                    feeder.feed(b"\x1b[2J\x1b[H");
                    // Apply input and resizes in the order the user made them.
                    std::thread::spawn(move || {
                        for command in commands {
                            match command {
                                term::Cmd::Input(bytes) => {
                                    let _ = backend.term_write(remote, bytes);
                                }
                                term::Cmd::Resize(cols, lines) => {
                                    let _ = backend.term_resize(remote, cols, lines);
                                }
                                term::Cmd::Close => break,
                            }
                        }
                        let _ = backend.term_close(remote);
                    });
                    let _ = slint::invoke_from_event_loop(|| {
                        if let Some(app) = crate::app() {
                            app.reload();
                        }
                    });
                }
                Err(error) => {
                    feeder.feed(format!("\r\n{}\r\n", friendly(&error)).as_bytes());
                    feeder.end();
                }
            }
        });
        session
    }
}

fn base36(mut value: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_desktop_needs_a_connected_x11_session_with_x11vnc() {
        assert_eq!(desktop_problem(true, "x11", true), "");
        assert_eq!(
            desktop_problem(false, "x11", true),
            "Connect the host first."
        );
        assert!(desktop_problem(true, "x11", false).contains("sudo dnf install x11vnc"));
        assert!(desktop_problem(true, "wayland", true).contains("Wayland"));
        assert!(desktop_problem(true, "none", false).contains("text mode"));
    }
}
