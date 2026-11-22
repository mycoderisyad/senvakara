//! Whether a panel can run with the hosts and phones as they are, and what to say if not.

use senvakara_app::HostView;
use serde_json::Value;

use crate::inventory::{desktop_problem, transport_label};

use super::{DeviceView, Shell, Spec};

/// Why a panel cannot run right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocked {
    pub text: String,
    /// The host whose connection would fix it.
    pub connect: Option<String>,
}

pub(super) fn route_text(device: &DeviceView) -> String {
    format!(
        "{} > {}",
        device.record.host_name,
        transport_label(&device.record.connection_type)
    )
}

pub(super) fn trusted(host: &HostView) -> bool {
    host.trust_state == "trusted"
}

fn host_blocked(hosts: &[HostView], host_id: &str) -> Option<Blocked> {
    let Some(host) = hosts.iter().find(|host| host.id == host_id) else {
        return Some(Blocked {
            text: "This host was removed from Senvakara. Close the panel and add another.".into(),
            connect: None,
        });
    };
    if host.is_local || host.connected {
        return None;
    }
    if !trusted(host) {
        return Some(Blocked {
            text: format!(
                "The key of {} is not trusted any more. Verify it on Home, then connect.",
                host.display_name
            ),
            connect: None,
        });
    }
    Some(Blocked {
        text: format!("{} is not connected.", host.display_name),
        connect: Some(host.id.clone()),
    })
}

/// Why `spec` cannot run with the hosts and phones as they are, or `None` when it can.
pub fn blocked(spec: &Spec, hosts: &[HostView], devices: &[DeviceView]) -> Option<Blocked> {
    match spec {
        Spec::Terminal {
            shell: Shell::Ssh { host_id, .. },
            ..
        } => host_blocked(hosts, host_id),
        Spec::Terminal { .. } => None,
        Spec::Files { host_id } => host_blocked(hosts, host_id),
        Spec::Desktop { host_id } => host_blocked(hosts, host_id).or_else(|| {
            let host = hosts.iter().find(|host| host.id == *host_id)?;
            let desktop = host.capabilities.get("desktop");
            let session = desktop
                .and_then(|d| d.get("session"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            let has_x11vnc = desktop
                .and_then(|d| d.get("x11vnc"))
                .is_some_and(|v| v.as_str().is_some_and(|path| !path.is_empty()));
            let problem = desktop_problem(true, session, has_x11vnc);
            (!problem.is_empty()).then(|| Blocked {
                text: problem.to_string(),
                connect: None,
            })
        }),
        Spec::Mirror { route_id } | Spec::Logcat { route_id } => {
            let Some(device) = devices
                .iter()
                .find(|device| device.record.route_id == *route_id)
            else {
                return Some(Blocked {
                    text: "This phone was forgotten. Close the panel and add another.".into(),
                    connect: None,
                });
            };
            let record = &device.record;
            let name = format!("{} ({})", record.display_name, route_text(device));
            match record.state.as_str() {
                "available" => None,
                "unauthorized" => Some(Blocked {
                    text: format!(
                        "{name} has not allowed USB debugging from that computer. Unlock the phone and accept the prompt."
                    ),
                    connect: None,
                }),
                _ => Some(Blocked {
                    text: format!("{name} is not connected."),
                    connect: hosts
                        .iter()
                        .find(|host| host.id == record.host_id)
                        .filter(|host| !host.is_local && !host.connected && trusted(host))
                        .map(|host| host.id.clone()),
                }),
            }
        }
        Spec::Unavailable { panel_type, .. } => Some(Blocked {
            text: format!(
                "This saved panel ({panel_type}) could not be read. Close it and add another."
            ),
            connect: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_terminals_always_run_and_missing_targets_say_why() {
        let local = Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows,
        };
        assert_eq!(blocked(&local, &[], &[]), None);
        let gone = blocked(
            &Spec::Files {
                host_id: "host_x".into(),
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(gone.text.contains("removed"));
        assert_eq!(gone.connect, None);
        let forgotten = blocked(
            &Spec::Logcat {
                route_id: "route_x".into(),
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(forgotten.text.contains("forgotten"));
    }
}
