//! The host side of Home's state: lookups over the host list and the card built for each host.

use senvakara_app::{HostView, Protocol};
use slint::{ModelRc, SharedString, VecModel};

use crate::{Action, HostItem, KeyValue, PathItem, VpnHint};

use super::metrics::meters;
use super::text::{action, blocked, fact};
use super::State;

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

/// What a card says under a remote host's name: how it is opened and as whom ("ssh, bob").
pub fn host_subtitle(protocol: Protocol, username: &str) -> String {
    match username.trim() {
        "" => protocol.as_str().to_string(),
        username => format!("{}, {username}", protocol.as_str()),
    }
}

/// The card of a Remote Desktop host: (subtitle, actions, details). It is opened in
/// Windows' own client, so it has no connection state, terminal, files or host key.
fn rdp_card(host: &HostView) -> (String, Vec<Action>, Vec<KeyValue>) {
    let rdp = host.rdp.as_ref();
    let username = rdp.map(|rdp| rdp.username.as_str()).unwrap_or("");
    let mut details = Vec::new();
    if let Some(rdp) = rdp {
        details.push(fact(
            "Address",
            format!("{}:{}", rdp.address, rdp.port),
            true,
        ));
        if !rdp.username.is_empty() {
            details.push(fact("User", rdp.username.as_str(), true));
        }
        details.push(fact(
            "Password",
            if rdp.has_credential {
                "Saved in Windows Credential Manager"
            } else {
                "Not saved: Remote Desktop asks for it"
            },
            false,
        ));
    }
    let actions = vec![
        action("rdp", "Open in Remote Desktop"),
        action("edit", "Edit"),
        Action {
            danger: true,
            ..action("remove", "Remove host")
        },
    ];
    (host_subtitle(Protocol::Rdp, username), actions, details)
}

impl State {
    /// Every host, as the backend last reported them.
    pub fn hosts(&self) -> Vec<HostView> {
        self.hosts.borrow().clone()
    }

    /// Installed WSL distributions, default first.
    pub fn wsl(&self) -> Vec<String> {
        self.wsl.borrow().clone()
    }

    /// The WSL distributions the shell detection found. The caller redraws Home.
    pub fn set_wsl(&self, distros: Vec<String>) {
        *self.wsl.borrow_mut() = distros;
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

    pub(super) fn host_item(&self, host: &HostView) -> HostItem {
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
        } else if host.protocol == Protocol::Rdp {
            (subtitle, actions, details) = rdp_card(host);
            tone = if error.is_empty() {
                "idle"
            } else {
                "attention"
            };
            state_text = "Remote Desktop".to_string();
        } else {
            let ssh = host.ssh.as_ref();
            let who = host_subtitle(
                Protocol::Ssh,
                ssh.map(|ssh| ssh.username.as_str()).unwrap_or(""),
            );
            subtitle = match text("os_pretty_name") {
                Some(os) if host.connected => format!("{who}  ·  {os}"),
                _ => who,
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
            os: if host.protocol == Protocol::Rdp {
                "rdp"
            } else if host.os_family == "windows" {
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

    #[test]
    fn a_card_names_the_protocol_and_the_user() {
        assert_eq!(host_subtitle(Protocol::Ssh, "bob"), "ssh, bob");
        assert_eq!(host_subtitle(Protocol::Rdp, " alice "), "rdp, alice");
        assert_eq!(host_subtitle(Protocol::Rdp, ""), "rdp");
    }
}
