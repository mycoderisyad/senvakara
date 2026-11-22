use super::*;

fn input(addr: &str) -> HostUpsert {
    HostUpsert {
        display_name: " Fedora laptop ".into(),
        os_family: "linux".into(),
        protocol: Protocol::Ssh,
        address: addr.into(),
        port: 22,
        username: "dev".into(),
        auth: SshAuth::KeyFile {
            path: r"C:\keys\id".into(),
        },
        group_name: None,
        tags: Vec::new(),
        startup_snippet: None,
    }
}

fn rdp_input(addr: &str) -> HostUpsert {
    HostUpsert {
        display_name: "Office PC".into(),
        os_family: "windows".into(),
        protocol: Protocol::Rdp,
        address: addr.into(),
        port: 3389,
        username: "alice".into(),
        auth: SshAuth::Password,
        group_name: Some("Office".into()),
        tags: Vec::new(),
        startup_snippet: None,
    }
}

#[test]
fn create_list_and_trust() {
    let mut db = Db::open_in_memory().unwrap();
    let local = db.ensure_local_host("This PC").unwrap();
    assert_eq!(
        db.ensure_local_host("This PC").unwrap(),
        local,
        "local host is a singleton"
    );

    let h = db.create_host(&input("192.0.2.10")).unwrap();
    assert_eq!(h.display_name, "Fedora laptop");
    assert_eq!(h.trust_state, "untrusted");
    assert_eq!(h.protocol, Protocol::Ssh);
    assert_eq!(h.ssh.as_ref().unwrap().fingerprint, None);
    assert_eq!(
        h.ssh.as_ref().unwrap().auth,
        SshAuth::KeyFile {
            path: r"C:\keys\id".into()
        }
    );

    let all = db.list_hosts().unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].is_local);

    let t = db.trust_host_key(&h.id, "SHA256:abc").unwrap();
    assert_eq!(t.trust_state, "trusted");
    assert_eq!(t.ssh.unwrap().fingerprint.as_deref(), Some("SHA256:abc"));
}

#[test]
fn changing_address_keeps_id_but_clears_pinned_key() {
    let mut db = Db::open_in_memory().unwrap();
    let h = db.create_host(&input("192.0.2.10")).unwrap();
    db.trust_host_key(&h.id, "SHA256:abc").unwrap();

    let renamed = db
        .update_host(
            &h.id,
            &HostUpsert {
                display_name: "Box".into(),
                ..input("192.0.2.10")
            },
        )
        .unwrap();
    assert_eq!(
        renamed.ssh.as_ref().unwrap().fingerprint.as_deref(),
        Some("SHA256:abc"),
        "rename keeps trust"
    );

    let moved = db.update_host(&h.id, &input("192.0.2.99")).unwrap();
    assert_eq!(moved.id, h.id);
    assert_eq!(moved.ssh.unwrap().fingerprint, None);
    assert_eq!(moved.trust_state, "untrusted");
}

#[test]
fn rejects_invalid_input_and_local_delete() {
    let mut db = Db::open_in_memory().unwrap();
    assert!(matches!(
        db.create_host(&input("bad host")),
        Err(StorageError::Invalid(_))
    ));
    assert!(matches!(
        db.create_host(&HostUpsert {
            port: 0,
            ..input("192.0.2.1")
        }),
        Err(StorageError::Invalid(_))
    ));
    let local = db.ensure_local_host("This PC").unwrap();
    assert!(db.delete_host(&local).is_err());
    assert!(db.trust_host_key(&local, "MD5:xx").is_err());
}

