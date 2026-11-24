//! How Home files its hosts: by parent group, and what the search field looks through.

use senvakara_app::{HostView, Protocol};

use super::quick_connect::parse_ssh_target;

/// The heading over the hosts that have no parent group. They come first: this PC is one.
pub const UNGROUPED: &str = "Hosts";

/// File items under their group. Items without a group come first under [`UNGROUPED`]; the
/// groups follow by name. Names differing only in case are one group (the first spelling
/// wins), and items keep their order within a group.
pub fn grouped<T>(items: Vec<(Option<String>, T)>) -> Vec<(String, Vec<T>)> {
    let mut loose = Vec::new();
    let mut groups: Vec<(String, Vec<T>)> = Vec::new();
    for (group, item) in items {
        let name = group.as_deref().map(str::trim).unwrap_or("");
        if name.is_empty() {
            loose.push(item);
            continue;
        }
        match groups
            .iter_mut()
            .find(|(have, _)| have.eq_ignore_ascii_case(name))
        {
            Some((_, members)) => members.push(item),
            None => groups.push((name.to_string(), vec![item])),
        }
    }
    groups.sort_by_key(|(name, _)| name.to_lowercase());
    if !loose.is_empty() {
        groups.insert(0, (UNGROUPED.to_string(), loose));
    }
    groups
}

/// Whether the search text finds this host: in its name, address, user, group or one of its
/// tags, or as an SSH quick-connect string (`ssh user@host`). `query` is already lower case; an
/// empty one finds everything.
pub fn host_matches(host: &HostView, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let (address, user) = match (&host.ssh, &host.rdp) {
        (Some(ssh), _) => (ssh.address.as_str(), ssh.username.as_str()),
        (None, Some(rdp)) => (rdp.address.as_str(), rdp.username.as_str()),
        (None, None) => ("", ""),
    };
    let direct = [
        host.display_name.as_str(),
        address,
        user,
        host.group_name.as_deref().unwrap_or(""),
    ]
    .into_iter()
    .chain(host.tags.iter().map(String::as_str))
    .any(|field| field.to_lowercase().contains(query));
    if direct {
        return true;
    }
    if host.protocol == Protocol::Ssh {
        if let (Some(ssh), Some(target)) = (&host.ssh, parse_ssh_target(query)) {
            let addr = target.address.to_lowercase();
            let host_ok = ssh.address.to_lowercase().contains(&addr)
                || host.display_name.to_lowercase().contains(&addr);
            let user_ok = target
                .username
                .as_deref()
                .is_none_or(|wanted| ssh.username.to_lowercase().contains(&wanted.to_lowercase()));
            return host_ok && user_ok;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use senvakara_app::{Protocol, RdpView, SshAuth, SshView};

    use super::*;

    fn filed(list: &[(&str, &str)]) -> Vec<(Option<String>, String)> {
        list.iter()
            .map(|(group, host)| {
                (
                    (!group.is_empty()).then(|| group.to_string()),
                    host.to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn hosts_without_a_group_come_first_and_groups_follow_by_name() {
        let groups = grouped(filed(&[
            ("", "This PC"),
            ("prod", "web-1"),
            ("Lab", "fedora"),
            ("", "laptop"),
            ("Prod", "web-2"),
            ("  ", "spare"),
        ]));
        assert_eq!(
            groups,
            [
                (
                    "Hosts".to_string(),
                    vec![
                        "This PC".to_string(),
                        "laptop".to_string(),
                        "spare".to_string()
                    ]
                ),
                ("Lab".to_string(), vec!["fedora".to_string()]),
                (
                    "prod".to_string(),
                    vec!["web-1".to_string(), "web-2".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn a_list_with_no_ungrouped_host_has_no_empty_first_group() {
        let groups = grouped(filed(&[("b", "two"), ("a", "one")]));
        let names: Vec<&str> = groups.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(grouped(Vec::<(Option<String>, u8)>::new()).is_empty());
    }

    fn host(group: Option<&str>, tags: &[&str]) -> HostView {
        HostView {
            id: "h1".into(),
            display_name: "Build box".into(),
            os_family: "linux".into(),
            is_local: false,
            trust_state: "trusted".into(),
            last_seen_at: None,
            protocol: Protocol::Ssh,
            group_name: group.map(str::to_string),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            capabilities: Default::default(),
            connected: false,
            reconnecting: None,
            ssh: None,
            rdp: None,
            vpn: Vec::new(),
            path: None,
        }
    }

    #[test]
    fn search_looks_through_name_address_user_group_and_tags() {
        let ssh = HostView {
            ssh: Some(SshView {
                address: "192.0.2.10".into(),
                port: 22,
                username: "bob".into(),
                auth: SshAuth::Agent,
                private_key_path: String::new(),
                fingerprint: None,
                has_passphrase: false,
                has_password: false,
                startup_snippet: None,
            }),
            ..host(Some("Production"), &["web", "EU-west"])
        };
        for query in ["", "build", "192.0.2", "bob", "produ", "eu-west", "web"] {
            assert!(host_matches(&ssh, query), "{query}");
        }
        assert!(!host_matches(&ssh, "staging"));

        let rdp = HostView {
            protocol: Protocol::Rdp,
            rdp: Some(RdpView {
                address: "office.example".into(),
                port: 3389,
                username: "alice".into(),
                has_credential: false,
            }),
            ..host(None, &[])
        };
        assert!(host_matches(&rdp, "office"));
        assert!(host_matches(&rdp, "alice"));
        assert!(!host_matches(&rdp, "bob"));
    }

    #[test]
    fn an_ssh_quick_connect_query_still_finds_the_matching_host() {
        let ssh = HostView {
            ssh: Some(SshView {
                address: "192.168.1.10".into(),
                port: 22,
                username: "raflan".into(),
                auth: SshAuth::Password,
                private_key_path: String::new(),
                fingerprint: None,
                has_passphrase: false,
                has_password: true,
                startup_snippet: None,
            }),
            ..host(None, &[])
        };
        assert!(host_matches(&ssh, "ssh raflan@192.168.1.10"));
        assert!(host_matches(&ssh, "raflan@192.168.1.10"));
        assert!(host_matches(&ssh, "ssh 192.168.1.10"));
        assert!(!host_matches(&ssh, "ssh root@192.168.1.10"));
    }
}
