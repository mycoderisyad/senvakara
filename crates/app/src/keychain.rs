//! The keychain of SSH keys (CRED-02, CRED-03, CRED-04; ADR-025).
//!
//! A key's private text and passphrase live only in the OS credential store;
//! the database row holds the label, algorithm, public key, certificate and
//! the opaque references. The UI gets a [`KeyView`], where secrets are
//! reduced to booleans. A FIDO2 entry has no private text at all: it names a
//! key the Windows OpenSSH agent holds (ADR-027).
//!
//! Everything here blocks (credential store, key decryption, file reads), so
//! the `App` facade runs it on the blocking pool.

use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
use senvakara_adapters_ssh::keys::{
    check_certificate, inspect_private_key, inspect_public_key, same_public_key, KeyInfo,
};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_secrets::SecretStore;
use senvakara_storage::{KeyKind, KeyRecord, KeyUpsert};
use serde::Serialize;
use std::path::Path;

/// Largest file accepted as a key file; real keys are a few kilobytes.
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

/// What the UI may see about a key. Secrets are reduced to booleans.
#[derive(Debug, Clone, Serialize)]
pub struct KeyView {
    pub id: String,
    pub label: String,
    /// Algorithm as OpenSSH names it, for example `ssh-ed25519`.
    pub algorithm: String,
    pub public_key: String,
    pub certificate: Option<String>,
    pub kind: KeyKind,
    pub has_private_key: bool,
    pub has_passphrase: bool,
    /// Names of the hosts that sign in with this key.
    pub used_by: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A new keychain entry. Not `Debug`: it carries the private key.
#[derive(Clone, Default)]
pub struct KeyCreate {
    pub label: String,
    pub kind: KeyKind,
    /// Private key text (OpenSSH, PEM or PuTTY). Unused for a FIDO2 entry.
    pub private_key: String,
    /// Passphrase of an encrypted private key.
    pub passphrase: Option<String>,
    /// OpenSSH public key line; derived from the private key when absent.
    /// Required for a FIDO2 entry.
    pub public_key: Option<String>,
    /// OpenSSH user certificate line (the `-cert.pub` file).
    pub certificate: Option<String>,
}

/// Changes to a keychain entry. Not `Debug`: it may carry a private key.
#[derive(Clone, Default)]
pub struct KeyUpdate {
    pub label: String,
    /// The certificate to keep; `None` or empty removes it.
    pub certificate: Option<String>,
    /// New private key text; `None` keeps the stored one.
    pub private_key: Option<String>,
    /// With a new private key: its passphrase (`None` for an unencrypted
    /// key). Without one: a new passphrase for the stored key, checked
    /// against it; `None` keeps the stored passphrase.
    pub passphrase: Option<String>,
}

/// A key file read for the "new key" form. Not `Debug`: it carries the key.
#[derive(Clone)]
pub struct KeyImport {
    /// Suggested label: the file name.
    pub label: String,
    pub private_key: String,
    /// Contents of `<file>.pub` next to it, if present.
    pub public_key: Option<String>,
    /// Contents of `<file>-cert.pub` next to it, if present.
    pub certificate: Option<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidInput, message)
}

fn store(state: &AppState) -> CmdResult<&SecretStore> {
    state.secrets.as_ref().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "credential store unavailable",
        )
    })
}

