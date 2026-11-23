use super::*;

fn filled() -> Fields {
    Fields {
        protocol: Protocol::Ssh,
        name: " Fedora laptop ".into(),
        address: "192.0.2.10".into(),
        port: "22".into(),
        username: "alice".into(),
        os: "linux".into(),
        group: String::new(),
        tags: Vec::new(),
        method: Method::KeyFile,
        key_path: "C:\\Users\\alice\\.ssh\\id_ed25519".into(),
        key_id: String::new(),
        snippet: String::new(),
    }
}

fn rdp() -> Fields {
    Fields {
        protocol: Protocol::Rdp,
        name: "Office PC".into(),
        address: "192.0.2.20".into(),
        port: "3389".into(),
        username: "OFFICE\\alice".into(),
        method: Method::Password,
        key_path: String::new(),
        ..filled()
    }
}

fn only(fields: Fields, password: Password) -> Problems {
    check(&fields, password).expect_err("invalid")
}

#[test]
fn a_complete_form_is_accepted_and_trimmed() {
    let input = check(&filled(), Password::Missing).expect("valid");
    assert_eq!(input.display_name, "Fedora laptop");
    assert_eq!((input.address.as_str(), input.port), ("192.0.2.10", 22));
    assert_eq!(input.os_family, "linux");
    assert_eq!(input.protocol, Protocol::Ssh);
    let other = check(
        &Fields {
            os: "unknown".into(),
            port: " 2222 ".into(),
            ..filled()
        },
        Password::Missing,
    )
    .expect("valid");
    assert_eq!((other.os_family.as_str(), other.port), ("unknown", 2222));
}

#[test]
fn an_empty_form_names_every_field_it_needs() {
    let problems = only(Fields::default(), Password::Missing);
    assert!(problems.address.starts_with("Enter the host name"));
    assert!(problems.port.starts_with("Enter the SSH port"));
    assert!(problems.username.starts_with("Enter the user name"));
    assert!(problems.password.starts_with("Enter the password"));
    assert_eq!(problems.key, "");
}

#[test]
fn a_host_without_a_label_is_named_after_its_address() {
    let input = check(
        &Fields {
            name: "  ".into(),
            ..filled()
        },
        Password::Missing,
    )
    .expect("valid");
    assert_eq!(input.display_name, "192.0.2.10");
}

#[test]
fn each_field_is_checked_on_its_own() {
    let port = only(
        Fields {
            port: "0".into(),
            ..filled()
        },
        Password::Missing,
    );
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

    let address = |address: &str| {
        only(
            Fields {
                address: address.into(),
                ..filled()
            },
            Password::Missing,
        )
        .address
    };
    assert_eq!(address("my host"), "An address cannot contain spaces.");
    assert!(address("alice@host").starts_with("Enter only the address"));
    assert!(address("ssh://host").contains("not a link"));
    assert_eq!(
        only(
            Fields {
                username: "al ice".into(),
                ..filled()
            },
            Password::Missing
        )
        .username,
        "A user name cannot contain spaces."
    );
    assert!(only(
        Fields {
            group: "g".repeat(65),
            ..filled()
        },
        Password::Missing
    )
    .group
    .starts_with("A group name can be at most 64"));
}

#[test]
fn a_password_host_needs_a_password_typed_or_saved() {
    let fields = Fields {
        method: Method::Password,
        key_path: String::new(),
        ..filled()
    };
    let problems = only(fields.clone(), Password::Missing);
    assert_eq!(
        problems,
        Problems {
            password: "Enter the password, or choose a key to sign in with.",
            ..Problems::default()
        }
    );
    for password in [Password::Typed, Password::Saved] {
        let input = check(&fields, password).expect("valid");
        assert_eq!(input.auth, SshAuth::Password);
    }
}

#[test]
fn a_key_file_host_needs_the_path_and_no_password() {
    let problems = only(
        Fields {
            key_path: "  ".into(),
            ..filled()
        },
        Password::Typed,
    );
    assert!(problems.key.starts_with("Choose the private key file"));
    assert_eq!(problems.password, "");
    let input = check(&filled(), Password::Missing).expect("valid");
    assert_eq!(
        input.auth,
        SshAuth::KeyFile {
            path: "C:\\Users\\alice\\.ssh\\id_ed25519".into()
        }
    );
}

#[test]
fn a_keychain_host_needs_a_chosen_key() {
    let fields = Fields {
        method: Method::Keychain,
        key_path: String::new(),
        ..filled()
    };
    assert_eq!(
        only(fields.clone(), Password::Missing).key,
        "Choose a key from the Keychain."
    );
    let input = check(
        &Fields {
            key_id: "key-1".into(),
            ..fields
        },
        Password::Missing,
    )
    .expect("valid");
    assert_eq!(
        input.auth,
        SshAuth::Keychain {
            key_id: "key-1".into()
        }
    );
}

#[test]
fn an_agent_host_needs_neither_key_nor_password() {
    let input = check(
        &Fields {
            method: Method::Agent,
            key_path: String::new(),
            ..filled()
        },
        Password::Missing,
    )
    .expect("valid");
    assert_eq!(input.auth, SshAuth::Agent);
}

