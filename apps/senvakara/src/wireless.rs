//! "Add over Wi-Fi": pair with a phone's Wireless debugging (Android 11 and newer) and connect
//! to it, through a host whose ADB is being watched.

use std::cell::RefCell;
use std::rc::Rc;

use senvakara_app::CoreError;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{App, Inventory, Wireless, WirelessNote, WirelessService};

#[derive(Default)]
pub struct State {
    /// Host ids in the order of the picker.
    hosts: RefCell<Vec<String>>,
    found: Rc<VecModel<WirelessService>>,
    log: Rc<VecModel<WirelessNote>>,
    /// The address a scan last put in the pairing field, while it is still there.
    pair_from_scan: RefCell<String>,
    /// The same for the connect field.
    connect_from_scan: RefCell<String>,
}

/// Whether a scan may put an address in a field: when the user typed nothing there, or the
/// value is one an earlier scan put there (the phone's port changes every time).
fn scan_may_fill(current: &str, from_scan: &str) -> bool {
    current.trim().is_empty() || current == from_scan
}

/// A pairing code as the phone shows it: digits only, six at most.
pub fn code_digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).take(6).collect()
}

/// Why `address` cannot be used as "IP address and port", or None when it can.
pub fn address_problem(address: &str) -> Option<&'static str> {
    let address = address.trim();
    if address.is_empty() {
        return Some("Enter the IP address and port shown on the phone.");
    }
    let Some((host, port)) = address.rsplit_once(':') else {
        return Some("Add the port after the address, for example 192.0.2.20:37123.");
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Some("Enter the address before the port, for example 192.0.2.20:37123.");
    }
    match port.parse::<u16>() {
        Ok(port) if port > 0 => None,
        _ => Some("The port must be a number between 1 and 65535."),
    }
}

/// Why Pair cannot be pressed, or "" when it can.
pub fn pair_problem(address: &str, code: &str) -> &'static str {
    if let Some(problem) = address_problem(address) {
        return problem;
    }
    if code.len() != 6 {
        return "Enter the 6-digit code shown on the phone.";
    }
    ""
}

const PAIRING_OPEN: &str = "Pairing screen open";

/// What a discovered service means, from its mDNS type.
fn service_state(service: &str) -> &'static str {
    if service.contains("pairing") {
        PAIRING_OPEN
    } else {
        "Ready to connect"
    }
}

/// ADB's own wording, behind a sentence that says what to check.
fn said(lead: &str, error: &CoreError) -> String {
    if error.message.is_empty() {
        lead.to_string()
    } else {
        format!("{lead} ({})", error.message)
    }
}