#[test]
fn each_sign_in_method_needs_its_own_detail() {
    let mut db = Db::open_in_memory().unwrap();
    let no_path = HostUpsert {
        auth: SshAuth::KeyFile { path: "  ".into() },
        ..input("192.0.2.1")
    };
    assert!(db.create_host(&no_path).is_err());
    let no_key = HostUpsert {
        auth: SshAuth::Keychain { key_id: "".into() },
        ..input("192.0.2.1")
    };
    assert!(db.create_host(&no_key).is_err());
    let no_user = HostUpsert {
        username: " ".into(),
        auth: SshAuth::Password,
        ..input("192.0.2.1")
    };
    assert!(db.create_host(&no_user).is_err());

    let password = db
        .create_host(&HostUpsert {
            auth: SshAuth::Password,
            ..input("192.0.2.1")
        })
        .unwrap();
    assert_eq!(password.ssh.as_ref().unwrap().auth, SshAuth::Password);
    db.set_password_ref(&password.id, Some("secret-pw"))
        .unwrap();
    let ssh = db.get_host(&password.id).unwrap().ssh.unwrap();
    assert_eq!(ssh.password_ref.as_deref(), Some("secret-pw"));

    let agent = db
        .create_host(&HostUpsert {
            auth: SshAuth::Agent,
            ..input("192.0.2.2")
        })
        .unwrap();
    assert_eq!(agent.ssh.unwrap().auth, SshAuth::Agent);
}

#[test]
fn group_tags_and_snippet_are_cleaned_and_kept() {
    let mut db = Db::open_in_memory().unwrap();
    let h = db
        .create_host(&HostUpsert {
            group_name: Some("  VPS ".into()),
            tags: vec![" prod ".into(), "PROD".into(), "".into(), "web".into()],
            startup_snippet: Some("cd ~/app && ls".into()),
            ..input("192.0.2.10")
        })
        .unwrap();
    assert_eq!(h.group_name.as_deref(), Some("VPS"));
    assert_eq!(h.tags, ["prod", "web"]);
    assert_eq!(
        h.ssh.as_ref().unwrap().startup_snippet.as_deref(),
        Some("cd ~/app && ls")
    );
    assert_eq!(db.host_groups().unwrap(), ["VPS"]);

    let cleared = db
        .update_host(
            &h.id,
            &HostUpsert {
                group_name: Some(" ".into()),
                tags: vec!["db".into()],
                startup_snippet: Some("   ".into()),
                ..input("192.0.2.10")
            },
        )
        .unwrap();
    assert_eq!(cleared.group_name, None);
    assert_eq!(cleared.tags, ["db"]);
    assert_eq!(cleared.ssh.unwrap().startup_snippet, None);
    assert!(db.host_groups().unwrap().is_empty());

    let comma = HostUpsert {
        tags: vec!["a,b".into()],
        ..input("192.0.2.11")
    };
    assert!(db.create_host(&comma).is_err());
    let too_many = HostUpsert {
        tags: (0..17).map(|i| format!("t{i}")).collect(),
        ..input("192.0.2.11")
    };
    assert!(db.create_host(&too_many).is_err());
}

#[test]
fn a_remote_desktop_host_has_no_ssh_endpoint_and_keeps_its_protocol() {
    let mut db = Db::open_in_memory().unwrap();
    let h = db.create_host(&rdp_input("192.0.2.30")).unwrap();
    assert_eq!(h.protocol, Protocol::Rdp);
    assert_eq!(h.trust_state, "external");
    assert!(h.ssh.is_none());
    let rdp = h.rdp.as_ref().unwrap();
    assert_eq!((rdp.address.as_str(), rdp.port), ("192.0.2.30", 3389));
    assert!(db.add_vpn_endpoint(&h.id, "10.8.0.3", 22).is_err());

    db.set_rdp_credential_target(&h.id, Some("TERMSRV/192.0.2.30"))
        .unwrap();
    let moved = db.update_host(&h.id, &rdp_input("192.0.2.31")).unwrap();
    let rdp = moved.rdp.unwrap();
    assert_eq!(rdp.address, "192.0.2.31");
    assert_eq!(rdp.credential_target.as_deref(), Some("TERMSRV/192.0.2.30"));

    assert!(
        db.update_host(&h.id, &input("192.0.2.31")).is_err(),
        "the protocol cannot change"
    );
    let ssh = db.create_host(&input("192.0.2.40")).unwrap();
    assert!(db.set_rdp_credential_target(&ssh.id, Some("x")).is_err());
}

