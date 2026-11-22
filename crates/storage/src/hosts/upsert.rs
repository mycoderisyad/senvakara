//! Input for creating or editing a host, and the rules it must follow
//! before anything is written (HOST-UI-02, CRED-01..04, DESK-IMPL-04).

use crate::{Result, StorageError};
use serde::{Deserialize, Serialize};

/// Longest parent group name.
pub const MAX_GROUP_LEN: usize = 64;
/// Longest single tag.
pub const MAX_TAG_LEN: usize = 32;
/// Most tags one host may carry.
pub const MAX_TAGS: usize = 16;
/// Longest startup snippet, in bytes.
pub const MAX_SNIPPET_LEN: usize = 4096;

/// How the host is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Terminal, files and the rest over SSH.
    #[default]
    Ssh,
    /// Windows Remote Desktop, opened in `mstsc.exe` (ADR-026).
    Rdp,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Ssh => "ssh",
            Protocol::Rdp => "rdp",
        }
    }

    pub(crate) fn parse(value: &str) -> Protocol {
        match value {
            "rdp" => Protocol::Rdp,
            _ => Protocol::Ssh,
        }
    }
}

/// How Senvakara signs in to an SSH host. Secrets themselves are never part
/// of this: a password or passphrase is referenced from the OS store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SshAuth {
    /// A private key file on this PC.
    KeyFile { path: String },
    /// A key stored in the Senvakara keychain.
    Keychain { key_id: String },
    /// A password kept in the OS credential store.
    Password,
    /// A key held by the Windows OpenSSH agent, such as a FIDO2 key.
    Agent,
}

impl SshAuth {
    pub fn method(&self) -> &'static str {
        match self {
            SshAuth::KeyFile { .. } => "key_file",
            SshAuth::Keychain { .. } => "keychain",
            SshAuth::Password => "password",
            SshAuth::Agent => "agent",
        }
    }

    /// Rebuild from the stored columns. An unknown method or a missing
    /// column falls back to an empty key file, which fails at connect time
    /// with a clear message instead of failing to list hosts.
    pub(crate) fn from_columns(
        method: &str,
        path: Option<String>,
        key_id: Option<String>,
    ) -> SshAuth {
        match (method, path, key_id) {
            ("keychain", _, Some(key_id)) => SshAuth::Keychain { key_id },
            ("password", _, _) => SshAuth::Password,
            ("agent", _, _) => SshAuth::Agent,
            (_, path, _) => SshAuth::KeyFile {
                path: path.unwrap_or_default(),
            },
        }
    }

    pub(crate) fn key_path(&self) -> Option<&str> {
        match self {
            SshAuth::KeyFile { path } => Some(path),
            _ => None,
        }
    }

    pub(crate) fn key_id(&self) -> Option<&str> {
        match self {
            SshAuth::Keychain { key_id } => Some(key_id),
            _ => None,
        }
    }
}

/// Input for creating or editing a remote host.
#[derive(Debug, Clone, Deserialize)]
pub struct HostUpsert {
    pub display_name: String,
    pub os_family: String,
    #[serde(default)]
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    pub username: String,
    /// Sign-in method for SSH hosts. RDP hosts always use a password.
    pub auth: SshAuth,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Text typed into the shell once after an SSH login.
    #[serde(default)]
    pub startup_snippet: Option<String>,
}

/// A hostname or IP: no spaces, paths or user part.
pub(crate) fn valid_address(a: &str) -> bool {
    !a.is_empty() && a.len() <= 253 && !a.chars().any(|c| c.is_whitespace() || c == '/' || c == '@')
}

fn bad<T>(message: &str) -> Result<T> {
    Err(StorageError::Invalid(message.into()))
}

/// Trimmed, de-duplicated tags (case-insensitive, first spelling wins).
fn clean_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if tag.chars().count() > MAX_TAG_LEN {
            return bad("a tag can be at most 32 characters");
        }
        if tag.contains(',') {
            return bad("a tag cannot contain a comma");
        }
        if !out.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            out.push(tag.to_string());
        }
    }
    if out.len() > MAX_TAGS {
        return bad("a host can have at most 16 tags");
    }
    Ok(out)
}

impl HostUpsert {
    pub(crate) fn validated(&self) -> Result<HostUpsert> {
        let t = |s: &str| s.trim().to_string();
        let auth = match &self.auth {
            SshAuth::KeyFile { path } => SshAuth::KeyFile { path: t(path) },
            SshAuth::Keychain { key_id } => SshAuth::Keychain { key_id: t(key_id) },
            other => other.clone(),
        };
        let group_name = self
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_string);
        let startup_snippet = self
            .startup_snippet
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let v = HostUpsert {
            display_name: t(&self.display_name),
            os_family: t(&self.os_family),
            protocol: self.protocol,
            address: t(&self.address),
            port: self.port,
            username: t(&self.username),
            auth,
            group_name,
            tags: clean_tags(&self.tags)?,
            startup_snippet,
        };
        if v.display_name.is_empty() {
            return bad("display name is required");
        }
        if !["windows", "linux", "macos", "unknown"].contains(&v.os_family.as_str()) {
            return bad("unsupported OS family");
        }
        if !valid_address(&v.address) {
            return bad("address must be a hostname or IP without spaces");
        }
        if v.port == 0 {
            return bad("port must be 1-65535");
        }
        if v.username.chars().any(char::is_whitespace) {
            return bad("username cannot contain spaces");
        }
        if v.group_name
            .as_ref()
            .is_some_and(|g| g.chars().count() > MAX_GROUP_LEN)
        {
            return bad("a group name can be at most 64 characters");
        }
        match v.protocol {
            Protocol::Ssh => {
                if v.username.is_empty() {
                    return bad("username is required");
                }
                match &v.auth {
                    SshAuth::KeyFile { path } if path.is_empty() => {
                        return bad("private key path is required")
                    }
                    SshAuth::Keychain { key_id } if key_id.is_empty() => {
                        return bad("choose a key from the keychain")
                    }
                    _ => {}
                }
                if v.startup_snippet
                    .as_ref()
                    .is_some_and(|s| s.len() > MAX_SNIPPET_LEN)
                {
                    return bad("the startup snippet can be at most 4096 bytes");
                }
            }
            Protocol::Rdp => {
                if v.auth != SshAuth::Password {
                    return bad("remote desktop hosts sign in with a password");
                }
                if v.startup_snippet.is_some() {
                    return bad("startup snippets only run on SSH hosts");
                }
            }
        }
        Ok(v)
    }
}
