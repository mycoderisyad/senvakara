//! SSH keys kept in the Senvakara keychain (CRED-02, ADR-025). Only public
//! material and opaque references live here: the private key and its
//! passphrase are in the OS credential store.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// Longest key label.
pub const MAX_KEY_LABEL_LEN: usize = 64;

/// What kind of key an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    /// A private key whose text is in the OS store.
    #[default]
    Key,
    /// A FIDO2 security key held by the Windows OpenSSH agent (CRED-04);
    /// there is no private key text to store.
    Fido2,
}

impl KeyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyKind::Key => "key",
            KeyKind::Fido2 => "fido2",
        }
    }

    fn parse(value: &str) -> KeyKind {
        match value {
            "fido2" => KeyKind::Fido2,
            _ => KeyKind::Key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRecord {
    pub id: String,
    pub label: String,
    /// Key algorithm as OpenSSH names it, for example `ssh-ed25519`.
    pub algorithm: String,
    /// OpenSSH public key line.
    pub public_key: String,
    /// OpenSSH user certificate line, if the key has one (CRED-03).
    pub certificate: Option<String>,
    /// OS-store reference of the private key text.
    pub private_ref: Option<String>,
    /// OS-store reference of the private key passphrase.
    pub passphrase_ref: Option<String>,
    pub kind: KeyKind,
    pub created_at: String,
    pub updated_at: String,
}

/// The non-secret fields of a keychain entry.
#[derive(Debug, Clone, Deserialize)]
pub struct KeyUpsert {
    pub label: String,
    pub algorithm: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub certificate: Option<String>,
    #[serde(default)]
    pub kind: KeyKind,
}

impl KeyUpsert {
    fn validated(&self) -> Result<KeyUpsert> {
        let label = self.label.trim().to_string();
        let bad = |m: &str| Err(StorageError::Invalid(m.into()));
        if label.is_empty() {
            return bad("a key needs a label");
        }
        if label.chars().count() > MAX_KEY_LABEL_LEN {
            return bad("a key label can be at most 64 characters");
        }
        let algorithm = self.algorithm.trim().to_string();
        if algorithm.is_empty() || algorithm.chars().any(char::is_whitespace) {
            return bad("the key algorithm is not recognised");
        }
        let one_line = |s: &str| s.trim().to_string();
        let public_key = one_line(&self.public_key);
        let certificate = self
            .certificate
            .as_deref()
            .map(one_line)
            .filter(|c| !c.is_empty());
        if public_key.contains('\n') || certificate.as_ref().is_some_and(|c| c.contains('\n')) {
            return bad("a public key or certificate is a single line");
        }
        Ok(KeyUpsert {
            label,
            algorithm,
            public_key,
            certificate,
            kind: self.kind,
        })
    }
}

const SELECT: &str = "SELECT id, label, algorithm, public_key, certificate, private_ref,
       passphrase_ref, kind, created_at, updated_at FROM keychain_keys";

fn map_row(r: &Row<'_>) -> rusqlite::Result<KeyRecord> {
    Ok(KeyRecord {
        id: r.get(0)?,
        label: r.get(1)?,
        algorithm: r.get(2)?,
        public_key: r.get(3)?,
        certificate: r.get(4)?,
        private_ref: r.get(5)?,
        passphrase_ref: r.get(6)?,
        kind: KeyKind::parse(&r.get::<_, String>(7)?),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

impl Db {
    pub fn list_keys(&self) -> Result<Vec<KeyRecord>> {
        let mut st = self
            .conn
            .prepare(&format!("{SELECT} ORDER BY label COLLATE NOCASE"))?;
        let rows = st
            .query_map([], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_key(&self, id: &str) -> Result<KeyRecord> {
        self.conn
            .query_row(&format!("{SELECT} WHERE id=?1"), [id], map_row)
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("key {id}")))
    }

    /// Add a key. `private_ref` must already point at the private key text in
    /// the OS store; a FIDO2 key has none.
    pub fn create_key(&self, input: &KeyUpsert, private_ref: Option<&str>) -> Result<KeyRecord> {
        let v = input.validated()?;
        if v.kind == KeyKind::Key && private_ref.is_none() {
            return Err(StorageError::Invalid("the private key is required".into()));
        }
        let id = new_id("key");
        self.conn.execute(
            "INSERT INTO keychain_keys(id, label, algorithm, public_key, certificate, private_ref, kind)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, v.label, v.algorithm, v.public_key, v.certificate, private_ref, v.kind.as_str()],
        )?;
        self.get_key(&id)
    }

    /// Change the public fields of a key. Its kind cannot change.
    pub fn update_key(&self, id: &str, input: &KeyUpsert) -> Result<KeyRecord> {
        let v = input.validated()?;
        let current = self.get_key(id)?;
        if current.kind != v.kind {
            return Err(StorageError::Invalid(
                "the kind of a key cannot change; add a new key instead".into(),
            ));
        }
        self.conn.execute(
            "UPDATE keychain_keys SET label=?2, algorithm=?3, public_key=?4, certificate=?5,
                    updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, v.label, v.algorithm, v.public_key, v.certificate],
        )?;
        self.get_key(id)
    }

    /// Point a key at new private key text in the OS store.
    pub fn set_key_private_ref(&self, id: &str, reference: &str) -> Result<()> {
        self.get_key(id)?;
        self.conn.execute(
            "UPDATE keychain_keys SET private_ref=?2, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, reference],
        )?;
        Ok(())
    }

    pub fn set_key_passphrase_ref(&self, id: &str, reference: Option<&str>) -> Result<()> {
        self.get_key(id)?;
        self.conn.execute(
            "UPDATE keychain_keys SET passphrase_ref=?2, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, reference],
        )?;
        Ok(())
    }

    /// Names of the hosts that sign in with this key.
    pub fn hosts_using_key(&self, id: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "SELECT h.display_name FROM ssh_settings s
             JOIN host_endpoints e ON e.id = s.endpoint_id
             JOIN hosts h ON h.id = e.host_id
             WHERE s.key_id = ?1 ORDER BY h.display_name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Delete a key no host uses. The caller removes its secrets from the OS
    /// store.
    pub fn delete_key(&self, id: &str) -> Result<()> {
        let users = self.hosts_using_key(id)?;
        if !users.is_empty() {
            return Err(StorageError::Invalid(format!(
                "the key is still used by {}; choose another sign-in for them first",
                users.join(", ")
            )));
        }
        let n = self
            .conn
            .execute("DELETE FROM keychain_keys WHERE id=?1", [id])?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("key {id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostUpsert, Protocol, SshAuth};

    fn key_input(label: &str) -> KeyUpsert {
        KeyUpsert {
            label: label.into(),
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample test".into(),
            certificate: None,
            kind: KeyKind::Key,
        }
    }

    fn host_with_key(key_id: &str) -> HostUpsert {
        HostUpsert {
            display_name: "Hermes VPS".into(),
            os_family: "linux".into(),
            protocol: Protocol::Ssh,
            address: "192.0.2.20".into(),
            port: 22,
            username: "root".into(),
            auth: SshAuth::Keychain {
                key_id: key_id.into(),
            },
            group_name: None,
            tags: Vec::new(),
            startup_snippet: None,
        }
    }

    #[test]
    fn a_key_is_stored_with_its_reference_and_listed_by_label() {
        let db = Db::open_in_memory().unwrap();
        db.create_key(&key_input("wireguard-vps"), Some("secret-b"))
            .unwrap();
        let a = db
            .create_key(&key_input(" hermes_vps "), Some("secret-a"))
            .unwrap();
        assert_eq!(a.label, "hermes_vps");
        assert_eq!(a.private_ref.as_deref(), Some("secret-a"));
        let labels: Vec<_> = db
            .list_keys()
            .unwrap()
            .into_iter()
            .map(|k| k.label)
            .collect();
        assert_eq!(labels, ["hermes_vps", "wireguard-vps"]);
    }

    #[test]
    fn a_plain_key_needs_private_text_but_a_fido2_key_does_not() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.create_key(&key_input("k"), None).is_err());
        let fido = KeyUpsert {
            kind: KeyKind::Fido2,
            algorithm: "sk-ssh-ed25519@openssh.com".into(),
            ..key_input("yubikey")
        };
        assert_eq!(db.create_key(&fido, None).unwrap().kind, KeyKind::Fido2);
    }

    #[test]
    fn bad_labels_and_multi_line_public_keys_are_refused() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.create_key(&key_input("  "), Some("r")).is_err());
        assert!(db
            .create_key(&key_input(&"x".repeat(65)), Some("r"))
            .is_err());
        let two_lines = KeyUpsert {
            public_key: "ssh-ed25519 AAAA\nssh-rsa BBBB".into(),
            ..key_input("k")
        };
        assert!(db.create_key(&two_lines, Some("r")).is_err());
    }

    #[test]
    fn a_key_in_use_cannot_be_deleted_until_the_host_lets_go() {
        let mut db = Db::open_in_memory().unwrap();
        let key = db
            .create_key(&key_input("lightsail"), Some("secret-k"))
            .unwrap();
        let host = db.create_host(&host_with_key(&key.id)).unwrap();
        assert_eq!(db.hosts_using_key(&key.id).unwrap(), ["Hermes VPS"]);
        assert!(matches!(
            db.delete_key(&key.id),
            Err(StorageError::Invalid(_))
        ));
        db.delete_host(&host.id).unwrap();
        db.delete_key(&key.id).unwrap();
        assert!(db.get_key(&key.id).is_err());
    }

    #[test]
    fn a_host_cannot_point_at_a_missing_key() {
        let mut db = Db::open_in_memory().unwrap();
        assert!(matches!(
            db.create_host(&host_with_key("key-missing")),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn updating_keeps_the_kind_and_the_secret_references() {
        let db = Db::open_in_memory().unwrap();
        let key = db.create_key(&key_input("old"), Some("secret-k")).unwrap();
        db.set_key_passphrase_ref(&key.id, Some("secret-p"))
            .unwrap();
        let cert = KeyUpsert {
            certificate: Some(" ssh-ed25519-cert-v01@openssh.com AAAA ".into()),
            ..key_input("new")
        };
        let updated = db.update_key(&key.id, &cert).unwrap();
        assert_eq!(updated.label, "new");
        assert_eq!(
            updated.certificate.as_deref(),
            Some("ssh-ed25519-cert-v01@openssh.com AAAA")
        );
        assert_eq!(updated.private_ref.as_deref(), Some("secret-k"));
        assert_eq!(updated.passphrase_ref.as_deref(), Some("secret-p"));
        let fido = KeyUpsert {
            kind: KeyKind::Fido2,
            ..key_input("new")
        };
        assert!(db.update_key(&key.id, &fido).is_err());
    }
}
