//! Application state shared by commands, plus error mapping to the
//! structured `CoreError` the UI understands (task CORE-08).

use crate::logging;
use senvakara_adapters_ssh::{
    connect_any, Auth, Candidate, ConnectParams, HostKeyPolicy, Secret, SshConnection,
};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_secrets::{redact, SecretStore};
use senvakara_storage::{Db, HostRecord, KeyKind, SshAuth, SshEndpoint, StorageError};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub struct AppState {
    pub db: Mutex<Db>,
    pub secrets: Option<SecretStore>,
    /// One live SSH connection per host, shared by terminals and SFTP. Held
    /// only for map lookups, never across a connect.
    conns: Mutex<HashMap<String, Arc<SshConnection>>>,
    /// One lock per host, held while that host connects or disconnects, so a
    /// slow or unreachable host never delays the others.
    host_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Hosts the user connected on purpose; only these are reconnected
    /// automatically after a loss (REC-01).
    pub want_connected: Mutex<std::collections::HashSet<String>>,
    /// Hosts with a reconnect loop running → current attempt number.
    pub reconnecting: Mutex<HashMap<String, u32>>,
    /// Which address each live connection went through (NET-03).
    pub paths: Mutex<HashMap<String, ActivePath>>,
    /// Told the host id after every new connection, whatever opened it (a
    /// terminal, Files, a desktop), so the host's phones are watched too.
    on_connect: OnceLock<OnConnect>,
    /// The connection each host was last prepared for (capabilities read,
    /// ADB watched), by address, so it is prepared once per connection.
    prepared: Mutex<HashMap<String, usize>>,
}

type OnConnect = Box<dyn Fn(&str) + Send + Sync>;

/// The network path a live host connection uses.
#[derive(Debug, Clone, Serialize)]
pub struct ActivePath {
    /// `lan` (the host's main SSH address) or `vpn`.
    pub transport: &'static str,
    pub address: String,
    pub port: u16,
    /// Why earlier (preferred) addresses were not used, if any.
    pub note: Option<String>,
}

/// The main SSH address first, then enabled VPN addresses. All of them must
/// present the same pinned host key: an address is a path, not an identity.
pub fn candidates(host: &HostRecord) -> Vec<Candidate> {
    let mut v = Vec::new();
    if let Some(s) = &host.ssh {
        v.push(Candidate {
            transport: "lan",
            address: s.address.clone(),
            port: s.port,
        });
    }
    for e in host.vpn.iter().filter(|e| e.enabled) {
        v.push(Candidate {
            transport: "vpn",
            address: e.address.clone(),
            port: e.port,
        });
    }
    v
}

pub type CmdResult<T> = Result<T, CoreError>;

/// Convert any internal error into a redacted `CoreError` and log it.
pub fn to_core(context: &str, e: anyhow::Error) -> CoreError {
    let ce = if let Some(ce) = e.downcast_ref::<CoreError>() {
        ce.clone()
    } else if let Some(se) = e.downcast_ref::<StorageError>() {
        storage_core(se)
    } else {
        CoreError::internal(redact(&format!("{e:#}")))
    };
    logging::warn(&format!("{context}: {:?}: {}", ce.code, ce.message));
    ce
}

pub fn storage_core(e: &StorageError) -> CoreError {
    match e {
        StorageError::Invalid(m) => CoreError::new(ErrorCode::InvalidInput, m.clone()),
        StorageError::NotFound(m) => {
            CoreError::new(ErrorCode::InvalidInput, format!("{m} does not exist"))
        }
        other => CoreError::internal(redact(&other.to_string())),
    }
}

pub fn db_err(context: &str, e: StorageError) -> CoreError {
    to_core(context, e.into())
}

