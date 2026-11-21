//! Secret handling (tasks CORE-07, CORE-08).
//!
//! - [`SecretStore`]: secrets live in the OS credential store (Windows
//!   Credential Manager). The database only keeps the opaque reference
//!   returned by [`SecretStore::put`] (ADR-009, docs/SECURITY.md §5).
//! - [`redact`]: scrub secret-looking content before anything is logged.

mod redact;

pub use redact::redact;

use keyring_core::Entry;
use std::sync::Once;

/// Service name under which all Senvakara entries are filed.
const SERVICE: &str = "dev.senvakara.desktop";

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
        Entry::new(SERVICE, &reference)
            .and_then(|e| e.set_password(secret))
            .map_err(store_err)?;
        Ok(reference)
    }

    /// Fetch a secret by reference. `Ok(None)` if it no longer exists.
    pub fn get(&self, reference: &str) -> Result<Option<String>, SecretError> {
        let entry = Entry::new(SERVICE, reference).map_err(store_err)?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    /// Delete a secret. Deleting a missing entry is not an error.
    pub fn delete(&self, reference: &str) -> Result<(), SecretError> {
        let entry = Entry::new(SERVICE, reference).map_err(store_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
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
}
