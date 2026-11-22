//! Host registry commands (INV-01..04, tasks CORE-03, CORE-05, CORE-07).

mod credentials;

pub(crate) use credentials::{host_set_password, host_set_rdp_password};

use crate::logging;
use crate::state::{candidates, db_err, to_core, ActivePath, AppState, CmdResult};
use crate::App;
use senvakara_adapters_ssh::{first_reachable, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::{HostRecord, HostUpsert, Protocol, SshAuth, VpnEndpoint};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// What the UI may see about a host. Secrets and secret references are
/// reduced to booleans.
#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    pub id: String,
    pub display_name: String,
    pub os_family: String,
    pub is_local: bool,
    pub trust_state: String,
    pub last_seen_at: Option<String>,
    pub protocol: Protocol,
    pub group_name: Option<String>,
    pub tags: Vec<String>,
    pub capabilities: serde_json::Value,
    pub connected: bool,
    /// Attempt number while an automatic reconnect is in progress.
    pub reconnecting: Option<u32>,
    pub ssh: Option<SshView>,
    pub rdp: Option<RdpView>,
    /// VPN addresses of the same host (NET-03).
    pub vpn: Vec<VpnEndpoint>,
    /// Address the live connection went through; `None` when not connected.
    pub path: Option<ActivePath>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SshView {
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    /// The key file of a `key_file` host; empty for other sign-in methods.
    pub private_key_path: String,
    pub fingerprint: Option<String>,
    pub has_passphrase: bool,
    pub has_password: bool,
    pub startup_snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RdpView {
    pub address: String,
    pub port: u16,
    pub username: String,
    pub has_credential: bool,
}

fn view(h: HostRecord, connected: bool) -> HostView {
    HostView {
        capabilities: serde_json::from_str(&h.capabilities_json).unwrap_or_default(),
        ssh: h.ssh.map(|s| SshView {
            address: s.address,
            port: s.port,
            username: s.username,
            private_key_path: match &s.auth {
                SshAuth::KeyFile { path } => path.clone(),
                _ => String::new(),
            },
            auth: s.auth,
            fingerprint: s.fingerprint,
            has_passphrase: s.passphrase_ref.is_some(),
            has_password: s.password_ref.is_some(),
            startup_snippet: s.startup_snippet,
        }),
        rdp: h.rdp.map(|r| RdpView {
            address: r.address,
            port: r.port,
            username: r.username,
            has_credential: r.credential_target.is_some(),
        }),
        protocol: h.protocol,
        group_name: h.group_name,
        tags: h.tags,
        vpn: h.vpn,
        id: h.id,
        display_name: h.display_name,
        os_family: h.os_family,
        is_local: h.is_local,
        trust_state: h.trust_state,
        last_seen_at: h.last_seen_at,
        connected,
        reconnecting: None,
        path: None,
    }
}

fn with_runtime(state: &AppState, mut v: HostView, reconnecting: Option<u32>) -> HostView {
    v.reconnecting = reconnecting;
    if v.connected {
        v.path = state.paths.lock().unwrap().get(&v.id).cloned();
    }
    v
}

fn host_view(state: &AppState, id: &str) -> CmdResult<HostView> {
    let rec = state
        .db
        .lock()
        .unwrap()
        .get_host(id)
        .map_err(|e| db_err("get host", e))?;
    let connected = state.is_connected(id);
    let attempt = state.reconnecting.lock().unwrap().get(id).copied();
    Ok(with_runtime(state, view(rec, connected), attempt))
}

pub(crate) fn hosts_list(app: &App) -> CmdResult<Vec<HostView>> {
    let state = &app.state;
    let recs = state
        .db
        .lock()
        .unwrap()
        .list_hosts()
        .map_err(|e| db_err("list hosts", e))?;
    let reconnecting = state.reconnecting.lock().unwrap().clone();
    Ok(recs
        .into_iter()
        .map(|h| {
            let connected = state.is_connected(&h.id);
            let attempt = reconnecting.get(&h.id).copied();
            with_runtime(state, view(h, connected), attempt)
        })
        .collect())
}

/// Register a VPN address for a host (NET-IMPL-01). Nothing is trusted by
/// this: connections through it must still present the pinned host key.
pub(crate) fn host_add_vpn(
    app: &App,
    id: String,
    address: String,
    port: u16,
) -> CmdResult<HostView> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .add_vpn_endpoint(&id, &address, port)
        .map_err(|e| db_err("add VPN address", e))?;
    logging::info(&format!("VPN address added for host {id}"));
    host_view(state, &id)
}

pub(crate) fn host_remove_vpn(app: &App, id: String, endpoint_id: String) -> CmdResult<HostView> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .remove_vpn_endpoint(&id, &endpoint_id)
        .map_err(|e| db_err("remove VPN address", e))?;
    logging::info(&format!("VPN address removed for host {id}"));
    host_view(state, &id)
}

