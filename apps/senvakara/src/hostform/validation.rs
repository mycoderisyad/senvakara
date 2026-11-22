//! Checking what the host form and the VPN address fields hold, without touching the UI.

use senvakara_app::{HostUpsert, Protocol, SshAuth};

/// A host may have this many VPN addresses (the storage layer's limit).
const MAX_VPN: usize = 4;
/// A host may carry this many tags, each this long, and a group name this long (the storage
/// layer's limits).
const MAX_TAGS: usize = 16;
const MAX_TAG_CHARS: usize = 32;
const MAX_GROUP_CHARS: usize = 64;
/// The longest startup snippet, in bytes.
const MAX_SNIPPET_BYTES: usize = 4096;

/// What an SSH host signs in with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    #[default]
    Password,
    /// A private key file on this PC.
    KeyFile,
    /// A key of the Keychain.
    Keychain,
    /// The Windows OpenSSH agent (a FIDO2 security key).
    Agent,
}

impl Method {
    /// The name the form uses for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Password => "password",
            Method::KeyFile => "key_file",
            Method::Keychain => "keychain",
            Method::Agent => "agent",
        }
    }
}

/// What the form holds, as typed. No secret is part of it: a password or passphrase stays in
/// its field until it is saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub protocol: Protocol,
    pub name: String,
    pub address: String,
    pub port: String,
    pub username: String,
    /// "linux" or "unknown".
    pub os: String,
    pub group: String,
    pub tags: Vec<String>,
    pub method: Method,
    pub key_path: String,
    /// The Keychain key, when `method` is [`Method::Keychain`].
    pub key_id: String,
    pub snippet: String,
}

/// What the form knows about the host's password without ever holding a saved one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Password {
    /// None is saved and none is typed, or the saved one is being removed.
    #[default]
    Missing,
    /// One is saved and stays.
    Saved,
    /// One is typed and will be saved.
    Typed,
}

/// What is wrong with each field, in words. An empty string means the field is fine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub address: &'static str,
    pub group: &'static str,
    pub port: &'static str,
    pub username: &'static str,
    pub password: &'static str,
    pub key: &'static str,
    pub snippet: &'static str,
}

impl Problems {
    pub fn any(&self) -> bool {
        [
            self.address,
            self.group,
            self.port,
            self.username,
            self.password,
            self.key,
            self.snippet,
        ]
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

/// Why `tag` cannot be added to `tags`, or "" when it can.
pub fn tag_problem(tag: &str, tags: &[String]) -> &'static str {
    let tag = tag.trim();
    if tag.is_empty() {
        "Type the tag first."
    } else if tag.contains(',') {
        "A tag cannot contain a comma. Add tags one at a time."
    } else if tag.chars().count() > MAX_TAG_CHARS {
        "A tag can be at most 32 characters."
    } else if tags.iter().any(|have| have.eq_ignore_ascii_case(tag)) {
        "This host already has that tag."
    } else if tags.len() >= MAX_TAGS {
        "A host can have at most 16 tags. Remove one first."
    } else {
        ""
    }
}

/// A Remote Desktop host keeps its saved password only while its address and user name stay
/// the same: Windows files the credential under both. True when an edit would lose it.
pub fn rdp_password_lost(before: &Fields, now: &Fields) -> bool {
    now.protocol == Protocol::Rdp
        && (!before
            .address
            .trim()
            .eq_ignore_ascii_case(now.address.trim())
            || before.username.trim() != now.username.trim())
}

/// How the host signs in, or the problem with what was chosen.
fn sign_in(fields: &Fields, password: Password, problems: &mut Problems) -> SshAuth {
    if fields.protocol == Protocol::Rdp {
        return SshAuth::Password;
    }
    match fields.method {
        Method::Password => {
            if password == Password::Missing {
                problems.password = "Enter the password, or choose a key to sign in with.";
            }
            SshAuth::Password
        }
        Method::KeyFile => {
            let path = fields.key_path.trim();
            if path.is_empty() {
                problems.key = "Choose the private key file to sign in with.";
            }
            SshAuth::KeyFile {
                path: path.to_string(),
            }
        }
        Method::Keychain => {
            if fields.key_id.is_empty() {
                problems.key = "Choose a key from the Keychain.";
            }
            SshAuth::Keychain {
                key_id: fields.key_id.clone(),
            }
        }
        Method::Agent => SshAuth::Agent,
    }
}

/// Check the form. `password` says whether the host will have a password once saved. Ok
/// carries what the backend stores.
pub fn check(fields: &Fields, password: Password) -> Result<HostUpsert, Problems> {
    let mut problems = Problems::default();
    let ssh = fields.protocol == Protocol::Ssh;
    problems.address = address_problem(&fields.address);
    // A host without a label is listed by its address.
    let name = match fields.name.trim() {
        "" => fields.address.trim(),
        name => name,
    };
    let port = parse_port(&fields.port);
    if let Err(problem) = port {
        problems.port = if !ssh && fields.port.trim().is_empty() {
            "Enter the Remote Desktop port. The usual one is 3389."
        } else {
            problem
        };
    }
    let username = fields.username.trim();
    if username.chars().any(char::is_whitespace) {
        problems.username = "A user name cannot contain spaces.";
    } else if username.is_empty() && ssh {
        problems.username = "Enter the user name to sign in with.";
    } else if username.is_empty() && password == Password::Typed {
        problems.username = "Enter the user name the password belongs to.";
    }
    let group = fields.group.trim();
    if group.chars().count() > MAX_GROUP_CHARS {
        problems.group = "A group name can be at most 64 characters.";
    }
    let auth = sign_in(fields, password, &mut problems);
    let snippet = fields.snippet.trim_end();
    if ssh && snippet.len() > MAX_SNIPPET_BYTES {
        problems.snippet = "The startup snippet is too long. Keep it under 4096 bytes.";
    }
    match port {
        Ok(port) if !problems.any() => Ok(HostUpsert {
            display_name: name.to_string(),
            os_family: if !ssh {
                "windows"
            } else if fields.os == "unknown" {
                "unknown"
            } else {
                "linux"
            }
            .to_string(),
            protocol: fields.protocol,
            address: fields.address.trim().to_string(),
            port,
            username: username.to_string(),
            auth,
            group_name: (!group.is_empty()).then(|| group.to_string()),
            tags: fields.tags.clone(),
            startup_snippet: (ssh && !snippet.trim().is_empty()).then(|| snippet.to_string()),
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
mod tests;
