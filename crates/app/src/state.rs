//! Application state shared by commands, plus error mapping to the
//! structured `CoreError` the UI understands (task CORE-08).

use crate::logging;
use senvakara_adapters_ssh::{connect_any, Candidate, ConnectParams, HostKeyPolicy, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_secrets::{redact, SecretStore};
use senvakara_storage::{Db, HostRecord, StorageError};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
}

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
        })
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
        let passphrase = match (&ssh.passphrase_ref, &self.secrets) {
            (Some(r), Some(store)) => store.get(r).map_err(|e| to_core("passphrase", e.into()))?,
            _ => None,
        };
        // Route fallback (NET-03): LAN first, then VPN addresses; every
        // address must present the same pinned key (checked before any
        // credential is sent), so a different machine at an old LAN address
        // is refused and the next path is tried.
        let cands = candidates(&host);
        let base = ConnectParams {
            host: String::new(),
            port: 0,
            user: ssh.username.clone(),
            private_key: PathBuf::from(&ssh.private_key_path),
            host_key: HostKeyPolicy::Pinned(fingerprint),
            connect_timeout: Duration::from_secs(10),
            passphrase,
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
        Ok(done.conn)
    }

    pub async fn disconnect(&self, host_id: &str) {
        // Wait for a connect in flight, so it cannot put the connection back.
        let lock = self.host_lock(host_id);
        let _connecting = lock.lock().await;
        self.paths.lock().unwrap().remove(host_id);
        let removed = self.conns.lock().unwrap().remove(host_id);
        if let Some(c) = removed {
            let _ = c.close().await;
            logging::info(&format!("disconnected from host {host_id}"));
        }
    }
}
