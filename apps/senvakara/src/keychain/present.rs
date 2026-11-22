//! Keys as the Keychain page draws them.

use senvakara_app::{KeyKind, KeyView};

use crate::KeyItem;

/// The key type as people say it ("ED25519"), from the algorithm as OpenSSH names it.
pub fn type_label(algorithm: &str) -> String {
    let security_key = algorithm.starts_with("sk-");
    let name = algorithm
        .trim_start_matches("sk-")
        .split('@')
        .next()
        .unwrap_or_default();
    let base = match name {
        "ssh-ed25519" => "ED25519".to_string(),
        "ssh-rsa" | "rsa-sha2-256" | "rsa-sha2-512" => "RSA".to_string(),
        "ssh-dss" => "DSA".to_string(),
        other => match other.strip_prefix("ecdsa-sha2-nistp") {
            Some(bits) => format!("ECDSA P-{bits}"),
            None => other.to_uppercase(),
        },
    };
    if security_key {
        format!("{base} (FIDO2)")
    } else {
        base
    }
}

/// How many hosts sign in with a key, in words.
pub fn used_text(hosts: usize) -> String {
    match hosts {
        0 => "Not used".to_string(),
        1 => "Used by 1 host".to_string(),
        many => format!("Used by {many} hosts"),
    }
}

pub(super) fn key_item(key: &KeyView) -> KeyItem {
    KeyItem {
        id: key.id.as_str().into(),
        label: key.label.as_str().into(),
        type_text: format!("Type {}", type_label(&key.algorithm)).into(),
        kind: match key.kind {
            KeyKind::Key => "key",
            KeyKind::Fido2 => "fido2",
        }
        .into(),
        certificate: key.certificate.is_some(),
        used: used_text(key.used_by.len()).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_types_are_named_the_way_people_say_them() {
        assert_eq!(type_label("ssh-ed25519"), "ED25519");
        assert_eq!(type_label("ssh-rsa"), "RSA");
        assert_eq!(type_label("rsa-sha2-512"), "RSA");
        assert_eq!(type_label("ecdsa-sha2-nistp256"), "ECDSA P-256");
        assert_eq!(type_label("ecdsa-sha2-nistp521"), "ECDSA P-521");
        assert_eq!(type_label("sk-ssh-ed25519@openssh.com"), "ED25519 (FIDO2)");
        assert_eq!(
            type_label("sk-ecdsa-sha2-nistp256@openssh.com"),
            "ECDSA P-256 (FIDO2)"
        );
        assert_eq!(type_label("something-new"), "SOMETHING-NEW");
    }

    #[test]
    fn a_card_says_how_many_hosts_use_the_key() {
        assert_eq!(used_text(0), "Not used");
        assert_eq!(used_text(1), "Used by 1 host");
        assert_eq!(used_text(3), "Used by 3 hosts");
    }
}