fn filled(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

fn view(state: &AppState, key: KeyRecord) -> CmdResult<KeyView> {
    let used_by = state
        .db
        .lock()
        .unwrap()
        .hosts_using_key(&key.id)
        .map_err(|e| db_err("keychain", e))?;
    Ok(KeyView {
        has_private_key: key.private_ref.is_some(),
        has_passphrase: key.passphrase_ref.is_some(),
        id: key.id,
        label: key.label,
        algorithm: key.algorithm,
        public_key: key.public_key,
        certificate: key.certificate,
        kind: key.kind,
        used_by,
        created_at: key.created_at,
        updated_at: key.updated_at,
    })
}

pub(crate) fn keychain_list(state: &AppState) -> CmdResult<Vec<KeyView>> {
    let keys = state
        .db
        .lock()
        .unwrap()
        .list_keys()
        .map_err(|e| db_err("list keys", e))?;
    keys.into_iter().map(|key| view(state, key)).collect()
}

/// Unlock `private_key` to prove the passphrase works and describe the key.
/// A public key line the user supplied must belong to it and is kept (it
/// carries their comment); otherwise the derived line is used.
fn describe(
    private_key: &str,
    passphrase: Option<&str>,
    public_key: Option<&str>,
) -> CmdResult<KeyInfo> {
    let mut info =
        inspect_private_key(private_key, passphrase).map_err(|e| to_core("read key", e))?;
    if let Some(line) = public_key {
        if !same_public_key(line, &info.public_key) {
            return Err(invalid(
                "the public key does not belong to this private key; leave it empty to derive it",
            ));
        }
        info.public_key = line.to_string();
    }
    Ok(info)
}

fn checked_certificate(certificate: Option<&str>, public_key: &str) -> CmdResult<Option<String>> {
    let Some(line) = filled(certificate) else {
        return Ok(None);
    };
    check_certificate(line, public_key).map_err(|e| to_core("read certificate", e))?;
    Ok(Some(line.to_string()))
}

/// Private text and passphrase written to the OS store, not yet referenced
/// by the database. [`Stored::discard`] removes them again.
struct Stored {
    private_ref: String,
    passphrase_ref: Option<String>,
}

impl Stored {
    fn write(store: &SecretStore, private_key: &str, passphrase: Option<&str>) -> CmdResult<Self> {
        let private_ref = store
            .put_large(private_key)
            .map_err(|e| to_core("save private key", e.into()))?;
        let passphrase_ref = match passphrase.map(|p| store.put(p)) {
            None => None,
            Some(Ok(reference)) => Some(reference),
            Some(Err(error)) => {
                let _ = store.delete_large(&private_ref);
                return Err(to_core("save passphrase", error.into()));
            }
        };
        Ok(Self {
            private_ref,
            passphrase_ref,
        })
    }

    fn discard(&self, store: &SecretStore) {
        let _ = store.delete_large(&self.private_ref);
        if let Some(reference) = &self.passphrase_ref {
            let _ = store.delete(reference);
        }
    }
}

pub(crate) fn keychain_create(state: &AppState, input: KeyCreate) -> CmdResult<KeyView> {
    let public_key = filled(input.public_key.as_deref());
    if input.kind == KeyKind::Fido2 {
        return create_fido2(state, &input, public_key);
    }
    // Stored exactly as given, so the key file round-trips byte for byte.
    let private_key = input.private_key.as_str();
    if private_key.trim().is_empty() {
        return Err(invalid("paste the private key or import it from a file"));
    }
    let passphrase = input.passphrase.as_deref().filter(|p| !p.is_empty());
    let info = describe(private_key, passphrase, public_key)?;
    let certificate = checked_certificate(input.certificate.as_deref(), &info.public_key)?;
    let upsert = KeyUpsert {
        label: input.label.clone(),
        algorithm: info.algorithm,
        public_key: info.public_key,
        certificate,
        kind: KeyKind::Key,
    };
    let store = store(state)?;
    let stored = Stored::write(store, private_key, passphrase)?;
    // If the row cannot be written, the secrets must not stay behind.
    let saved = (|| {
        let db = state.db.lock().unwrap();
        let key = db.create_key(&upsert, Some(&stored.private_ref))?;
        if let Some(reference) = &stored.passphrase_ref {
            if let Err(error) = db.set_key_passphrase_ref(&key.id, Some(reference)) {
                let _ = db.delete_key(&key.id);
                return Err(error);
            }
        }
        db.get_key(&key.id)
    })();
    match saved {
        Ok(key) => {
            logging::info(&format!("key added to the keychain {}", key.id));
            view(state, key)
        }
        Err(error) => {
            stored.discard(store);
            Err(db_err("add key", error))
        }
    }
}

fn create_fido2(
    state: &AppState,
    input: &KeyCreate,
    public_key: Option<&str>,
) -> CmdResult<KeyView> {
    let line = public_key.ok_or_else(|| {
        invalid("a FIDO2 key is added by its public key line (the .pub file ssh-keygen wrote)")
    })?;
    let info = inspect_public_key(line).map_err(|e| to_core("read public key", e))?;
    if !info.algorithm.starts_with("sk-") {
        return Err(invalid(
            "this is not a FIDO2 public key: its type starts with sk-, for example sk-ssh-ed25519@openssh.com",
        ));
    }
    let certificate = checked_certificate(input.certificate.as_deref(), line)?;
    let upsert = KeyUpsert {
        label: input.label.clone(),
        algorithm: info.algorithm,
        public_key: line.to_string(),
        certificate,
        kind: KeyKind::Fido2,
    };
    let key = state
        .db
        .lock()
        .unwrap()
        .create_key(&upsert, None)
        .map_err(|e| db_err("add key", e))?;
    logging::info(&format!("FIDO2 key added to the keychain {}", key.id));
    view(state, key)
}

pub(crate) fn keychain_update(state: &AppState, id: &str, input: KeyUpdate) -> CmdResult<KeyView> {
    let current = state
        .db
        .lock()
        .unwrap()
        .get_key(id)
        .map_err(|e| db_err("edit key", e))?;
    let new_private = input
        .private_key
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    let passphrase = input.passphrase.as_deref().filter(|p| !p.is_empty());
    if current.kind == KeyKind::Fido2 && (new_private.is_some() || passphrase.is_some()) {
        return Err(invalid(
            "a FIDO2 key has no private key or passphrase to save",
        ));
    }

    // Check everything before anything is written.
    let mut upsert = KeyUpsert {
        label: input.label.clone(),
        algorithm: current.algorithm.clone(),
        public_key: current.public_key.clone(),
        certificate: None,
        kind: current.kind,
    };
    if let Some(private_key) = new_private {
        let info = describe(private_key, passphrase, None)?;
        upsert.algorithm = info.algorithm;
        upsert.public_key = info.public_key;
    } else if let Some(passphrase) = passphrase {
        let stored = stored_private_key(state, &current)?;
        describe(&stored, Some(passphrase), None)?;
    }
    upsert.certificate = checked_certificate(input.certificate.as_deref(), &upsert.public_key)?;

    // New secrets first, then the row, then the old secrets go.
    let mut stale: Vec<(bool, String)> = Vec::new();
    if let Some(private_key) = new_private {
        let store = store(state)?;
        let stored = Stored::write(store, private_key, passphrase)?;
        let swapped = (|| {
            let db = state.db.lock().unwrap();
            db.update_key(id, &upsert)?;
            db.set_key_private_ref(id, &stored.private_ref)?;
            db.set_key_passphrase_ref(id, stored.passphrase_ref.as_deref())
        })();
        if let Err(error) = swapped {
            stored.discard(store);
            return Err(db_err("edit key", error));
        }
        stale.extend(current.private_ref.map(|r| (true, r)));
        stale.extend(current.passphrase_ref.map(|r| (false, r)));
    } else {
        let new_ref = match passphrase {
            Some(p) => Some(
                store(state)?
                    .put(p)
                    .map_err(|e| to_core("save passphrase", e.into()))?,
            ),
            None => None,
        };
        let swapped = (|| {
            let db = state.db.lock().unwrap();
            db.update_key(id, &upsert)?;
            match &new_ref {
                Some(reference) => db.set_key_passphrase_ref(id, Some(reference)),
                None => Ok(()),
            }
        })();
        if let Err(error) = swapped {
            if let (Some(reference), Some(store)) = (&new_ref, &state.secrets) {
                let _ = store.delete(reference);
            }
            return Err(db_err("edit key", error));
        }
        if new_ref.is_some() {
            stale.extend(current.passphrase_ref.map(|r| (false, r)));
        }
    }
    if let Some(store) = &state.secrets {
        for (large, reference) in stale {
            let _ = if large {
                store.delete_large(&reference)
            } else {
                store.delete(&reference)
            };
        }
    }
    logging::info(&format!("key updated in the keychain {id}"));
    let key = state
        .db
        .lock()
        .unwrap()
        .get_key(id)
        .map_err(|e| db_err("edit key", e))?;
    view(state, key)
}

fn stored_private_key(state: &AppState, key: &KeyRecord) -> CmdResult<String> {
    let missing = || invalid("the private key is no longer in the credential store: add it again");
    let reference = key.private_ref.as_deref().ok_or_else(missing)?;
    store(state)?
        .get_large(reference)
        .map_err(|e| to_core("read private key", e.into()))?
        .ok_or_else(missing)
}

/// Remove a key no host uses: the row first, so a failure in the OS store
/// can never leave a row that points at deleted secrets.
pub(crate) fn keychain_delete(state: &AppState, id: &str) -> CmdResult<()> {
    let key = {
        let db = state.db.lock().unwrap();
        let key = db.get_key(id).map_err(|e| db_err("delete key", e))?;
        db.delete_key(id).map_err(|e| db_err("delete key", e))?;
        key
    };
    if let Some(store) = &state.secrets {
        if let Some(reference) = &key.private_ref {
            store
                .delete_large(reference)
                .map_err(|e| to_core("delete private key", e.into()))?;
        }
        if let Some(reference) = &key.passphrase_ref {
            store
                .delete(reference)
                .map_err(|e| to_core("delete passphrase", e.into()))?;
        }
    }
    logging::info(&format!("key deleted from the keychain {id}"));
    Ok(())
}

/// One line of text from a small file next to the key, if it is there.
fn sibling(path: &Path, suffix: &str) -> Option<String> {
    let mut name = path.file_name()?.to_os_string();
    name.push(suffix);
    let file = path.with_file_name(name);
    if std::fs::metadata(&file).ok()?.len() > MAX_KEY_FILE_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(file).ok()?;
    let line = text.trim();
    (!line.is_empty() && !line.contains('\n')).then(|| line.to_string())
}

/// Read a private key file for the "new key" form. Nothing is stored yet.
/// The path is the user's own choice; it is not echoed in errors or logs.
pub(crate) fn keychain_import_file(path: &Path) -> CmdResult<KeyImport> {
    let unreadable = || invalid("the key file cannot be read");
    let size = std::fs::metadata(path).map_err(|_| unreadable())?.len();
    if size > MAX_KEY_FILE_BYTES {
        return Err(invalid("this file is too large to be a key file"));
    }
    let bytes = std::fs::read(path).map_err(|_| unreadable())?;
    let private_key = String::from_utf8(bytes)
        .map_err(|_| invalid("this is not a text key file (OpenSSH, PEM or PuTTY format)"))?;
    if !private_key.contains("PRIVATE KEY") && !private_key.contains("PuTTY-User-Key-File") {
        return Err(invalid(
            "this file holds no private key; choose the file without the .pub ending",
        ));
    }
    let label = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(KeyImport {
        label,
        private_key,
        public_key: sibling(path, ".pub"),
        certificate: sibling(path, "-cert.pub"),
    })
}

#[cfg(test)]
mod tests;
