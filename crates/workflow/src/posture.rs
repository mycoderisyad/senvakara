//! Host security posture (plan WF §5): one read-only script that looks at a
//! host without sudo, and the reading of its report. Each control carries the
//! evidence it was judged on and the module that fixes it. This is a
//! hardening overview, not a CIS benchmark: nothing here claims compliance.

use serde::{Deserialize, Serialize};

/// The read-only check, run as the signed-in user (no sudo, no changes).
pub const POSTURE_SCRIPT: &str = include_str!("../posture/posture.sh");

/// Longest evidence kept per control.
const MAX_EVIDENCE: usize = 200;

/// One control the posture check reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// The catalog module that sets the control right, if one does.
    pub fix: Option<&'static str>,
    /// Counted in "Hardened N/M"; information only otherwise.
    pub scored: bool,
}

/// Every control, in the order the inspector lists them.
pub const CONTROLS: [ControlInfo; 10] = [
    ControlInfo {
        id: "root_login",
        label: "Root cannot sign in with a password",
        fix: Some("security.ssh_hardening"),
        scored: true,
    },
    ControlInfo {
        id: "password_auth",
        label: "SSH password sign-in off",
        fix: Some("security.ssh_hardening"),
        scored: true,
    },
    ControlInfo {
        id: "firewall",
        label: "Firewall on, incoming denied by default",
        fix: Some("networking.firewall"),
        scored: true,
    },
    ControlInfo {
        id: "auto_updates",
        label: "Automatic security updates",
        fix: Some("security.unattended_upgrades"),
        scored: true,
    },
    ControlInfo {
        id: "fail2ban",
        label: "fail2ban running",
        fix: Some("security.fail2ban"),
        scored: true,
    },
    ControlInfo {
        id: "security_updates",
        label: "No security updates pending",
        fix: Some("security.system_update"),
        scored: true,
    },
    ControlInfo {
        id: "reboot_required",
        label: "No reboot pending",
        fix: None,
        scored: true,
    },
    ControlInfo {
        id: "sudo_user",
        label: "A non-root user with sudo",
        fix: Some("security.sudo_user"),
        scored: true,
    },
    ControlInfo {
        id: "time_sync",
        label: "Clock synchronised",
        fix: Some("networking.time_sync"),
        scored: true,
    },
    ControlInfo {
        id: "listening_ports",
        label: "Listening ports",
        fix: Some("networking.firewall"),
        scored: false,
    },
];

pub fn control_info(id: &str) -> Option<&'static ControlInfo> {
    CONTROLS.iter().find(|c| c.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    /// The host did not say, or reading it needs root.
    Unknown,
    /// Shown, not judged.
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub id: String,
    pub status: Status,
    pub evidence: String,
}

/// What one posture check found, as cached per host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posture {
    /// `/etc/os-release` `ID` and `VERSION_ID`.
    #[serde(default)]
    pub os: Option<(String, String)>,
    pub controls: Vec<Control>,
}

impl Posture {
    /// Reads the script's output. Every known control appears once, in
    /// [`CONTROLS`] order; one the output does not mention is unknown.
    /// Unknown lines are ignored, so an older app reads a newer script.
    ///
    /// Fails when the output holds no control at all (the script did not
    /// run), so an empty report never looks like a verdict.
    pub fn parse(stdout: &str) -> Result<Self, String> {
        let mut os = None;
        let mut found: Vec<Control> = Vec::new();
        for line in stdout.lines() {
            let mut parts = line.splitn(4, '\t');
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some("SV_POSTURE_OS"), Some(id), Some(version), None) => {
                    os = Some((clean(id), clean(version)));
                }
                (Some("SV_POSTURE"), Some(id), Some(status), evidence) => {
                    let Some(info) = control_info(id) else {
                        continue;
                    };
                    if found.iter().any(|c| c.id == info.id) {
                        continue;
                    }
                    let status = match (status, info.scored) {
                        ("pass", true) => Status::Pass,
                        ("fail", true) => Status::Fail,
                        ("info", false) => Status::Info,
                        _ => Status::Unknown,
                    };
                    found.push(Control {
                        id: info.id.to_string(),
                        status,
                        evidence: clean(evidence.unwrap_or_default()),
                    });
                }
                _ => {}
            }
        }
        if found.is_empty() {
            return Err("the posture check printed no result".into());
        }
        let controls = CONTROLS
            .iter()
            .map(|info| {
                found
                    .iter()
                    .find(|c| c.id == info.id)
                    .cloned()
                    .unwrap_or_else(|| Control {
                        id: info.id.to_string(),
                        status: Status::Unknown,
                        evidence: "not reported by the host".into(),
                    })
            })
            .collect();
        Ok(Self { os, controls })
    }

    /// (controls that pass, controls that count): "Hardened N/M".
    pub fn score(&self) -> (usize, usize) {
        let scored = self
            .controls
            .iter()
            .filter(|c| control_info(&c.id).is_some_and(|i| i.scored));
        let (mut pass, mut all) = (0, 0);
        for c in scored {
            all += 1;
            if c.status == Status::Pass {
                pass += 1;
            }
        }
        (pass, all)
    }
}