#[test]
fn group_tags_and_snippet_reach_the_backend_trimmed() {
    let input = check(
        &Fields {
            group: "  Production ".into(),
            tags: vec!["web".into(), "eu".into()],
            snippet: "cd /srv/app\n\n".into(),
            ..filled()
        },
        Password::Missing,
    )
    .expect("valid");
    assert_eq!(input.group_name.as_deref(), Some("Production"));
    assert_eq!(input.tags, ["web", "eu"]);
    assert_eq!(input.startup_snippet.as_deref(), Some("cd /srv/app"));

    let plain = check(&filled(), Password::Missing).expect("valid");
    assert_eq!(plain.group_name, None);
    assert_eq!(plain.startup_snippet, None);

    let long = only(
        Fields {
            snippet: "x".repeat(4097),
            ..filled()
        },
        Password::Missing,
    );
    assert!(long.snippet.starts_with("The startup snippet is too long"));
}

#[test]
fn a_remote_desktop_host_signs_in_with_a_password_and_has_no_snippet() {
    let input = check(
        &Fields {
            // Left over from the SSH side of the form: neither applies to Remote Desktop.
            method: Method::Keychain,
            snippet: "echo hi".into(),
            os: "linux".into(),
            ..rdp()
        },
        Password::Missing,
    )
    .expect("valid: Remote Desktop asks for the password itself when none is saved");
    assert_eq!(input.protocol, Protocol::Rdp);
    assert_eq!(input.auth, SshAuth::Password);
    assert_eq!(input.startup_snippet, None);
    assert_eq!(input.os_family, "windows");
    assert_eq!(
        (input.username.as_str(), input.port),
        ("OFFICE\\alice", 3389)
    );
}

#[test]
fn a_remote_desktop_password_needs_the_user_it_belongs_to() {
    let nameless = Fields {
        username: String::new(),
        ..rdp()
    };
    assert!(check(&nameless, Password::Missing).is_ok());
    assert_eq!(
        only(nameless.clone(), Password::Typed).username,
        "Enter the user name the password belongs to."
    );
    assert_eq!(
        only(
            Fields {
                port: " ".into(),
                ..nameless
            },
            Password::Missing
        )
        .port,
        "Enter the Remote Desktop port. The usual one is 3389."
    );
}

#[test]
fn a_remote_desktop_host_loses_its_password_when_address_or_user_changes() {
    let before = rdp();
    assert!(!rdp_password_lost(&before, &before));
    assert!(!rdp_password_lost(
        &before,
        &Fields {
            name: "Renamed".into(),
            port: "3390".into(),
            address: " 192.0.2.20 ".into(),
            ..rdp()
        }
    ));
    assert!(rdp_password_lost(
        &before,
        &Fields {
            address: "192.0.2.21".into(),
            ..rdp()
        }
    ));
    assert!(rdp_password_lost(
        &before,
        &Fields {
            username: "bob".into(),
            ..rdp()
        }
    ));
    // The backend compares the address exactly, so a new case drops the saved password.
    let named = Fields {
        address: "server.example".into(),
        ..rdp()
    };
    assert!(rdp_password_lost(
        &named,
        &Fields {
            address: "SERVER.example".into(),
            ..rdp()
        }
    ));
    // An SSH password is filed under the host, not its address.
    assert!(!rdp_password_lost(
        &filled(),
        &Fields {
            address: "192.0.2.99".into(),
            ..filled()
        }
    ));
}

#[test]
fn a_tag_is_refused_when_empty_repeated_too_long_or_one_too_many() {
    let tags = vec!["web".to_string(), "EU".to_string()];
    assert_eq!(tag_problem(" db ", &tags), "");
    assert_eq!(tag_problem("  ", &tags), "Type the tag first.");
    assert_eq!(tag_problem("eu", &tags), "This host already has that tag.");
    assert!(tag_problem("a,b", &tags).starts_with("A tag cannot contain a comma"));
    assert_eq!(
        tag_problem(&"t".repeat(33), &tags),
        "A tag can be at most 32 characters."
    );
    assert_eq!(tag_problem(&"t".repeat(32), &tags), "");
    let full: Vec<String> = (0..16).map(|n| format!("tag{n}")).collect();
    assert!(tag_problem("one-more", &full).starts_with("A host can have at most 16 tags"));
}

#[test]
fn vpn_addresses() {
    let known = vec![("192.0.2.10".to_string(), 22), ("10.8.0.3".to_string(), 22)];
    assert_eq!(vpn_problem("100.64.0.7", "22", &known, 1), "");
    assert_eq!(vpn_problem("my-host.tailnet", " 2222 ", &known, 1), "");
    assert!(vpn_problem("", "22", &known, 1).starts_with("Type the VPN address"));
    assert_eq!(
        vpn_problem("10.8.0.3", "22", &known, 1),
        "This host already has that address and port."
    );
    // The same address on another port is another way in, as storage allows.
    assert_eq!(vpn_problem("10.8.0.3", "2222", &known, 1), "");
    assert_eq!(vpn_problem("192.0.2.10", "2222", &known, 1), "");
    assert_eq!(
        vpn_problem("192.0.2.10", " 22 ", &known, 1),
        "This host already has that address and port."
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

#[test]
fn an_encrypted_key_file_needs_a_typed_or_kept_passphrase() {
    let said = "This key is encrypted: enter its passphrase.";
    // Typed, saved, or removed (with nothing typed instead).
    assert_eq!(
        key_passphrase_problem(Some(true), false, false, false),
        said
    );
    assert_eq!(key_passphrase_problem(Some(true), true, false, false), "");
    assert_eq!(key_passphrase_problem(Some(true), false, true, false), "");
    assert_eq!(key_passphrase_problem(Some(true), false, true, true), said);
    assert_eq!(key_passphrase_problem(Some(true), true, true, true), "");
    // A plain key, or one that has not been read, claims nothing.
    assert_eq!(key_passphrase_problem(Some(false), false, false, false), "");
    assert_eq!(key_passphrase_problem(None, false, false, false), "");
}