#[test]
fn a_remote_desktop_host_signs_in_with_a_password_only() {
    let mut db = Db::open_in_memory().unwrap();
    let with_key = HostUpsert {
        auth: SshAuth::KeyFile { path: "k".into() },
        ..rdp_input("192.0.2.30")
    };
    assert!(db.create_host(&with_key).is_err());
    let with_snippet = HostUpsert {
        startup_snippet: Some("ls".into()),
        ..rdp_input("192.0.2.30")
    };
    assert!(db.create_host(&with_snippet).is_err());
    let no_user = HostUpsert {
        username: "".into(),
        ..rdp_input("192.0.2.30")
    };
    assert!(
        db.create_host(&no_user).is_ok(),
        "mstsc asks for the user when none is saved"
    );
}

#[test]
fn vpn_endpoints_are_extra_addresses_of_the_same_host() {
    let mut db = Db::open_in_memory().unwrap();
    let h = db.create_host(&input("192.0.2.10")).unwrap();
    db.trust_host_key(&h.id, "SHA256:abc").unwrap();
    let h2 = db.add_vpn_endpoint(&h.id, " 10.8.0.3 ", 22).unwrap();
    assert_eq!(h2.id, h.id);
    assert_eq!(h2.vpn.len(), 1);
    assert_eq!(h2.vpn[0].address, "10.8.0.3");
    // The pinned key and the main endpoint are untouched.
    assert_eq!(
        h2.ssh.as_ref().unwrap().fingerprint.as_deref(),
        Some("SHA256:abc")
    );
    assert_eq!(h2.trust_state, "trusted");
    let listed = db.list_hosts().unwrap();
    assert_eq!(listed.iter().find(|x| x.id == h.id).unwrap().vpn.len(), 1);
    // Duplicates, the main address, bad input and the local host are refused.
    assert!(db.add_vpn_endpoint(&h.id, "10.8.0.3", 22).is_err());
    assert!(db.add_vpn_endpoint(&h.id, "192.0.2.10", 22).is_err());
    assert!(db.add_vpn_endpoint(&h.id, "a b", 22).is_err());
    assert!(db.add_vpn_endpoint(&h.id, "u@10.8.0.4", 22).is_err());
    assert!(db.add_vpn_endpoint(&h.id, "10.8.0.9", 0).is_err());
    let local = db.ensure_local_host("PC").unwrap();
    assert!(db.add_vpn_endpoint(&local, "10.8.0.9", 22).is_err());
    for i in 0..3 {
        db.add_vpn_endpoint(&h.id, &format!("100.64.0.{i}"), 22)
            .unwrap();
    }
    assert!(
        db.add_vpn_endpoint(&h.id, "100.64.0.9", 22).is_err(),
        "limit"
    );
    let ep = h2.vpn[0].endpoint_id.clone();
    assert_eq!(db.remove_vpn_endpoint(&h.id, &ep).unwrap().vpn.len(), 3);
    assert!(db.remove_vpn_endpoint(&h.id, &ep).is_err());
    // The SSH endpoint can never be removed through this call.
    let ssh_ep = h2.ssh.unwrap().endpoint_id;
    assert!(db.remove_vpn_endpoint(&h.id, &ssh_ep).is_err());
    db.delete_host(&h.id).unwrap();
    let left: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM host_endpoints", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn delete_cascades_endpoint_settings_tags_and_rdp() {
    let mut db = Db::open_in_memory().unwrap();
    let h = db
        .create_host(&HostUpsert {
            tags: vec!["x".into()],
            ..input("192.0.2.10")
        })
        .unwrap();
    let r = db.create_host(&rdp_input("192.0.2.30")).unwrap();
    db.delete_host(&h.id).unwrap();
    db.delete_host(&r.id).unwrap();
    for table in ["ssh_settings", "host_tags", "rdp_settings"] {
        let n: i64 = db
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}