/// Evidence comes from the host: printable characters only, bounded.
fn clean(text: &str) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = text.trim();
    match text.char_indices().nth(MAX_EVIDENCE) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "SV_POSTURE_OS\tubuntu\t24.04\n\
        SV_POSTURE\troot_login\tpass\tPermitRootLogin no\n\
        SV_POSTURE\tpassword_auth\tfail\tPasswordAuthentication yes (default), KbdInteractiveAuthentication no\n\
        SV_POSTURE\tfirewall\tunknown\tnftables is active; its rules need root to read\n\
        SV_POSTURE\tfail2ban\tpass\tfail2ban service active\n\
        SV_POSTURE\tlistening_ports\tinfo\treachable from outside: 22; this host only: 631\n\
        SV_POSTURE\tfuture_control\tpass\tsomething new\n\
        noise from a login banner\n";

    #[test]
    fn a_report_lists_every_control_in_order_with_missing_ones_unknown() {
        let p = Posture::parse(SAMPLE).unwrap();
        assert_eq!(p.os, Some(("ubuntu".into(), "24.04".into())));
        let ids: Vec<&str> = p.controls.iter().map(|c| c.id.as_str()).collect();
        let expected: Vec<&str> = CONTROLS.iter().map(|c| c.id).collect();
        assert_eq!(ids, expected);
        let status = |id: &str| p.controls.iter().find(|c| c.id == id).unwrap().status;
        assert_eq!(status("root_login"), Status::Pass);
        assert_eq!(status("password_auth"), Status::Fail);
        assert_eq!(status("firewall"), Status::Unknown);
        assert_eq!(status("time_sync"), Status::Unknown);
        assert_eq!(status("listening_ports"), Status::Info);
        let missing = p.controls.iter().find(|c| c.id == "sudo_user").unwrap();
        assert_eq!(missing.evidence, "not reported by the host");
    }

    #[test]
    fn the_score_counts_passes_over_scored_controls_only() {
        let p = Posture::parse(SAMPLE).unwrap();
        // root_login and fail2ban pass; listening_ports is information.
        assert_eq!(p.score(), (2, 9));
    }

    #[test]
    fn an_empty_or_foreign_output_is_an_error_not_a_verdict() {
        assert!(Posture::parse("").is_err());
        assert!(Posture::parse("bash: posture: command not found\n").is_err());
    }

    #[test]
    fn a_host_cannot_claim_a_verdict_for_an_information_control() {
        let p = Posture::parse("SV_POSTURE\tlistening_ports\tpass\tall good\n").unwrap();
        let ports = p.controls.iter().find(|c| c.id == "listening_ports");
        assert_eq!(ports.unwrap().status, Status::Unknown);
        let p = Posture::parse("SV_POSTURE\troot_login\tinfo\t?\n").unwrap();
        assert_eq!(p.controls[0].status, Status::Unknown);
    }

    #[test]
    fn evidence_is_printable_and_bounded() {
        let long = "x".repeat(500);
        let text = format!("SV_POSTURE\troot_login\tpass\t\u{1b}[31m{long}\n");
        let p = Posture::parse(&text).unwrap();
        let evidence = &p.controls[0].evidence;
        assert!(!evidence.contains('\u{1b}'));
        assert_eq!(evidence.chars().count(), MAX_EVIDENCE + 1);
    }

    #[test]
    fn every_fix_names_a_bundled_module() {
        let catalog = crate::Catalog::bundled().unwrap();
        for c in CONTROLS {
            if let Some(fix) = c.fix {
                assert!(catalog.get(fix).is_some(), "{} -> {fix}", c.id);
            }
        }
    }

    #[test]
    fn a_stored_posture_reads_back() {
        let p = Posture::parse(SAMPLE).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Posture>(&json).unwrap(), p);
    }
}
