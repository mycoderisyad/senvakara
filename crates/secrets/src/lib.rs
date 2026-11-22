//! Secret handling (tasks CORE-07, CORE-08, CRED-01, CRED-02, DESK-IMPL-04).
//!
//! - [`SecretStore`]: secrets live in the OS credential store (Windows
//!   Credential Manager). The database only keeps the opaque reference
//!   returned by [`SecretStore::put`] (ADR-009, docs/SECURITY.md §5).
//! - Secrets larger than one credential blob (an RSA private key, say) are
//!   split over several entries by [`SecretStore::put_large`] (ADR-025).
//! - Remote Desktop credentials are written under `TERMSRV/<address>`, where
//!   `mstsc.exe` looks for them (ADR-026).
//! - [`redact`]: scrub secret-looking content before anything is logged.

mod chunks;
mod redact;

pub use redact::redact;

use chunks::{manifest, parse_manifest, CHUNK_BYTES};
use keyring_core::Entry;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, Once, PoisonError};

/// Service name under which all Senvakara entries are filed.
const SERVICE: &str = "dev.senvakara.desktop";

/// Target prefix `mstsc.exe` reads saved credentials from.
const RDP_PREFIX: &str = "TERMSRV/";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential store unavailable: {0}")]
    Unavailable(String),
    #[error("credential store error: {0}")]
    Store(String),
}

fn store_err(e: keyring_core::Error) -> SecretError {
    // keyring errors never contain the secret value itself.
    SecretError::Store(e.to_string())
}

static INIT: Once = Once::new();

/// Credentials written from several threads at once can go missing in
/// Windows Credential Manager (seen with eight writers), so every operation
/// on the store runs under this lock, a split secret as one unit.
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> MutexGuard<'static, ()> {
    // The lock guards no data of its own: a panic elsewhere changes nothing.
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct SecretStore;

impl SecretStore {
    /// Select the platform credential store. Safe to call more than once.
    pub fn init() -> Result<Self, SecretError> {
        #[cfg(windows)]
        {
            let mut result = Ok(());
            INIT.call_once(|| match windows_native_keyring_store::Store::new() {
                Ok(store) => keyring_core::set_default_store(store),
                Err(e) => result = Err(SecretError::Unavailable(e.to_string())),
            });
            result.map(|_| SecretStore)
        }
        #[cfg(not(windows))]
        {
            let _ = &INIT;
            Err(SecretError::Unavailable(
                "only the Windows credential store is wired up".into(),
            ))
        }
    }

    /// Store a new secret and return its opaque reference.
    pub fn put(&self, secret: &str) -> Result<String, SecretError> {
        let reference = format!("secret-{}", uuid::Uuid::new_v4());
        let _gate = gate();
        Entry::new(SERVICE, &reference)
            .and_then(|e| e.set_password(secret))
            .map_err(store_err)?;
        Ok(reference)
    }

    /// Fetch a secret by reference. `Ok(None)` if it no longer exists.
    pub fn get(&self, reference: &str) -> Result<Option<String>, SecretError> {
        let _gate = gate();
        self.read(reference)
    }