impl AppState {
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        let db = Db::open(&db_path)?;
        let local_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into());
        db.ensure_local_host(&local_name)?;
        // Device states in the DB are last observations; nothing is live yet.
        db.reset_route_states()?;
        let secrets = match SecretStore::init() {
            Ok(s) => Some(s),
            Err(e) => {
                logging::warn(&format!("credential store unavailable: {e}"));
                None
            }
        };
        Ok(Self {
            db: Mutex::new(db),
            secrets,
            conns: Default::default(),
            host_locks: Default::default(),
            want_connected: Default::default(),
            reconnecting: Default::default(),
            paths: Default::default(),
            on_connect: OnceLock::new(),
            prepared: Default::default(),
        })
    }

    /// Set what runs after every new connection. Only the first call counts.
    pub(crate) fn set_on_connect(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        let _ = self.on_connect.set(Box::new(f));
    }

    /// Claim the preparation of `conn` for `host_id`: true for the first
    /// caller per connection, false when it was already claimed.
    pub(crate) fn claim_prepare(&self, host_id: &str, conn: &Arc<SshConnection>) -> bool {
        claim(
            &mut self.prepared.lock().unwrap(),
            host_id,
            Arc::as_ptr(conn) as usize,
        )
    }

    fn host_lock(&self, host_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.host_locks
            .lock()
            .unwrap()
            .entry(host_id.to_string())
            .or_default()
            .clone()
    }

    /// The host's connection if it is up right now. Never connects.
    pub fn live(&self, host_id: &str) -> Option<Arc<SshConnection>> {
        self.conns
            .lock()
            .unwrap()
            .get(host_id)
            .filter(|c| !c.is_closed())
            .cloned()
    }

    pub fn is_connected(&self, host_id: &str) -> bool {
        self.live(host_id).is_some()
    }

    /// Every pooled connection (open or not), for the heartbeat.
    pub fn connections(&self) -> Vec<(String, Arc<SshConnection>)> {
        self.conns
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// A secret from the OS store by reference; `None` when there is no
    /// reference, no store, or the entry is gone.
    fn secret(&self, reference: Option<&str>, what: &str) -> CmdResult<Option<Secret>> {
        let (Some(reference), Some(store)) = (reference, &self.secrets) else {
            return Ok(None);
        };
        Ok(store
            .get(reference)
            .map_err(|e| to_core(what, e.into()))?
            .map(Secret::new))
    }

    /// Collect what the host's sign-in method needs (CRED-01..04). Secrets
    /// come from the OS store and go nowhere but the SSH adapter.
    fn sign_in(&self, ssh: &SshEndpoint) -> CmdResult<Auth> {
        let missing = |message: &str| CoreError::new(ErrorCode::Unauthenticated, message);
        match &ssh.auth {
            SshAuth::KeyFile { path } => Ok(Auth::KeyFile {
                path: PathBuf::from(path),
                passphrase: self.secret(ssh.passphrase_ref.as_deref(), "passphrase")?,
            }),
            SshAuth::Password => self
                .secret(ssh.password_ref.as_deref(), "password")?
                .map(Auth::Password)
                .ok_or_else(|| missing("no password is saved: save the password for this host")),
            SshAuth::Agent => Ok(Auth::Agent),
            SshAuth::Keychain { key_id } => {
                let key = self.db.lock().unwrap().get_key(key_id).map_err(|_| {
                    missing("the key this host signs in with is no longer in the keychain")
                })?;
                if key.kind == KeyKind::Fido2 {
                    // The security key signs through the agent (ADR-027).
                    return Ok(Auth::Agent);
                }
                let gone = "the private key is missing from the credential store: add the key to the keychain again";
                let store = self.secrets.as_ref().ok_or_else(|| missing(gone))?;
                let private_key = match key.private_ref.as_deref() {
                    Some(reference) => store
                        .get_large(reference)
                        .map_err(|e| to_core("private key", e.into()))?,
                    None => None,
                };
                Ok(Auth::KeyText {
                    private_key: Secret::new(private_key.ok_or_else(|| missing(gone))?),
                    passphrase: self.secret(key.passphrase_ref.as_deref(), "passphrase")?,
                    certificate: key.certificate,
                })
            }
        }
    }

    /// Get a live connection to `host_id`, connecting with the pinned host key.
    /// Concurrent callers for the same host share one connect attempt; other
    /// hosts are not held up.
    pub async fn connection(&self, host_id: &str) -> CmdResult<Arc<SshConnection>> {
        if let Some(c) = self.live(host_id) {
            return Ok(c);
        }
        let lock = self.host_lock(host_id);
        let _connecting = lock.lock().await;
        if let Some(c) = self.live(host_id) {
            return Ok(c);
        }
        self.conns.lock().unwrap().remove(host_id);

        let host = self
            .db
            .lock()
            .unwrap()
            .get_host(host_id)
            .map_err(|e| db_err("connect", e))?;
        let ssh = host.ssh.clone().ok_or_else(|| {
            CoreError::new(ErrorCode::UnsupportedCapability, "host has no SSH endpoint")
        })?;
        let fingerprint = ssh.fingerprint.clone().ok_or_else(|| {
            CoreError::new(
                ErrorCode::Unauthenticated,
                "host key not trusted yet: verify and trust it first",
            )
        })?;
        let auth = self.sign_in(&ssh)?;
        // Route fallback (NET-03): LAN first, then VPN addresses; every
        // address must present the same pinned key (checked before any
        // credential is sent), so a different machine at an old LAN address
        // is refused and the next path is tried.
        let cands = candidates(&host);
        let base = ConnectParams {
            host: String::new(),
            port: 0,
            user: ssh.username.clone(),
            auth,
            host_key: HostKeyPolicy::Pinned(fingerprint),
            connect_timeout: Duration::from_secs(10),
        };
        let done = connect_any(&cands, &base, Duration::from_secs(3))
            .await
            .map_err(|e| to_core("connect", e))?;
        let c = &cands[done.index];
        let note = (!done.notes.is_empty()).then(|| done.notes.join("; "));
        let _ = self.db.lock().unwrap().mark_seen(host_id);
        logging::info(&format!(
            "connected to host {host_id} via {}{}",
            c.transport,
            note.as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default()
        ));
        self.paths.lock().unwrap().insert(
            host_id.to_string(),
            ActivePath {
                transport: c.transport,
                address: c.address.clone(),
                port: c.port,
                note,
            },
        );
        self.conns
            .lock()
            .unwrap()
            .insert(host_id.to_string(), done.conn.clone());
        // A new connection may reuse the address of a dropped one.
        self.prepared.lock().unwrap().remove(host_id);
        if let Some(f) = self.on_connect.get() {
            f(host_id);
        }
        Ok(done.conn)
    }

    pub async fn disconnect(&self, host_id: &str) {
        // Wait for a connect in flight, so it cannot put the connection back.
        let lock = self.host_lock(host_id);
        let _connecting = lock.lock().await;
        self.paths.lock().unwrap().remove(host_id);
        self.prepared.lock().unwrap().remove(host_id);
        let removed = self.conns.lock().unwrap().remove(host_id);
        if let Some(c) = removed {
            let _ = c.close().await;
            logging::info(&format!("disconnected from host {host_id}"));
        }
    }
}

