//! Quick connect: an SSH target typed into the Hosts search field (`ssh user@host`,
//! `ssh -p 2222 user@host`, `user@host:2222`) opens the saved host it means, or the host form
//! filled in for a new one.

use std::rc::Rc;

use senvakara_app::{HostView, Protocol};
use slint::{ComponentHandle, SharedString};

use crate::hostform::{address_problem, parse_port, Next};
use crate::{App, Inventory};

/// A parsed SSH destination from the Hosts search and quick-connect bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTarget {
    pub username: Option<String>,
    pub address: String,
    pub port: u16,
}

impl SshTarget {
    /// Human-readable target such as `root@192.168.1.10` or `root@192.168.1.10:2222`.
    pub fn display(&self) -> String {
        let host = if self.address.contains(':') {
            format!("[{}]", self.address)
        } else {
            self.address.clone()
        };
        let mut out = match &self.username {
            Some(user) => format!("{user}@{host}"),
            None => host,
        };
        if self.port != 22 {
            out.push_str(&format!(":{}", self.port));
        }
        out
    }
}

/// Parse an SSH quick-connect string into a target, or `None` when `input` is a normal search
/// query or is not a valid SSH target.
///
/// Accepted forms:
/// - `ssh user@host`, `ssh host`, `ssh -p 2222 user@host`, `ssh -l user host`
/// - `ssh user@host:2222`, `ssh user@[::1]:2222`
/// - `user@host`, `user@host:2222`, `user@[::1]:2222` (shorthand without `ssh`)
pub fn parse_ssh_target(input: &str) -> Option<SshTarget> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let command = match tokens.as_slice() {
        [ssh, arguments @ ..] if ssh.eq_ignore_ascii_case("ssh") => ssh_arguments(arguments)?,
        [destination] if destination.contains('@') => SshCommand {
            destination,
            username: None,
            port: None,
        },
        _ => return None,
    };

    let (username, host) = match command.destination.split_once('@') {
        // The user is given once: before the "@" or with -l.
        Some(_) if command.username.is_some() => return None,
        Some((user, host)) => (Some(valid_username(user)?), host),
        None => (command.username, command.destination),
    };
    let (address, port) = split_host_port(host)?;
    // The port may be given twice (-p and after the address) only when both agree.
    if matches!((command.port, port), (Some(flag), Some(inline)) if flag != inline) {
        return None;
    }
    if !address_problem(address).is_empty() || address.chars().any(char::is_control) {
        return None;
    }
    Some(SshTarget {
        username: username.map(str::to_string),
        address: address.to_string(),
        port: command.port.or(port).unwrap_or(22),
    })
}

/// What follows `ssh` on a command line, as far as quick connect reads it.
struct SshCommand<'a> {
    /// `host`, `user@host` or either with `:port`.
    destination: &'a str,
    /// The value of `-l`.
    username: Option<&'a str>,
    /// The value of `-p`.
    port: Option<u16>,
}

/// Read the arguments of an `ssh` command: one destination, with `-p` and `-l` before or after
/// it, each at most once and with its value attached (`-p2222`) or next (`-p 2222`). `None`
/// for any other option, a second destination or no destination.
fn ssh_arguments<'a>(arguments: &[&'a str]) -> Option<SshCommand<'a>> {
    let (mut destination, mut username, mut port) = (None, None, None);
    let mut rest = arguments.iter().copied();
    while let Some(argument) = rest.next() {
        // The value of an option is the rest of its argument or, failing that, the next one.
        let mut value = |attached: &'a str| match attached {
            "" => rest.next(),
            attached => Some(attached),
        };
        if let Some(attached) = argument.strip_prefix("-p") {
            if port.is_some() {
                return None;
            }
            port = Some(parse_port(value(attached)?).ok()?);
        } else if let Some(attached) = argument.strip_prefix("-l") {
            if username.is_some() {
                return None;
            }
            username = Some(valid_username(value(attached)?)?);
        } else if argument.starts_with('-') || destination.is_some() {
            return None;
        } else {
            destination = Some(argument);
        }
    }
    Some(SshCommand {
        destination: destination?,
        username,
        port,
    })
}

