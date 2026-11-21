//! Host registry commands (INV-01..04, tasks CORE-03, CORE-05, CORE-07).

use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
use senvakara_adapters_ssh::SshConnection;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::{HostRecord, HostUpsert};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, State};

/// What the UI may see about a host. Secrets and secret references are
/// reduced to booleans.
#[derive(Debug, Serialize)]
pub struct HostView {
    pub id: String,
    pub display_name: String,
    pub os_family: String,
    pub is_local: bool,
    pub trust_state: String,
    pub last_seen_at: Option<String>,
    pub capabilities: serde_json::Value,
    pub connected: bool,
    pub ssh: Option<SshView>,
}

#[derive(Debug, Serialize)]
pub struct SshView {
    pub address: String,
    pub port: u16,
    pub username: String,
    pub private_key_path: String,
    pub fingerprint: Option<String>,
    pub has_passphrase: bool,
}

fn view(h: HostRecord, connected: bool) -> HostView {
    HostView {
        capabilities: serde_json::from_str(&h.capabilities_json).unwrap_or_default(),
        ssh: h.ssh.map(|s| SshView {
            address: s.address,
            port: s.port,
            username: s.username,
            private_key_path: s.private_key_path,
            fingerprint: s.fingerprint,
            has_passphrase: s.passphrase_ref.is_some(),
        }),
        id: h.id,
        display_name: h.display_name,
        os_family: h.os_family,
        is_local: h.is_local,
        trust_state: h.trust_state,
        last_seen_at: h.last_seen_at,
        connected,
    }
}

async fn host_view(state: &AppState, id: &str) -> CmdResult<HostView> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .get_host(id)
        .map_err(|e| db_err("get host", e))?;
    let connected = state
        .conns
        .lock()
        .await
        .get(id)
        .is_some_and(|c| !c.is_closed());
    Ok(view(rec, connected))
}

#[tauri::command]
pub async fn hosts_list(state: State<'_, AppState>) -> CmdResult<Vec<HostView>> {
    let recs = state
        .db
        .lock()
        .unwrap()
        .list_hosts()
        .map_err(|e| db_err("list hosts", e))?;
    let conns = state.conns.lock().await;
    Ok(recs
        .into_iter()
        .map(|h| {
            let connected = conns.get(&h.id).is_some_and(|c| !c.is_closed());
            view(h, connected)
        })
        .collect())
}

#[tauri::command]
pub async fn host_create(state: State<'_, AppState>, input: HostUpsert) -> CmdResult<HostView> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .create_host(&input)
        .map_err(|e| db_err("create host", e))?;
    logging::info(&format!("host created {}", rec.id));
    Ok(view(rec, false))
}

#[tauri::command]
pub async fn host_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    input: HostUpsert,
) -> CmdResult<HostView> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .update_host(&id, &input)
        .map_err(|e| db_err("update host", e))?;
    // Settings may have changed (address, user, key): force a fresh connection.
    crate::android::stop_host(&app, &id);
    state.disconnect(&id).await;
    Ok(view(rec, false))
}

#[tauri::command]
pub async fn host_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    crate::android::stop_host(&app, &id);
    state.disconnect(&id).await;
    let passphrase_ref = {
        let db = state.db.lock().unwrap();
        let rec = db.get_host(&id).map_err(|e| db_err("delete host", e))?;
        db.delete_host(&id).map_err(|e| db_err("delete host", e))?;
        rec.ssh.and_then(|s| s.passphrase_ref)
    };
    if let (Some(r), Some(store)) = (passphrase_ref, &state.secrets) {
        store
            .delete(&r)
            .map_err(|e| to_core("delete passphrase", e.into()))?;
    }
    logging::info(&format!("host deleted {id}"));
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct HostKeyCheck {
    /// Fingerprint the server presents right now.
    pub presented: String,
    /// Fingerprint currently trusted for this host, if any.
    pub pinned: Option<String>,
    pub matches: bool,
}

async fn presented_key(state: &AppState, id: &str) -> CmdResult<(String, Option<String>)> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .get_host(id)
        .map_err(|e| db_err("host key", e))?;
    let ssh = rec.ssh.ok_or_else(|| {
        CoreError::new(ErrorCode::UnsupportedCapability, "host has no SSH endpoint")
    })?;
    let presented = SshConnection::fetch_host_key(&ssh.address, ssh.port, Duration::from_secs(8))
        .await
        .map_err(|e| to_core("fetch host key", e))?;
    Ok((presented, ssh.fingerprint))
}

