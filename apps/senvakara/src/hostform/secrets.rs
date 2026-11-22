//! What saving the host form does to the host's secrets. A password or passphrase is never
//! read back: the form only knows whether one is saved, and what was typed just now.

use senvakara_app::Protocol;

use super::validation::{Method, Password};

/// What the secret fields of the form hold.
#[derive(Clone, Default)]
pub struct Typed {
    /// "empty" | "saved" | "replace" | "remove": what the password part of the form shows.
    pub password_mode: String,
    pub password: String,
    pub passphrase: String,
    /// "Remove saved passphrase" is ticked.
    pub remove_passphrase: bool,
}

/// Which secrets the host had when the form opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub password: bool,
    pub passphrase: bool,
}

/// One secret to store (`Some`) or remove (`None`) once the host itself is saved.
#[derive(Clone, PartialEq, Eq)]
pub enum Change {
    Password(Option<String>),
    Passphrase(Option<String>),
    RdpPassword(Option<String>),
}

// By hand, so that a secret can never end up in a log or a failed assertion.
impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, value) = match self {
            Change::Password(value) => ("Password", value),
            Change::Passphrase(value) => ("Passphrase", value),
            Change::RdpPassword(value) => ("RdpPassword", value),
        };
        write!(
            f,
            "{name}({})",
            if value.is_some() { "set" } else { "clear" }
        )
    }
}

/// Whether the host will have a password once the form is saved.
pub fn password_state(mode: &str, typed: &str) -> Password {
    match mode {
        "remove" => Password::Missing,
        _ if !typed.is_empty() => Password::Typed,
        "saved" | "replace" => Password::Saved,
        _ => Password::Missing,
    }
}

/// The password to store or remove, if the form changes it at all.
fn password_change(typed: &Typed) -> Option<Option<String>> {
    match typed.password_mode.as_str() {
        "remove" => Some(None),
        "saved" => None,
        _ if !typed.password.is_empty() => Some(Some(typed.password.clone())),
        _ => None,
    }
}

/// Everything to do to the host's secrets after it is saved, in order. A secret the chosen
/// sign-in method no longer uses is removed, so nothing unused stays in the credential store.
pub fn changes(protocol: Protocol, method: Method, typed: &Typed, saved: Saved) -> Vec<Change> {
    let mut changes = Vec::new();
    if protocol == Protocol::Rdp {
        changes.extend(password_change(typed).map(Change::RdpPassword));
        return changes;
    }
    if method == Method::Password {
        changes.extend(password_change(typed).map(Change::Password));
    } else if saved.password {
        changes.push(Change::Password(None));
    }
    if method == Method::KeyFile {
        if !typed.passphrase.is_empty() {
            changes.push(Change::Passphrase(Some(typed.passphrase.clone())));
        } else if typed.remove_passphrase && saved.passphrase {
            changes.push(Change::Passphrase(None));
        }
    } else if saved.passphrase {
        changes.push(Change::Passphrase(None));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(mode: &str, password: &str) -> Typed {
        Typed {
            password_mode: mode.into(),
            password: password.into(),
            ..Typed::default()
        }
    }

    const NOTHING: Saved = Saved {
        password: false,
        passphrase: false,
    };

    #[test]
    fn a_password_counts_when_typed_or_saved_and_not_when_being_removed() {
        assert_eq!(password_state("empty", ""), Password::Missing);
        assert_eq!(password_state("empty", "hunter2"), Password::Typed);
        assert_eq!(password_state("saved", ""), Password::Saved);
        assert_eq!(password_state("replace", ""), Password::Saved);
        assert_eq!(password_state("replace", "new"), Password::Typed);
        assert_eq!(password_state("remove", "left over"), Password::Missing);
    }

    #[test]
    fn a_typed_password_is_stored_and_a_kept_one_is_left_alone() {
        let ssh = |typed: &Typed, saved| changes(Protocol::Ssh, Method::Password, typed, saved);
        assert_eq!(
            ssh(&typed("empty", "hunter2"), NOTHING),
            [Change::Password(Some("hunter2".into()))]
        );
        let has = Saved {
            password: true,
            passphrase: false,
        };
        assert!(ssh(&typed("saved", ""), has).is_empty());
        assert!(ssh(&typed("replace", ""), has).is_empty());
        assert_eq!(
            ssh(&typed("replace", "new"), has),
            [Change::Password(Some("new".into()))]
        );
        assert_eq!(ssh(&typed("remove", ""), has), [Change::Password(None)]);
        assert!(ssh(&typed("empty", ""), NOTHING).is_empty());
    }

    #[test]
    fn switching_to_a_key_removes_the_password_that_is_no_longer_used() {
        let has = Saved {
            password: true,
            passphrase: true,
        };
        assert_eq!(
            changes(Protocol::Ssh, Method::Keychain, &typed("saved", ""), has),
            [Change::Password(None), Change::Passphrase(None)]
        );
        assert!(changes(Protocol::Ssh, Method::Agent, &typed("empty", "x"), NOTHING).is_empty());
        // A key file keeps its passphrase; only the password goes.
        assert_eq!(
            changes(Protocol::Ssh, Method::KeyFile, &typed("saved", ""), has),
            [Change::Password(None)]
        );
    }

    #[test]
    fn a_key_file_passphrase_is_stored_replaced_or_removed() {
        let key = |passphrase: &str, remove, saved| {
            let typed = Typed {
                password_mode: "empty".into(),
                passphrase: passphrase.into(),
                remove_passphrase: remove,
                ..Typed::default()
            };
            changes(Protocol::Ssh, Method::KeyFile, &typed, saved)
        };
        let has = Saved {
            password: false,
            passphrase: true,
        };
        assert_eq!(
            key("open sesame", false, NOTHING),
            [Change::Passphrase(Some("open sesame".into()))]
        );
        assert!(key("", false, has).is_empty());
        assert_eq!(key("", true, has), [Change::Passphrase(None)]);
        assert!(key("", true, NOTHING).is_empty());
    }

    #[test]
    fn a_remote_desktop_host_only_ever_changes_its_own_password() {
        let rdp = |typed: &Typed| changes(Protocol::Rdp, Method::Password, typed, NOTHING);
        assert_eq!(
            rdp(&typed("empty", "hunter2")),
            [Change::RdpPassword(Some("hunter2".into()))]
        );
        assert_eq!(rdp(&typed("remove", "")), [Change::RdpPassword(None)]);
        assert!(rdp(&typed("saved", "")).is_empty());
        assert!(rdp(&typed("empty", "")).is_empty());
    }

    #[test]
    fn a_change_never_prints_its_secret() {
        let shown = format!("{:?}", Change::Password(Some("hunter2".into())));
        assert_eq!(shown, "Password(set)");
        assert_eq!(
            format!("{:?}", Change::Passphrase(None)),
            "Passphrase(clear)"
        );
    }
}