    fn read(&self, reference: &str) -> Result<Option<String>, SecretError> {
        let entry = Entry::new(SERVICE, reference).map_err(store_err)?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    /// Delete a secret. Deleting a missing entry is not an error.
    pub fn delete(&self, reference: &str) -> Result<(), SecretError> {
        let _gate = gate();
        self.remove(reference)
    }

    fn remove(&self, reference: &str) -> Result<(), SecretError> {
        delete_entry(&Entry::new(SERVICE, reference).map_err(store_err)?)
    }

    /// Store a secret of any size and return its opaque reference. The text
    /// is split into pieces small enough for one credential each, listed by
    /// a manifest entry under the reference.
    pub fn put_large(&self, secret: &str) -> Result<String, SecretError> {
        let reference = format!("secret-{}", uuid::Uuid::new_v4());
        let _gate = gate();
        let pieces: Vec<&[u8]> = secret.as_bytes().chunks(CHUNK_BYTES).collect();
        for (index, piece) in pieces.iter().enumerate() {
            let written = Entry::new(SERVICE, &chunk_name(&reference, index))
                .and_then(|e| e.set_secret(piece));
            if let Err(error) = written {
                let _ = self.delete_pieces(&reference, index);
                return Err(store_err(error));
            }
        }
        let written =
            Entry::new(SERVICE, &reference).and_then(|e| e.set_password(&manifest(pieces.len())));
        if let Err(error) = written {
            let _ = self.delete_pieces(&reference, pieces.len());
            return Err(store_err(error));
        }
        Ok(reference)
    }

    /// Fetch a secret stored by [`Self::put_large`]. A plain secret stored by
    /// [`Self::put`] is returned as it is. `Ok(None)` if it no longer exists.
    pub fn get_large(&self, reference: &str) -> Result<Option<String>, SecretError> {
        let _gate = gate();
        let Some(head) = self.read(reference)? else {
            return Ok(None);
        };
        let Some(count) = parse_manifest(&head) else {
            return Ok(Some(head));
        };
        let mut bytes = Vec::new();
        for index in 0..count {
            let entry = Entry::new(SERVICE, &chunk_name(reference, index)).map_err(store_err)?;
            match entry.get_secret() {
                Ok(piece) => bytes.extend_from_slice(&piece),
                Err(keyring_core::Error::NoEntry) => {
                    return Err(SecretError::Store(
                        "part of the stored secret is missing".into(),
                    ))
                }
                Err(e) => return Err(store_err(e)),
            }
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| SecretError::Store("the stored secret is damaged".into()))
    }

    /// Delete a secret stored by [`Self::put_large`] (or [`Self::put`]).
    /// Deleting a missing entry is not an error.
    pub fn delete_large(&self, reference: &str) -> Result<(), SecretError> {
        let _gate = gate();
        if let Some(count) = self.read(reference)?.as_deref().and_then(parse_manifest) {
            self.delete_pieces(reference, count)?;
        }
        self.remove(reference)
    }

    fn delete_pieces(&self, reference: &str, count: usize) -> Result<(), SecretError> {
        for index in 0..count {
            let entry = Entry::new(SERVICE, &chunk_name(reference, index)).map_err(store_err)?;
            delete_entry(&entry)?;
        }
        Ok(())
    }

    /// Save the user name and password `mstsc.exe` uses for `address`, under
    /// the `TERMSRV/<address>` target, and return that target name. An
    /// existing entry for the address is replaced.
    pub fn put_rdp_credential(
        &self,
        address: &str,
        user: &str,
        password: &str,
    ) -> Result<String, SecretError> {
        let target = rdp_target(address);
        let _gate = gate();
        let entry = rdp_entry(&target)?;
        entry.set_password(password).map_err(store_err)?;
        let attributes = HashMap::from([("username", user)]);
        if let Err(error) = entry.update_attributes(&attributes) {
            let _ = delete_entry(&entry);
            return Err(store_err(error));
        }
        Ok(target)
    }

    /// Remove a `TERMSRV/...` entry written by [`Self::put_rdp_credential`].
    /// Deleting a missing entry is not an error; any other target is refused.
    pub fn delete_rdp_credential(&self, target: &str) -> Result<(), SecretError> {
        if !target.starts_with(RDP_PREFIX) {
            return Err(SecretError::Store("not a remote desktop credential".into()));
        }
        let _gate = gate();
        delete_entry(&rdp_entry(target)?)
    }
}

/// The credential target `mstsc.exe` uses for `address`.
pub fn rdp_target(address: &str) -> String {
    format!("{RDP_PREFIX}{}", address.trim())
}

fn rdp_entry(target: &str) -> Result<Entry, SecretError> {
    let modifiers = HashMap::from([("target", target)]);
    Entry::new_with_modifiers(SERVICE, target, &modifiers).map_err(store_err)
}

fn chunk_name(reference: &str, index: usize) -> String {
    format!("{reference}.{index}")
}

fn delete_entry(entry: &Entry) -> Result<(), SecretError> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(store_err(e)),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Round trip against the real Windows Credential Manager; cleans up.
    #[test]
    fn windows_credential_manager_round_trip() {
        let store = SecretStore::init().unwrap();
        let r = store.put("test-value-not-a-real-secret").unwrap();
        assert!(r.starts_with("secret-"));
        assert_eq!(
            store.get(&r).unwrap().as_deref(),
            Some("test-value-not-a-real-secret")
        );
        store.delete(&r).unwrap();
        assert_eq!(store.get(&r).unwrap(), None);
        store.delete(&r).unwrap();
    }

    /// A secret bigger than one credential blob survives a round trip.
    #[test]
    fn a_large_secret_is_split_and_joined_again() {
        let store = SecretStore::init().unwrap();
        let text: String = (0..7000u32)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let r = store.put_large(&text).unwrap();
        assert_eq!(store.get_large(&r).unwrap().as_deref(), Some(text.as_str()));
        store.delete_large(&r).unwrap();
        assert_eq!(store.get_large(&r).unwrap(), None);
        assert_eq!(store.get(&chunk_name(&r, 0)).unwrap(), None);
        store.delete_large(&r).unwrap();
    }

    /// Several threads at once (a key being saved while a host connects):
    /// no secret may go missing.
    #[test]
    fn secrets_written_from_several_threads_at_once_all_read_back() {
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                std::thread::spawn(move || {
                    let store = SecretStore::init().unwrap();
                    for round in 0..3 {
                        let text = format!("worker-{worker}-round-{round}-").repeat(150);
                        let r = store.put_large(&text).unwrap();
                        let read = store.get_large(&r);
                        store.delete_large(&r).unwrap();
                        assert_eq!(read.unwrap().as_deref(), Some(text.as_str()));
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn get_large_reads_a_plain_secret_too() {
        let store = SecretStore::init().unwrap();
        let r = store.put("plain-test-value").unwrap();
        assert_eq!(
            store.get_large(&r).unwrap().as_deref(),
            Some("plain-test-value")
        );
        store.delete_large(&r).unwrap();
        assert_eq!(store.get(&r).unwrap(), None);
    }

    #[test]
    fn a_remote_desktop_credential_is_written_under_termsrv_and_removed() {
        let store = SecretStore::init().unwrap();
        let address = format!("senvakara-test-{}.invalid", uuid::Uuid::new_v4());
        let target = store
            .put_rdp_credential(&address, "tester", "not-a-real-password")
            .unwrap();
        assert_eq!(target, format!("TERMSRV/{address}"));
        let entry = rdp_entry(&target).unwrap();
        assert_eq!(entry.get_password().unwrap(), "not-a-real-password");
        assert_eq!(entry.get_attributes().unwrap()["username"], "tester");
        store.delete_rdp_credential(&target).unwrap();
        assert!(matches!(
            entry.get_password(),
            Err(keyring_core::Error::NoEntry)
        ));
        assert!(store.delete_rdp_credential("secret-x").is_err());
    }
}
