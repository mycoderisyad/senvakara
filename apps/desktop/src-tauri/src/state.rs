//! Application state shared by commands, plus error mapping to the
//! structured `CoreError` the UI understands (task CORE-08).

use crate::logging;
use senvakara_adapters_ssh::{ConnectParams, HostKeyPolicy, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_secrets::{redact, SecretStore};
use senvakara_storage::{Db, StorageError};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AppState {
    pub db: Mutex<Db>,
    pub secrets: Option<SecretStore>,
    /// One live SSH connection per host, shared by terminals and SFTP.
    pub conns: tokio::sync::Mutex<HashMap<String, Arc<SshConnection>>>,
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
        })
    }

    /// Get a live connection to `host_id`, connecting with the pinned host key.
    pub async fn connection(&self, host_id: &str) -> CmdResult<Arc<SshConnection>> {
        let mut conns = self.conns.lock().await;
        if let Some(c) = conns.get(host_id) {
            if !c.is_closed() {
                return Ok(c.clone());
            }
            conns.remove(host_id);
        }

        let host = self
            .db
            .lock()
            .unwrap()
            .get_host(host_id)
            .map_err(|e| db_err("connect", e))?;
        let ssh = host.ssh.ok_or_else(|| {
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
        let params = ConnectParams {
            host: ssh.address.clone(),
            port: ssh.port,
            user: ssh.username.clone(),
            private_key: PathBuf::from(&ssh.private_key_path),
            host_key: HostKeyPolicy::Pinned(fingerprint),
            connect_timeout: Duration::from_secs(10),
            passphrase,
        };
        let conn = SshConnection::connect(&params)
            .await
            .map_err(|e| to_core("connect", e))?;
        let _ = self.db.lock().unwrap().mark_seen(host_id);
        logging::info(&format!("connected to host {host_id}"));
        conns.insert(host_id.to_string(), conn.clone());
        Ok(conn)
    }

    pub async fn disconnect(&self, host_id: &str) {
        if let Some(c) = self.conns.lock().await.remove(host_id) {
            let _ = c.close().await;
            logging::info(&format!("disconnected from host {host_id}"));
        }
    }
}
