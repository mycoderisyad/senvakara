//! What a host script learns about the session that runs it. These values
//! come from Hosts, not from node settings, so an SSH or firewall module can
//! refuse a change that would lock this very session out (plan WF §4).

use crate::params::shell_quote;

/// Names a module parameter may not take, because the engine sets
/// `SV_SESSION_*`, `SV_GUARD_*`, `SV_MODULE_DIR` and `SV_HEALTH_WAIT` itself.
pub(crate) const RESERVED_PREFIXES: [&str; 4] = ["session_", "guard_", "module_", "health_"];

/// The SSH session a run uses on one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The account Senvakara signs in as.
    pub user: String,
    /// The port that session reached sshd on.
    pub port: u16,
    /// The session signed in with a key (file, keychain or agent), not a
    /// password.
    pub key_login: bool,
}

/// `export` lines a script gets after its settings: `SV_SESSION_USER`,
/// `SV_SESSION_PORT`, `SV_SESSION_AUTH` (`key` or `password`) and, for a
/// guarded module, `SV_GUARD_DIR`, where `apply.sh` keeps the backup that
/// `rollback.sh` restores.
pub fn session_exports(session: &Session, guard_dir: Option<&str>) -> String {
    let mut out = format!(
        "export SV_SESSION_USER={}\nexport SV_SESSION_PORT={}\nexport SV_SESSION_AUTH={}\n",
        shell_quote(&session.user),
        session.port,
        if session.key_login { "key" } else { "password" },
    );
    if let Some(dir) = guard_dir {
        out.push_str(&format!("export SV_GUARD_DIR={}\n", shell_quote(dir)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_values_are_quoted_words() {
        let s = Session {
            user: "o'brien; rm -rf /".into(),
            port: 2222,
            key_login: true,
        };
        let text = session_exports(&s, Some("/var/lib/senvakara/guard/sv-guard-1"));
        assert_eq!(
            text,
            "export SV_SESSION_USER='o'\\''brien; rm -rf /'\n\
             export SV_SESSION_PORT=2222\n\
             export SV_SESSION_AUTH=key\n\
             export SV_GUARD_DIR='/var/lib/senvakara/guard/sv-guard-1'\n"
        );
        let plain = session_exports(
            &Session {
                key_login: false,
                ..s
            },
            None,
        );
        assert!(plain.contains("SV_SESSION_AUTH=password\n"));
        assert!(!plain.contains("SV_GUARD_DIR"));
    }
}
