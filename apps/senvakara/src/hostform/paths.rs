//! The network paths of the selected host: adding and removing its VPN addresses.

use std::rc::Rc;

use senvakara_app::HostView;
use slint::{ComponentHandle, SharedString};

use crate::inventory::{fact, friendly_with};
use crate::{App, NetPaths, Pending, Question};

use super::validation::{parse_port, vpn_problem};

impl App {
    /// Called whenever the selection is published: another host starts with empty fields.
    pub fn paths_sync(&self, host_id: &str) {
        if *self.hosts.paths_host.borrow() == host_id {
            return;
        }
        *self.hosts.paths_host.borrow_mut() = host_id.to_string();
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let port = self
            .inv
            .host(host_id)
            .and_then(|host| host.ssh.map(|ssh| ssh.port))
            .unwrap_or(22);
        paths.set_address(SharedString::new());
        paths.set_port(port.to_string().into());
        paths.set_error(SharedString::new());
        self.paths_validate();
    }

    /// (host, why the typed address cannot be added)
    fn paths_check(&self, address: &str, port: &str) -> Option<(HostView, &'static str)> {
        let host = self.inv.host(&self.hosts.paths_host.borrow())?;
        let mut known: Vec<String> = host.vpn.iter().map(|vpn| vpn.address.clone()).collect();
        if let Some(ssh) = &host.ssh {
            known.push(ssh.address.clone());
        }
        let problem = vpn_problem(address, port, &known, host.vpn.len());
        Some((host, problem))
    }

    pub(super) fn paths_validate(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let reason = self
            .paths_check(&paths.get_address(), &paths.get_port())
            .map(|(_, problem)| problem)
            .unwrap_or("Select a host first.");
        paths.set_add_reason(reason.into());
    }

    /// Add the typed address, or `hint`: an address the host itself reported, on its SSH port.
    pub(super) fn paths_add(self: &Rc<Self>, hint: Option<String>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let (address, port) = match hint {
            Some(address) => {
                let port = self
                    .inv
                    .host(&self.hosts.paths_host.borrow())
                    .and_then(|host| host.ssh.map(|ssh| ssh.port))
                    .unwrap_or(22);
                (address, port.to_string())
            }
            None => (
                paths.get_address().to_string(),
                paths.get_port().to_string(),
            ),
        };
        let Some((host, problem)) = self.paths_check(&address, &port) else {
            return;
        };
        if !problem.is_empty() {
            paths.set_error(problem.into());
            return;
        }
        let Ok(port) = parse_port(&port) else { return };
        match self
            .backend
            .host_add_vpn(host.id, address.trim().to_string(), port)
        {
            Ok(_) => {
                paths.set_address(SharedString::new());
                paths.set_error(SharedString::new());
            }
            Err(error) => {
                paths.set_error(friendly_with(&error, "This host already has that address.").into())
            }
        }
        self.reload();
        self.paths_validate();
    }

    pub(super) fn paths_remove_ask(self: &Rc<Self>, endpoint_id: &str) {
        let Some(host) = self.inv.host(&self.hosts.paths_host.borrow()) else {
            return;
        };
        let Some(endpoint) = host.vpn.iter().find(|vpn| vpn.endpoint_id == endpoint_id) else {
            return;
        };
        self.ask_with(
            Question {
                title: "Remove this VPN address?".into(),
                body: "Senvakara stops trying this address when the host does not answer on its main one. The VPN itself is not changed.".into(),
                confirm: "Remove address".into(),
                danger: true,
            },
            vec![
                fact("Host", host.display_name.as_str(), false),
                fact(
                    "VPN address",
                    format!("{}:{}", endpoint.address, endpoint.port),
                    true,
                ),
            ],
            "",
            Pending::RemoveVpn {
                host_id: host.id.clone(),
                endpoint_id: endpoint_id.to_string(),
            },
        );
    }

    pub fn paths_remove(self: &Rc<Self>, host_id: String, endpoint_id: String) {
        let result = self.backend.host_remove_vpn(host_id, endpoint_id);
        if let (Err(error), Some(ui)) = (result, self.ui.upgrade()) {
            ui.global::<NetPaths>()
                .set_error(friendly_with(&error, "The address could not be removed.").into());
        }
        self.reload();
        self.paths_validate();
    }
}