/// True the first time `key` is claimed for `host_id`; a different key (a
/// new connection) claims again.
fn claim(claims: &mut HashMap<String, usize>, host_id: &str, key: usize) -> bool {
    if claims.get(host_id) == Some(&key) {
        return false;
    }
    claims.insert(host_id.to_string(), key);
    true
}

#[cfg(test)]
mod claim_tests {
    use super::claim;
    use std::collections::HashMap;

    #[test]
    fn a_connection_is_prepared_once_and_a_new_one_again() {
        let mut claims = HashMap::new();
        assert!(claim(&mut claims, "fedora", 1));
        assert!(!claim(&mut claims, "fedora", 1));
        assert!(claim(&mut claims, "other", 1));
        assert!(claim(&mut claims, "fedora", 2));
        claims.remove("fedora");
        assert!(claim(&mut claims, "fedora", 2));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use russh::keys::ssh_key::LineEnding;
    use russh::keys::{Algorithm, PrivateKey};
    use senvakara_storage::{HostUpsert, KeyUpsert, Protocol};

    fn state() -> (PathBuf, AppState) {
        let dir = std::env::temp_dir().join(format!("svk-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::open(dir.join("test.db")).unwrap();
        (dir, state)
    }

    fn host(state: &mut AppState, auth: SshAuth) -> SshEndpoint {
        let input = HostUpsert {
            display_name: "Build box".into(),
            os_family: "linux".into(),
            protocol: Protocol::Ssh,
            address: "192.0.2.10".into(),
            port: 22,
            username: "dev".into(),
            auth,
            group_name: None,
            tags: Vec::new(),
            startup_snippet: None,
        };
        let db = state.db.get_mut().unwrap();
        db.create_host(&input).unwrap().ssh.unwrap()
    }

    #[test]
    fn a_password_host_without_a_saved_password_says_so() {
        let (dir, mut state) = state();
        let mut ssh = host(&mut state, SshAuth::Password);
        let error = state.sign_in(&ssh).err().unwrap();
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert!(error.message.contains("save the password"), "{error}");

        let store = state.secrets.as_ref().unwrap();
        let reference = store.put("not-a-real-password").unwrap();
        ssh.password_ref = Some(reference.clone());
        let auth = state.sign_in(&ssh).unwrap();
        store.delete(&reference).unwrap();
        let Auth::Password(password) = &auth else {
            panic!("expected a password sign-in, got {auth:?}");
        };
        assert_eq!(password.as_str(), "not-a-real-password");
        // Even the debug form of the sign-in keeps the password out.
        assert!(!format!("{auth:?}").contains("not-a-real"));

        // The entry is gone from the OS store: same answer as never saved.
        assert!(state.sign_in(&ssh).is_err());
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_keychain_host_signs_in_with_the_stored_key_text_and_certificate() {
        let (dir, mut state) = state();
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let text = key.to_openssh(LineEnding::LF).unwrap().to_string();
        let private_ref = state.secrets.as_ref().unwrap().put_large(&text).unwrap();
        let entry = KeyUpsert {
            label: "deploy".into(),
            algorithm: "ssh-ed25519".into(),
            public_key: key.public_key().to_openssh().unwrap(),
            certificate: Some("ssh-ed25519-cert-v01@openssh.com AAAA".into()),
            kind: KeyKind::Key,
        };
        let db = state.db.get_mut().unwrap();
        let record = db.create_key(&entry, Some(&private_ref)).unwrap();
        let ssh = host(
            &mut state,
            SshAuth::Keychain {
                key_id: record.id.clone(),
            },
        );

        let auth = state.sign_in(&ssh).unwrap();
        let store = state.secrets.as_ref().unwrap();
        store.delete_large(&private_ref).unwrap();
        let Auth::KeyText {
            private_key,
            passphrase,
            certificate,
        } = &auth
        else {
            panic!("expected key text, got {auth:?}");
        };
        assert_eq!(private_key.as_str(), text);
        assert!(passphrase.is_none());
        assert_eq!(certificate, &entry.certificate);

        // With the private key gone from the OS store the user is told.
        let error = state.sign_in(&ssh).err().unwrap();
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert!(error.message.contains("keychain again"), "{error}");
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agent_and_fido2_hosts_sign_in_through_the_agent() {
        let (dir, mut state) = state();
        let ssh = host(&mut state, SshAuth::Agent);
        assert!(matches!(state.sign_in(&ssh), Ok(Auth::Agent)));

        let entry = KeyUpsert {
            label: "YubiKey".into(),
            algorithm: "sk-ssh-ed25519@openssh.com".into(),
            public_key: "sk-ssh-ed25519@openssh.com AAAA".into(),
            certificate: None,
            kind: KeyKind::Fido2,
        };
        let key = state
            .db
            .get_mut()
            .unwrap()
            .create_key(&entry, None)
            .unwrap();
        let mut fido = ssh.clone();
        fido.auth = SshAuth::Keychain { key_id: key.id };
        assert!(matches!(state.sign_in(&fido), Ok(Auth::Agent)));
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }
}