/// Find the one saved SSH host `target` means: by address and port (the main address or a
/// VPN one) and, when a user was typed, by user; failing that, by its name when only a name
/// was typed. `None` when no host matches and when more than one does (two accounts on the
/// same address and no user typed), because picking one would be a guess.
pub fn find_matching_host<'a>(hosts: &'a [HostView], target: &SshTarget) -> Option<&'a HostView> {
    let saved = || {
        hosts
            .iter()
            .filter(|host| !host.is_local && host.protocol == Protocol::Ssh)
    };
    let at_endpoint = |host: &&HostView| {
        let Some(ssh) = &host.ssh else {
            return false;
        };
        let user_fits = target
            .username
            .as_deref()
            .is_none_or(|user| user == ssh.username);
        let is_target = |address: &str, port: u16| {
            address.eq_ignore_ascii_case(&target.address) && port == target.port
        };
        user_fits
            && (is_target(&ssh.address, ssh.port)
                || host.vpn.iter().any(|vpn| is_target(&vpn.address, vpn.port)))
    };
    let named = |host: &&HostView| host.display_name.eq_ignore_ascii_case(&target.address);

    let mut matches = saved().filter(at_endpoint);
    match (matches.next(), matches.next()) {
        (Some(host), None) => Some(host),
        (Some(_), Some(_)) => None,
        (None, _) if target.username.is_none() && target.port == 22 => {
            let mut matches = saved().filter(named);
            matches.next().filter(|_| matches.next().is_none())
        }
        (None, _) => None,
    }
}

fn valid_username(text: &str) -> Option<&str> {
    let valid = !text.is_empty()
        && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        && !text.contains(['@', '/']);
    valid.then_some(text)
}

fn split_host_port(text: &str) -> Option<(&str, Option<u16>)> {
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('[') {
        let (addr, after) = rest.split_once(']')?;
        if addr.is_empty() {
            return None;
        }
        if after.is_empty() {
            return Some((addr, None));
        }
        let port_str = after.strip_prefix(':')?;
        let port = parse_port(port_str).ok()?;
        return Some((addr, Some(port)));
    }
    // Single colon means host:port; multiple colons without brackets is a bare IPv6 address.
    match text.split_once(':') {
        Some((addr, port_str)) if !port_str.contains(':') => {
            if addr.is_empty() {
                return None;
            }
            let port = parse_port(port_str).ok()?;
            Some((addr, Some(port)))
        }
        _ => Some((text, None)),
    }
}

impl App {
    /// Enter or Connect in the Hosts search field, which holds an SSH target: open a terminal
    /// on the saved host it means (after its key is verified, if it is not yet), or the host
    /// form filled in with the address, port and user for a host that is not saved.
    pub(super) fn inventory_quick_connect(self: &Rc<Self>) {
        let typed = self.inv.query.borrow().clone();
        let Some(target) = parse_ssh_target(&typed) else {
            return;
        };
        let saved = find_matching_host(&self.inv.hosts.borrow(), &target).cloned();
        self.inv.query.borrow_mut().clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Inventory>().set_query(SharedString::new());
        }
        self.rebuild();
        let Some(host) = saved else {
            if self.host_form_leave(Next::Quick(target.clone())) {
                self.host_form_open_quick(&target);
            }
            return;
        };
        if !self.host_form_leave(Next::Select("host".into(), host.id.clone())) {
            return;
        }
        self.select_item("host", &host.id);
        if host.trust_state == "trusted" {
            self.host_action(&host.id, "terminal");
        } else {
            self.hosts.set_connect_after_trust(&host.id);
            self.host_action(&host.id, "trust");
        }
    }
}

#[cfg(test)]
mod tests {
    use senvakara_app::{SshAuth, SshView};

    use super::*;

    fn target(user: Option<&str>, address: &str, port: u16) -> SshTarget {
        SshTarget {
            username: user.map(str::to_string),
            address: address.to_string(),
            port,
        }
    }