impl App {
    pub fn init_wireless(self: &Rc<Self>, ui: &crate::MainWindow) {
        let wireless = ui.global::<Wireless>();
        wireless.set_found(self.wifi.found.clone().into());
        wireless.set_log(self.wifi.log.clone().into());
        ui.global::<Inventory>().on_add_wireless({
            let app = self.clone();
            move || app.wireless_open()
        });
        wireless.on_close({
            let app = self.clone();
            move || {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<Wireless>().set_open(false);
                }
            }
        });
        wireless.on_host_changed({
            let app = self.clone();
            move |index| {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<Wireless>().set_host_index(index);
                }
                app.wifi.found.set_vec(Vec::new());
                app.wireless_scan();
            }
        });
        wireless.on_edited({
            let app = self.clone();
            move || app.wireless_validate()
        });
        wireless.on_scan({
            let app = self.clone();
            move || app.wireless_scan()
        });
        wireless.on_pair({
            let app = self.clone();
            move || app.wireless_pair()
        });
        wireless.on_connect({
            let app = self.clone();
            move || app.wireless_connect()
        });
        wireless.on_use_found({
            let app = self.clone();
            move |index| app.wireless_use_found(index)
        });
    }

    /// The user picked a found phone: its address goes in the field for what it offers.
    fn wireless_use_found(&self, index: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(service) = usize::try_from(index)
            .ok()
            .and_then(|index| self.wifi.found.row_data(index))
        else {
            return;
        };
        let wireless = ui.global::<Wireless>();
        let address = service.address.to_string();
        if service.state == PAIRING_OPEN {
            wireless.set_pair_address(address.as_str().into());
            *self.wifi.pair_from_scan.borrow_mut() = address;
        } else {
            wireless.set_connect_address(address.as_str().into());
            *self.wifi.connect_from_scan.borrow_mut() = address;
        }
        self.wireless_validate();
    }

    fn wireless_open(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let wireless = ui.global::<Wireless>();
        let hosts = self.inv.adb_hosts();
        let names: Vec<SharedString> = hosts.iter().map(|(_, name)| name.into()).collect();
        *self.wifi.hosts.borrow_mut() = hosts.into_iter().map(|(id, _)| id).collect();
        wireless.set_host_names(ModelRc::new(VecModel::from(names)));
        wireless.set_host_index(0);
        wireless.set_pair_address(SharedString::new());
        wireless.set_code(SharedString::new());
        wireless.set_connect_address(SharedString::new());
        wireless.set_busy(SharedString::new());
        wireless.set_scanned(false);
        self.wifi.pair_from_scan.borrow_mut().clear();
        self.wifi.connect_from_scan.borrow_mut().clear();
        self.wifi.found.set_vec(Vec::new());
        self.wifi.log.set_vec(Vec::new());
        self.wireless_validate();
        wireless.set_open(true);
        self.wireless_scan();
    }

    /// The host chosen in the picker, while the dialog is open.
    fn wireless_host(&self) -> Option<(crate::MainWindow, String)> {
        let ui = self.ui.upgrade()?;
        let wireless = ui.global::<Wireless>();
        if !wireless.get_open() {
            return None;
        }
        let host = self
            .wifi
            .hosts
            .borrow()
            .get(wireless.get_host_index().max(0) as usize)
            .cloned()?;
        Some((ui, host))
    }

    /// Keep the code to six digits and say why a button cannot be used yet.
    fn wireless_validate(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let wireless = ui.global::<Wireless>();
        let code = code_digits(&wireless.get_code());
        if code != wireless.get_code().as_str() {
            wireless.set_code(code.as_str().into());
        }
        let no_host = if self.wifi.hosts.borrow().is_empty() {
            "No host has ADB running. Connect a host first."
        } else {
            ""
        };
        wireless.set_host_reason(no_host.into());
        let pair = pair_problem(&wireless.get_pair_address(), &code);
        let connect = address_problem(&wireless.get_connect_address()).unwrap_or("");
        let pick = |own: &'static str| if no_host.is_empty() { own } else { no_host };
        wireless.set_pair_reason(pick(pair).into());
        wireless.set_connect_reason(pick(connect).into());
    }

    fn wireless_note(&self, ok: bool, text: String) {
        // Newest first: the list is short and the latest result is what the user looks for.
        self.wifi.log.insert(
            0,
            WirelessNote {
                ok,
                text: text.into(),
            },
        );
    }

    fn wireless_scan(self: &Rc<Self>) {
        let Some((ui, host)) = self.wireless_host() else {
            return;
        };
        let wireless = ui.global::<Wireless>();
        if wireless.get_busy() != "" {
            return;
        }
        wireless.set_busy("scan".into());
        let backend = self.backend.clone();
        let asked = host.clone();
        self.run(
            async move { backend.wireless_scan(host).await },
            move |app, result| {
                let Some((ui, host)) = app.wireless_host() else {
                    return;
                };
                let wireless = ui.global::<Wireless>();
                wireless.set_busy(SharedString::new());
                if host != asked {
                    // The picker moved on while this scan ran: scan the host it shows now.
                    app.wireless_scan();
                    return;
                }
                wireless.set_scanned(true);
                match result {
                    Ok(services) => {
                        let address_of = |kind: &str| {
                            services
                                .iter()
                                .find(|service| service.service.contains(kind))
                                .map(|service| service.address.clone())
                        };
                        if scan_may_fill(
                            &wireless.get_pair_address(),
                            &app.wifi.pair_from_scan.borrow(),
                        ) {
                            if let Some(address) = address_of("pairing") {
                                wireless.set_pair_address(address.as_str().into());
                                *app.wifi.pair_from_scan.borrow_mut() = address;
                            }
                        }
                        if scan_may_fill(
                            &wireless.get_connect_address(),
                            &app.wifi.connect_from_scan.borrow(),
                        ) {
                            if let Some(address) = address_of("connect") {
                                wireless.set_connect_address(address.as_str().into());
                                *app.wifi.connect_from_scan.borrow_mut() = address;
                            }
                        }
                        app.wifi.found.set_vec(
                            services
                                .into_iter()
                                .map(|service| WirelessService {
                                    address: service.address.into(),
                                    state: service_state(&service.service).into(),
                                    name: service.name.into(),
                                })
                                .collect::<Vec<_>>(),
                        );
                        app.wireless_validate();
                    }
                    Err(error) => {
                        app.wifi.found.set_vec(Vec::new());
                        app.wireless_note(
                            false,
                            said("Could not look for phones on that host's network.", &error),
                        );
                    }
                }
            },
        );
    }

    fn wireless_pair(self: &Rc<Self>) {
        let Some((ui, host)) = self.wireless_host() else {
            return;
        };
        let wireless = ui.global::<Wireless>();
        if wireless.get_busy() != "" || wireless.get_pair_reason() != "" {
            return;
        }
        let address = wireless.get_pair_address().trim().to_string();
        let code = wireless.get_code().to_string();
        wireless.set_busy("pair".into());
        let backend = self.backend.clone();
        let shown = address.clone();
        self.run(
            async move { backend.wireless_pair(host, address, code).await },
            move |app, result| {
                let Some(ui) = app.ui.upgrade() else { return };
                let wireless = ui.global::<Wireless>();
                wireless.set_busy(SharedString::new());
                match result {
                    Ok(_) => {
                        app.wireless_note(
                            true,
                            format!("Paired with {shown}. Now connect to it below."),
                        );
                        // The code is used up; never keep it around.
                        wireless.set_code(SharedString::new());
                        app.wireless_validate();
                        app.wireless_scan();
                    }
                    Err(error) => app.wireless_note(
                        false,
                        said(
                            "Pairing failed. Check the address and the code on the phone's pairing screen.",
                            &error,
                        ),
                    ),
                }
            },
        );
    }

    fn wireless_connect(self: &Rc<Self>) {
        let Some((ui, host)) = self.wireless_host() else {
            return;
        };
        let wireless = ui.global::<Wireless>();
        if wireless.get_busy() != "" || wireless.get_connect_reason() != "" {
            return;
        }
        let address = wireless.get_connect_address().trim().to_string();
        wireless.set_busy("connect".into());
        let backend = self.backend.clone();
        let shown = address.clone();
        self.run(
            async move { backend.wireless_connect(host, address).await },
            move |app, result| {
                let Some(ui) = app.ui.upgrade() else { return };
                ui.global::<Wireless>().set_busy(SharedString::new());
                match result {
                    Ok(_) => {
                        app.wireless_note(
                            true,
                            format!("Connected to {shown}. The phone is now listed under Devices."),
                        );
                        app.reload();
                    }
                    Err(error) => app.wireless_note(
                        false,
                        said(
                            "Could not connect. Check the address and that the phone is paired with this host.",
                            &error,
                        ),
                    ),
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_keeps_six_digits() {
        assert_eq!(code_digits("12 34-56 78"), "123456");
        assert_eq!(code_digits("abc"), "");
        assert_eq!(code_digits("0042"), "0042");
    }

    #[test]
    fn address_needs_host_and_port() {
        assert_eq!(address_problem("192.168.1.20:37123"), None);
        assert_eq!(address_problem("  [fe80::1]:5555 "), None);
        assert!(address_problem("").is_some());
        assert!(address_problem("192.168.1.20").is_some());
        assert!(address_problem(":5555").is_some());
        assert!(address_problem("host:0").is_some());
        assert!(address_problem("host:99999").is_some());
        assert!(address_problem("a b:1").is_some());
    }

    #[test]
    fn pair_needs_address_then_code() {
        assert!(pair_problem("", "123456").contains("IP address"));
        assert!(pair_problem("192.168.1.20:37123", "123").contains("6-digit"));
        assert_eq!(pair_problem("192.168.1.20:37123", "123456"), "");
    }

    #[test]
    fn a_scan_fills_what_the_user_did_not_type() {
        assert!(scan_may_fill("", ""));
        assert!(scan_may_fill("  ", "192.0.2.20:37123"));
        // The earlier scan's port is replaced by the new one.
        assert!(scan_may_fill("192.0.2.20:37123", "192.0.2.20:37123"));
        // Typed or edited by the user: left alone.
        assert!(!scan_may_fill("192.0.2.20:4444", "192.0.2.20:37123"));
        assert!(!scan_may_fill("192.0.2.20:4444", ""));
    }

    #[test]
    fn service_kinds() {
        assert_eq!(
            service_state("_adb-tls-pairing._tcp"),
            "Pairing screen open"
        );
        assert_eq!(service_state("_adb-tls-connect._tcp"), "Ready to connect");
    }
}