pub(crate) fn host_create(app: &App, input: HostUpsert) -> CmdResult<HostView> {
    let state = &app.state;
    let rec = state
        .db
        .lock()
        .unwrap()
        .create_host(&input)
        .map_err(|e| db_err("create host", e))?;
    logging::info(&format!("host created {}", rec.id));
    Ok(view(rec, false))
}

pub(crate) async fn host_update(
    app: &Arc<App>,
    id: String,
    input: HostUpsert,
) -> CmdResult<HostView> {
    let state = &app.state;
    let (before, rec) = {
        let mut db = state.db.lock().unwrap();
        let before = db.get_host(&id).map_err(|e| db_err("update host", e))?;
        let rec = db
            .update_host(&id, &input)
            .map_err(|e| db_err("update host", e))?;
        (before, rec)
    };
    // Settings may have changed (address, user, key): force a fresh connection.
    crate::android::stop_host(app, &id);
    state.disconnect(&id).await;
    credentials::forget_moved_rdp_credential(state, &id, before.rdp.as_ref(), rec.rdp.as_ref())?;
    host_view(state, &id)
}

pub(crate) async fn host_delete(app: &Arc<App>, id: String) -> CmdResult<()> {
    let state = &app.state;
    state.want_connected.lock().unwrap().remove(&id);
    crate::android::stop_host(app, &id);
    state.disconnect(&id).await;
    let rec = {
        let db = state.db.lock().unwrap();
        let rec = db.get_host(&id).map_err(|e| db_err("delete host", e))?;
        db.delete_host(&id).map_err(|e| db_err("delete host", e))?;
        rec
    };
    // The host is gone from the database; its secrets go from the OS store.
    if let Some(store) = &state.secrets {
        let ssh = rec.ssh.as_ref();
        for r in ssh
            .iter()
            .flat_map(|s| [&s.passphrase_ref, &s.password_ref])
            .flatten()
        {
            store
                .delete(r)
                .map_err(|e| to_core("delete host secret", e.into()))?;
        }
        if let Some(target) = rec.rdp.and_then(|r| r.credential_target) {
            credentials::drop_rdp_credential(state, store, &id, &target)?;
        }
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
    let ssh = rec.ssh.clone().ok_or_else(|| {
        CoreError::new(ErrorCode::UnsupportedCapability, "host has no SSH endpoint")
    })?;
    // Ask through the first address that answers (LAN first, then VPN), so
    // the key can be checked from another network too.
    let cands = candidates(&rec);
    let all: Vec<usize> = (0..cands.len()).collect();
    let (best, _) = first_reachable(&cands, &all, Duration::from_secs(3)).await;
    let c = &cands[best.unwrap_or(0)];
    let presented = SshConnection::fetch_host_key(&c.address, c.port, Duration::from_secs(8))
        .await
        .map_err(|e| to_core("fetch host key", e))?;
    Ok((presented, ssh.fingerprint))
}

/// Read the host key the server presents, without authenticating.
pub(crate) async fn host_check_key(app: &Arc<App>, id: String) -> CmdResult<HostKeyCheck> {
    let state = &app.state;
    let (presented, pinned) = presented_key(state, &id).await?;
    let matches = pinned.as_deref() == Some(presented.as_str());
    Ok(HostKeyCheck {
        presented,
        pinned,
        matches,
    })
}

/// Pin the fingerprint the user approved. The server is asked again and must
/// still present exactly that key, so a key swapped after display is refused.
pub(crate) async fn host_trust_key(
    app: &Arc<App>,
    id: String,
    fingerprint: String,
) -> CmdResult<HostView> {
    let state = &app.state;
    let (presented, _) = presented_key(state, &id).await?;
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
    host_view(state, &id)
}

/// Save (or clear) the private-key passphrase in Windows Credential Manager.
pub(crate) async fn host_set_passphrase(
    app: &Arc<App>,
    id: String,
    passphrase: Option<String>,
) -> CmdResult<HostView> {
    let state = &app.state;
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
    host_view(state, &id)
}

/// Connect (or reuse the connection) and refresh detected capabilities.
pub(crate) async fn host_connect(app: &Arc<App>, id: String) -> CmdResult<HostView> {
    let state = &app.state;
    state.want_connected.lock().unwrap().insert(id.clone());
    connect_and_watch(app, state, &id).await?;
    host_view(state, &id)
}

/// Connect with the pinned host key, refresh capabilities and start watching
/// the host's Android devices. Shared by Connect and automatic reconnect.
pub(crate) async fn connect_and_watch(app: &Arc<App>, state: &AppState, id: &str) -> CmdResult<()> {
    let id = id.to_string();
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
        "tmux": conn.exec(&["sh", "-c", "command -v tmux"]).await.map(|o| o.success()).unwrap_or(false),
        // Hints for route registration only; nothing is trusted from them.
        "vpn_interfaces": conn
            .exec(&["sh", "-c", "ip -o -4 addr show 2>/dev/null"])
            .await
            .map(|o| vpn_interfaces(&o.stdout_str()))
            .unwrap_or_default(),
    });
    state
        .db
        .lock()
        .unwrap()
        .set_capabilities(&id, &caps.to_string())
        .map_err(|e| db_err("capabilities", e))?;
    // Android devices attached to this host become visible while connected.
    if let Some(path) = adb.as_deref() {
        crate::android::start_remote(app, &id, conn.clone(), path).await;
    }
    Ok(())
}