    #[test]
    fn parses_ssh_commands_and_shorthand_targets() {
        assert_eq!(
            parse_ssh_target("ssh root@192.168.1.10"),
            Some(target(Some("root"), "192.168.1.10", 22))
        );
        assert_eq!(
            parse_ssh_target("SSH -p 2222 Alice@example.com"),
            Some(target(Some("Alice"), "example.com", 2222))
        );
        assert_eq!(
            parse_ssh_target("ssh raflan@192.168.1.10 -p2200"),
            Some(target(Some("raflan"), "192.168.1.10", 2200))
        );
        assert_eq!(
            parse_ssh_target("ssh -l ops -p 2200 box.lan"),
            Some(target(Some("ops"), "box.lan", 2200))
        );
        assert_eq!(
            parse_ssh_target("ssh fedora"),
            Some(target(None, "fedora", 22))
        );
        assert_eq!(
            parse_ssh_target("raflan@192.168.1.10"),
            Some(target(Some("raflan"), "192.168.1.10", 22))
        );
        assert_eq!(
            parse_ssh_target("raflan@192.168.1.10:2222"),
            Some(target(Some("raflan"), "192.168.1.10", 2222))
        );
        assert_eq!(
            parse_ssh_target("ssh root@[2001:db8::1]:2222"),
            Some(target(Some("root"), "2001:db8::1", 2222))
        );
    }

    #[test]
    fn plain_searches_and_invalid_ssh_commands_are_ignored() {
        for text in [
            "",
            "   ",
            "fedora",
            "192.168.1.10",
            "ssh",
            "ssh ",
            "ssh -p 0 root@host",
            "ssh -p 99999 root@host",
            "ssh -p 2222 root@host:2200",
            "ssh -l a b@host",
            "ssh -X root@host",
            "@host",
            "user@",
            "user@host/path",
            "ssh root@host extra",
        ] {
            assert_eq!(parse_ssh_target(text), None, "{text:?}");
        }
    }

    #[test]
    fn finds_a_saved_ssh_host_unless_the_match_is_ambiguous() {
        let ssh_host = |id: &str, name: &str, addr: &str, port: u16, user: &str| HostView {
            id: id.into(),
            display_name: name.into(),
            os_family: "linux".into(),
            is_local: false,
            trust_state: "trusted".into(),
            last_seen_at: None,
            protocol: Protocol::Ssh,
            group_name: None,
            tags: Vec::new(),
            capabilities: Default::default(),
            connected: false,
            reconnecting: None,
            ssh: Some(SshView {
                address: addr.into(),
                port,
                username: user.into(),
                auth: SshAuth::Password,
                private_key_path: String::new(),
                fingerprint: None,
                has_passphrase: false,
                has_password: true,
                startup_snippet: None,
            }),
            rdp: None,
            vpn: Vec::new(),
            path: None,
        };
        let hosts = vec![
            ssh_host("h1", "Prod box", "192.168.1.10", 22, "raflan"),
            ssh_host("h2", "Prod root", "192.168.1.10", 22, "root"),
            ssh_host("h3", "Fedora", "10.0.0.5", 2222, "dev"),
        ];
        assert_eq!(
            find_matching_host(&hosts, &target(Some("raflan"), "192.168.1.10", 22))
                .map(|host| host.id.as_str()),
            Some("h1")
        );
        // Two saved accounts share 192.168.1.10: omitting the user must not guess.
        assert!(find_matching_host(&hosts, &target(None, "192.168.1.10", 22)).is_none());
        assert_eq!(
            find_matching_host(&hosts, &target(None, "10.0.0.5", 2222))
                .map(|host| host.id.as_str()),
            Some("h3")
        );
        assert_eq!(
            find_matching_host(&hosts, &target(None, "prod box", 22)).map(|host| host.id.as_str()),
            Some("h1")
        );
        assert!(find_matching_host(&hosts, &target(Some("other"), "10.0.0.5", 2222)).is_none());
    }
}
