//! Checking what the host form and the VPN address fields hold, without touching the UI.

use senvakara_app::HostUpsert;

/// A host may have this many VPN addresses (the storage layer's limit).
const MAX_VPN: usize = 4;

/// What the form holds, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub name: String,
    pub address: String,
    pub port: String,
    pub username: String,
    /// "linux" or "unknown".
    pub os: String,
    pub key_path: String,
}

/// What is wrong with each field, in words. An empty string means the field is fine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub name: &'static str,
    pub address: &'static str,
    pub port: &'static str,
    pub username: &'static str,
    pub key: &'static str,
}

impl Problems {
    pub fn any(&self) -> bool {
        [self.name, self.address, self.port, self.username, self.key]
            .iter()
            .any(|problem| !problem.is_empty())
    }
}

/// Why `text` is not a host name or IP address, or "" when it is.
pub fn address_problem(text: &str) -> &'static str {
    let text = text.trim();
    if text.is_empty() {
        "Enter the host name or IP address."
    } else if text.chars().any(char::is_whitespace) {
        "An address cannot contain spaces."
    } else if text.contains('@') {
        "Enter only the address here. The user name has its own field."
    } else if text.contains('/') {
        "Enter a host name or IP address, not a link."
    } else if text.len() > 253 {
        "This address is too long."
    } else {
        ""
    }
}

/// The port as a number, or why it is not one.
pub fn parse_port(text: &str) -> Result<u16, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter the SSH port. The usual one is 22.");
    }
    match text.parse::<u32>() {
        Ok(port @ 1..=65535) => Ok(port as u16),
        _ => Err("The port must be a number between 1 and 65535."),
    }
}

/// Check the form. Ok carries what the backend stores.
pub fn check(fields: &Fields) -> Result<HostUpsert, Problems> {
    let mut problems = Problems::default();
    let name = fields.name.trim();
    if name.is_empty() {
        problems.name = "Give the host a name.";
    }
    problems.address = address_problem(&fields.address);
    let port = parse_port(&fields.port);
    if let Err(problem) = port {
        problems.port = problem;
    }
    let username = fields.username.trim();
    if username.is_empty() {
        problems.username = "Enter the user name to sign in with.";
    } else if username.chars().any(char::is_whitespace) {
        problems.username = "A user name cannot contain spaces.";
    }
    let key = fields.key_path.trim();
    if key.is_empty() {
        problems.key = "Choose the private key file to sign in with.";
    }
    match port {
        Ok(port) if !problems.any() => Ok(HostUpsert {
            display_name: name.to_string(),
            os_family: if fields.os == "unknown" {
                "unknown"
            } else {
                "linux"
            }
            .to_string(),
            address: fields.address.trim().to_string(),
            port,
            username: username.to_string(),
            private_key_path: key.to_string(),
        }),
        _ => Err(problems),
    }
}

/// Why a VPN address cannot be added, or "" when it can. `known` are the addresses the host
/// already has (its main one included); `vpn_count` how many of them are VPN addresses.
pub fn vpn_problem(address: &str, port: &str, known: &[String], vpn_count: usize) -> &'static str {
    let address = address.trim();
    if vpn_count >= MAX_VPN {
        return "A host can have at most 4 VPN addresses. Remove one first.";
    }
    if address.is_empty() {
        return "Type the VPN address of this host first.";
    }
    let problem = address_problem(address);
    if !problem.is_empty() {
        return problem;
    }
    if known.iter().any(|have| have.eq_ignore_ascii_case(address)) {
        return "This host already has that address.";
    }
    match parse_port(port) {
        Ok(_) => "",
        Err(problem) => problem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Fields {
        Fields {
            name: " Fedora laptop ".into(),
            address: "192.0.2.10".into(),
            port: "22".into(),
            username: "alice".into(),
            os: "linux".into(),
            key_path: "C:\\Users\\alice\\.ssh\\id_ed25519".into(),
        }
    }

    #[test]
    fn a_complete_form_is_accepted_and_trimmed() {
        let input = check(&filled()).expect("valid");
        assert_eq!(input.display_name, "Fedora laptop");
        assert_eq!((input.address.as_str(), input.port), ("192.0.2.10", 22));
        assert_eq!(input.os_family, "linux");
        let other = check(&Fields {
            os: "unknown".into(),
            port: " 2222 ".into(),
            ..filled()
        })
        .expect("valid");
        assert_eq!((other.os_family.as_str(), other.port), ("unknown", 2222));
    }

    #[test]
    fn an_empty_form_names_every_field() {
        let problems = check(&Fields::default()).expect_err("invalid");
        assert!(problems.name.starts_with("Give the host"));
        assert!(problems.address.starts_with("Enter the host name"));
        assert!(problems.port.starts_with("Enter the SSH port"));
        assert!(problems.username.starts_with("Enter the user name"));
        assert!(problems.key.starts_with("Choose the private key"));
    }

    #[test]
    fn each_field_is_checked_on_its_own() {
        let only = |fields: Fields| check(&fields).expect_err("invalid");
        let port = only(Fields {
            port: "0".into(),
            ..filled()
        });
        assert_eq!(port.port, "The port must be a number between 1 and 65535.");
        assert_eq!(
            port,
            Problems {
                port: port.port,
                ..Problems::default()
            }
        );
        for bad in ["65536", "ssh", "-1", "22.5"] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_port("65535"), Ok(65535));
        assert_eq!(parse_port("1"), Ok(1));

        assert_eq!(
            only(Fields {
                address: "my host".into(),
                ..filled()
            })
            .address,
            "An address cannot contain spaces."
        );
        assert!(only(Fields {
            address: "alice@host".into(),
            ..filled()
        })
        .address
        .starts_with("Enter only the address"));
        assert!(only(Fields {
            address: "ssh://host".into(),
            ..filled()
        })
        .address
        .contains("not a link"));
        assert_eq!(
            only(Fields {
                username: "al ice".into(),
                ..filled()
            })
            .username,
            "A user name cannot contain spaces."
        );
    }

    #[test]
    fn vpn_addresses() {
        let known = vec!["192.0.2.10".to_string(), "10.8.0.3".to_string()];
        assert_eq!(vpn_problem("100.64.0.7", "22", &known, 1), "");
        assert_eq!(vpn_problem("my-host.tailnet", " 2222 ", &known, 1), "");
        assert!(vpn_problem("", "22", &known, 1).starts_with("Type the VPN address"));
        assert_eq!(
            vpn_problem("10.8.0.3", "22", &known, 1),
            "This host already has that address."
        );
        assert_eq!(
            vpn_problem("10.8.0.9", "70000", &known, 1),
            "The port must be a number between 1 and 65535."
        );
        assert!(vpn_problem("10.8.0.9", "22", &known, 4).starts_with("A host can have at most"));
        assert_eq!(
            vpn_problem("10.8 .0.9", "22", &known, 1),
            "An address cannot contain spaces."
        );
    }
}