/// Read the host key the server presents, without authenticating.
#[tauri::command]
pub async fn host_check_key(state: State<'_, AppState>, id: String) -> CmdResult<HostKeyCheck> {
    let (presented, pinned) = presented_key(&state, &id).await?;
    let matches = pinned.as_deref() == Some(presented.as_str());
    Ok(HostKeyCheck {
        presented,
        pinned,
        matches,
    })
}

/// Pin the fingerprint the user approved. The server is asked again and must
/// still present exactly that key, so a key swapped after display is refused.
#[tauri::command]
pub async fn host_trust_key(
    state: State<'_, AppState>,
    id: String,
    fingerprint: String,
) -> CmdResult<HostView> {
    let (presented, _) = presented_key(&state, &id).await?;
    if presented != fingerprint {
        return Err(to_core(
            "trust key",
            CoreError::new(
                ErrorCode::Conflict,
                "the server now presents a different key; check again",
            )
            .into(),
        ));
    }
    state
        .db
        .lock()
        .unwrap()
        .trust_host_key(&id, &fingerprint)
        .map_err(|e| db_err("trust key", e))?;
    state.disconnect(&id).await;
    logging::info(&format!("host key trusted for {id}: {fingerprint}"));
    host_view(&state, &id).await
}

/// Save (or clear) the private-key passphrase in Windows Credential Manager.
#[tauri::command]
pub async fn host_set_passphrase(
    state: State<'_, AppState>,
    id: String,
    passphrase: Option<String>,
) -> CmdResult<HostView> {
    let store = state.secrets.as_ref().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "credential store unavailable",
        )
    })?;
    let old = state
        .db
        .lock()
        .unwrap()
        .get_host(&id)
        .map_err(|e| db_err("passphrase", e))?
        .ssh
        .and_then(|s| s.passphrase_ref);
    let new_ref = match passphrase.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => Some(
            store
                .put(p)
                .map_err(|e| to_core("save passphrase", e.into()))?,
        ),
        None => None,
    };
    state
        .db
        .lock()
        .unwrap()
        .set_passphrase_ref(&id, new_ref.as_deref())
        .map_err(|e| db_err("passphrase", e))?;
    if let Some(r) = old {
        let _ = store.delete(&r);
    }
    state.disconnect(&id).await;
    host_view(&state, &id).await
}

/// Connect (or reuse the connection) and refresh detected capabilities.
#[tauri::command]
pub async fn host_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<HostView> {
    let conn = state.connection(&id).await?;
    let os = conn
        .exec(&["cat", "/etc/os-release"])
        .await
        .map(|o| o.stdout_str())
        .unwrap_or_default();
    let field = |k: &str| {
        os.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim_matches('"').to_string())
    };
    let arch = conn
        .exec(&["uname", "-m"])
        .await
        .map(|o| o.stdout_str().trim().to_string())
        .unwrap_or_default();
    let adb = conn
        .exec(&["sh", "-c", "command -v adb"])
        .await
        .map(|o| o.success().then(|| o.stdout_str().trim().to_string()))
        .unwrap_or(None);
    let caps = serde_json::json!({
        "os_pretty_name": field("PRETTY_NAME"),
        "os_id": field("ID"),
        "arch": arch,
        "adb_path": adb,
        "terminal": true,
        "files": true,
        "desktop": crate::desktop::detect(&conn).await.ok(),
    });
    state
        .db
        .lock()
        .unwrap()
        .set_capabilities(&id, &caps.to_string())
        .map_err(|e| db_err("capabilities", e))?;
    // Android devices attached to this host become visible while connected.
    if let Some(path) = adb.as_deref() {
        crate::android::start_remote(&app, &id, conn.clone(), path).await;
    }
    host_view(&state, &id).await
}

#[tauri::command]
pub async fn host_disconnect(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<HostView> {
    crate::android::stop_host(&app, &id);
    state.disconnect(&id).await;
    host_view(&state, &id).await
}
