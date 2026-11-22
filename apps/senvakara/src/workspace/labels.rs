//! What panels are called: header titles, icons, the picker entries and the list summary.

use senvakara_app::HostView;
use serde_json::Value;

use super::availability::{route_text, trusted};
use super::{DeviceView, Shell, Spec};

fn host_name(hosts: &[HostView], id: &str) -> String {
    hosts
        .iter()
        .find(|host| host.id == id)
        .map(|host| host.display_name.clone())
        .unwrap_or_else(|| "a removed host".into())
}

fn device_name(devices: &[DeviceView], route_id: &str) -> String {
    devices
        .iter()
        .find(|device| device.record.route_id == route_id)
        .map(|device| format!("{} ({})", device.record.display_name, route_text(device)))
        .unwrap_or_else(|| "a forgotten phone".into())
}

/// What the header of a panel says.
pub fn panel_title(spec: &Spec, hosts: &[HostView], devices: &[DeviceView]) -> String {
    match spec {
        Spec::Terminal { title, .. } => title.clone(),
        Spec::Desktop { host_id } => format!("Desktop of {}", host_name(hosts, host_id)),
        Spec::Files { host_id } => format!("Files on {}", host_name(hosts, host_id)),
        Spec::Mirror { route_id } => format!("Screen of {}", device_name(devices, route_id)),
        Spec::Logcat { route_id } => format!("Logcat of {}", device_name(devices, route_id)),
        Spec::Unavailable { panel_type, .. } => format!("Unavailable panel ({panel_type})"),
    }
}

/// The view that draws a panel, which is also its icon.
pub fn panel_kind(spec: &Spec) -> &'static str {
    match spec {
        Spec::Terminal { .. } | Spec::Unavailable { .. } => "terminal",
        Spec::Mirror { .. } => "mirror",
        Spec::Logcat { .. } => "logcat",
        Spec::Desktop { .. } => "desktop",
        Spec::Files { .. } => "files",
    }
}

/// Everything an empty panel can become, in the order of the picker. A tmux entry carries an
/// empty session name: the name is made when the panel is added.
pub fn panel_options(
    hosts: &[HostView],
    devices: &[DeviceView],
    distros: &[String],
) -> Vec<(String, Spec)> {
    let mut options = vec![(
        "Terminal: PowerShell".to_string(),
        Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows,
        },
    )];
    for distro in distros {
        options.push((
            format!("Terminal: WSL {distro}"),
            Spec::Terminal {
                title: format!("WSL {distro}"),
                shell: Shell::Wsl(Some(distro.clone())),
            },
        ));
    }
    for host in hosts.iter().filter(|host| !host.is_local && trusted(host)) {
        let name = &host.display_name;
        options.push((
            format!("Terminal: SSH to {name}"),
            Spec::Terminal {
                title: format!("SSH {name}"),
                shell: Shell::Ssh {
                    host_id: host.id.clone(),
                    tmux: None,
                },
            },
        ));
        if host
            .capabilities
            .get("tmux")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            options.push((
                format!("Terminal: tmux on {name} (survives disconnects)"),
                Spec::Terminal {
                    title: format!("tmux {name}"),
                    shell: Shell::Ssh {
                        host_id: host.id.clone(),
                        tmux: Some(String::new()),
                    },
                },
            ));
        }
        options.push((
            format!("Files: {name}"),
            Spec::Files {
                host_id: host.id.clone(),
            },
        ));
        options.push((
            format!("Desktop: {name}"),
            Spec::Desktop {
                host_id: host.id.clone(),
            },
        ));
    }
    for device in devices {
        let name = format!("{} ({})", device.record.display_name, route_text(device));
        let route_id = device.record.route_id.clone();
        options.push((
            format!("Screen: {name}"),
            Spec::Mirror {
                route_id: route_id.clone(),
            },
        ));
        options.push((format!("Logcat: {name}"), Spec::Logcat { route_id }));
    }
    options
}

/// One line naming the panels of a saved workspace.
pub fn summary(slots: &[Option<Spec>], hosts: &[HostView], devices: &[DeviceView]) -> String {
    let names: Vec<String> = slots
        .iter()
        .flatten()
        .map(|spec| panel_title(spec, hosts, devices))
        .collect();
    if names.is_empty() {
        "No panels".into()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_always_offers_local_shells() {
        let options = panel_options(&[], &[], &["Ubuntu".to_string()]);
        let labels: Vec<&str> = options.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Terminal: PowerShell", "Terminal: WSL Ubuntu"]);
    }
}