pub(crate) async fn host_disconnect(app: &Arc<App>, id: String) -> CmdResult<HostView> {
    let state = &app.state;
    // An explicit disconnect also cancels any automatic reconnect.
    state.want_connected.lock().unwrap().remove(&id);
    crate::android::stop_host(app, &id);
    state.disconnect(&id).await;
    host_view(state, &id)
}

/// VPN-looking interfaces from `ip -o -4 addr show`: WireGuard (`wg*`),
/// Tailscale, NetBird (`wt*`), ZeroTier (`zt*`), Nebula, OpenVPN (`tun*`),
/// plus any address in the 100.64.0.0/10 range Tailscale uses.
pub fn vpn_interfaces(ip_output: &str) -> Vec<serde_json::Value> {
    const PREFIXES: &[&str] = &["wg", "tailscale", "wt", "zt", "nebula", "tun"];
    let mut out = Vec::new();
    for line in ip_output.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(name), Some(pos)) = (f.get(1), f.iter().position(|x| *x == "inet")) else {
            continue;
        };
        let Some(addr) = f.get(pos + 1).and_then(|a| a.split('/').next()) else {
            continue;
        };
        let Ok(ip) = addr.parse::<std::net::Ipv4Addr>() else {
            continue;
        };
        let o = ip.octets();
        let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
        if PREFIXES.iter().any(|p| name.starts_with(p)) || cgnat {
            out.push(serde_json::json!({ "name": name, "address": addr }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::vpn_interfaces;

    #[test]
    fn finds_vpn_interfaces_only() {
        let sample = r"1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft forever preferred_lft forever
3: wlo1    inet 192.0.2.10/24 brd 192.0.2.255 scope global dynamic noprefixroute wlo1\       valid_lft 80000sec
5: wg0    inet 10.8.0.3/32 scope global wg0\       valid_lft forever preferred_lft forever
6: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\       valid_lft forever
7: tailscale0    inet 100.101.102.103/32 scope global tailscale0\       valid_lft forever
8: eth9    inet 100.64.1.2/10 scope global eth9\       valid_lft forever
garbage line";
        let v = vpn_interfaces(sample);
        let names: Vec<&str> = v.iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["wg0", "tailscale0", "eth9"]);
        assert_eq!(v[0]["address"], "10.8.0.3");
        assert!(vpn_interfaces("").is_empty());
    }
}
