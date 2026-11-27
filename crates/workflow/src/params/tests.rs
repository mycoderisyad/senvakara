use super::*;
use serde_json::json;

fn spec(name: &str, kind: ParamKind) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        label: name.into(),
        kind,
        required: false,
        default: None,
        min: None,
        max: None,
        options: Vec::new(),
        max_len: None,
        suffix: None,
        help: None,
    }
}

fn given(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn unknown_settings_are_refused() {
    let err = check_params(
        &[spec("port", ParamKind::Port)],
        &given(&[("pirt", json!(22))]),
    );
    assert_eq!(err.unwrap_err(), "unknown setting `pirt`");
}

#[test]
fn defaults_fill_in_and_required_values_must_be_there() {
    let mut port = spec("port", ParamKind::Port);
    port.default = Some(json!(22));
    let mut user = spec("user", ParamKind::UnixUser);
    user.required = true;
    assert_eq!(
        check_params(&[port.clone(), user.clone()], &given(&[])).unwrap_err(),
        "user is required"
    );
    let v = check_params(&[port, user], &given(&[("user", json!("deploy"))])).unwrap();
    assert_eq!(v["port"], ParamValue::Int(22));
    assert_eq!(v["user"], ParamValue::Text("deploy".into()));
}

#[test]
fn a_blank_value_counts_as_missing() {
    let mut user = spec("user", ParamKind::UnixUser);
    user.required = true;
    assert!(check_params(&[user], &given(&[("user", json!("  "))])).is_err());
}

#[test]
fn hostile_values_are_refused_by_their_kind() {
    let cases = [
        (ParamKind::Text, json!("a\nrm -rf /")),
        (ParamKind::Port, json!(0)),
        (ParamKind::Port, json!("22; reboot")),
        (ParamKind::Package, json!("x; reboot")),
        (ParamKind::UnixUser, json!("root;id")),
        (ParamKind::UnixUser, json!("Admin")),
        (ParamKind::Hostname, json!("a b.example.com")),
        (ParamKind::Hostname, json!("-bad.example.com")),
        (ParamKind::Secret, json!("hunter2")),
        (ParamKind::Bool, json!("yes")),
        (ParamKind::Multiline, json!("ok\0no")),
    ];
    for (kind, value) in cases {
        let s = spec("v", kind);
        assert!(
            check_params(&[s], &given(&[("v", value.clone())])).is_err(),
            "{kind:?} accepted {value}"
        );
    }
}

#[test]
fn ranges_choices_and_suffixes_are_enforced() {
    let mut wait = spec("seconds", ParamKind::Int);
    wait.min = Some(1);
    wait.max = Some(300);
    assert!(check_params(&[wait.clone()], &given(&[("seconds", json!(0))])).is_err());
    assert!(check_params(&[wait.clone()], &given(&[("seconds", json!(999))])).is_err());
    assert!(check_params(&[wait], &given(&[("seconds", json!("30"))])).is_ok());

    let mut mode = spec("mode", ParamKind::Choice);
    mode.options = vec!["debug".into(), "release".into()];
    assert!(check_params(&[mode.clone()], &given(&[("mode", json!("profile"))])).is_err());
    assert!(check_params(&[mode], &given(&[("mode", json!("release"))])).is_ok());

    let mut apk = spec("path", ParamKind::Path);
    apk.suffix = Some(".apk".into());
    assert!(check_params(&[apk.clone()], &given(&[("path", json!(r"C:\a.exe"))])).is_err());
    assert!(check_params(&[apk], &given(&[("path", json!(r"C:\build\App.APK"))])).is_ok());
}

#[test]
fn quoting_keeps_hostile_text_one_shell_word() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
    assert_eq!(
        shell_quote("$(reboot) `id` ; rm -rf /"),
        "'$(reboot) `id` ; rm -rf /'"
    );
}

#[test]
fn the_preamble_exports_every_value_and_resolves_secrets() {
    let mut values = BTreeMap::new();
    values.insert("port".to_string(), ParamValue::Int(2222));
    values.insert("enabled".to_string(), ParamValue::Bool(true));
    values.insert("note".to_string(), ParamValue::Text("x'; reboot; '".into()));
    values.insert(
        "password".to_string(),
        ParamValue::SecretRef("secret-1".into()),
    );
    let mut secrets = BTreeMap::new();
    assert!(preamble(&values, &secrets).is_err());
    secrets.insert("secret-1".to_string(), "p'w".to_string());
    let text = preamble(&values, &secrets).unwrap();
    assert_eq!(
        text,
        "set -euo pipefail\n\
         export SV_ENABLED=1\n\
         export SV_NOTE='x'\\''; reboot; '\\'''\n\
         export SV_PASSWORD='p'\\''w'\n\
         export SV_PORT=2222\n"
    );
}

#[test]
fn public_keys_are_one_plain_key_line() {
    let key = spec("key", ParamKind::SshPublicKey);
    let ok = |v: &str| check_one(&key, &json!(v));
    let body = "AAAAC3NzaC1lZDI1NTE5AAAAIGq0dZ1u0Tqk3gJvGQ1m0nI2x8C7w3m1d2Rk0b4y0aBc";
    assert_eq!(
        ok(&format!("  ssh-ed25519   {body}   me@pc  laptop ")).unwrap(),
        ParamValue::Text(format!("ssh-ed25519 {body} me@pc laptop"))
    );
    assert!(ok(&format!("ssh-ed25519 {body}")).is_ok());
    for bad in [
        String::new(),
        format!("ssh-dss {body}"),
        format!("from=\"10.0.0.1\" ssh-ed25519 {body}"),
        format!("ssh-ed25519 {body}\nssh-ed25519 {body}"),
        "ssh-ed25519 short".to_string(),
        format!("ssh-ed25519 {body}$(reboot)"),
        "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
    ] {
        assert!(ok(&bad).is_err(), "{bad:?} was accepted");
    }
}

#[test]
fn port_lists_are_normalised_and_bounded() {
    let ports = spec("ports", ParamKind::PortList);
    let check = |v: &str| check_one(&ports, &json!(v));
    assert_eq!(
        check("80/TCP, 443/tcp 51820/udp,,53 80/tcp").unwrap(),
        ParamValue::Text("80/tcp 443/tcp 51820/udp 53".into())
    );
    assert_eq!(check("").unwrap(), ParamValue::Text(String::new()));
    for bad in ["0", "65536", "22/sctp", "http", "22;reboot", "080", "-1"] {
        assert!(check(bad).is_err(), "{bad} was accepted");
    }
    let many: Vec<String> = (1..=33).map(|n| n.to_string()).collect();
    assert!(check(&many.join(",")).is_err());
}
