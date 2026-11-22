//! Host registry (INV-01, INV-04). A host's id never derives from its
//! address; changing the address keeps the identity but clears the pinned
//! host key, because a different machine may answer at the new address.
//!
//! A host is opened over SSH or, since schema v5, with Windows Remote Desktop
//! (ADR-026). Secrets are referenced from the OS store, never stored here.

mod upsert;

#[cfg(test)]
mod tests;

pub use upsert::{
    HostUpsert, Protocol, SshAuth, MAX_GROUP_LEN, MAX_SNIPPET_LEN, MAX_TAGS, MAX_TAG_LEN,
};

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use upsert::valid_address;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshEndpoint {
    pub endpoint_id: String,
    pub address: String,
    pub port: u16,
    /// Pinned host-key fingerprint. `None` until the user approves one.
    pub fingerprint: Option<String>,
    pub username: String,
    pub auth: SshAuth,
    /// Opaque credential-store reference for the login password, if any.
    pub password_ref: Option<String>,
    /// Opaque credential-store reference for the key passphrase, if any.
    pub passphrase_ref: Option<String>,
    /// Text typed into the shell once after login.
    pub startup_snippet: Option<String>,
}

/// Where a Remote Desktop host is reached (ADR-026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpEndpoint {
    pub address: String,
    pub port: u16,
    pub username: String,
    /// The `TERMSRV/<address>` entry written to the OS store, if any.
    pub credential_target: Option<String>,
}

/// An alternative address of the same host reached through a VPN the user
/// runs (WireGuard, Tailscale, ...). It is not a separate identity: the host
/// must present the host key pinned for its SSH endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnEndpoint {
    pub endpoint_id: String,
    pub address: String,
    pub port: u16,
    pub enabled: bool,
}

pub const MAX_VPN_ENDPOINTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRecord {
    pub id: String,
    pub display_name: String,
    pub os_family: String,
    pub is_local: bool,
    pub trust_state: String,
    /// JSON detected at connect time (os, arch, adb, ...). Untrusted hints only.
    pub capabilities_json: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub ssh: Option<SshEndpoint>,
    #[serde(default)]
    pub rdp: Option<RdpEndpoint>,
    /// VPN addresses, tried after the SSH endpoint's own address.
    #[serde(default)]
    pub vpn: Vec<VpnEndpoint>,
}

/// Trust state of a Remote Desktop host: `mstsc.exe` checks the server
/// certificate itself, so Senvakara pins nothing.
const RDP_TRUST: &str = "external";

const SELECT: &str = "
SELECT h.id, h.display_name, h.os_family, h.is_local, h.trust_state, h.created_at, h.updated_at,
       h.last_seen_at, e.id, e.address, e.port, e.fingerprint, s.username, s.private_key_path,
       s.passphrase_ref, h.capabilities_json, h.protocol, h.group_name, s.auth_method, s.key_id,
       s.password_ref, s.startup_snippet, r.address, r.port, r.username, r.credential_target
FROM hosts h
LEFT JOIN host_endpoints e ON e.host_id = h.id AND e.transport = 'ssh'
LEFT JOIN ssh_settings s ON s.endpoint_id = e.id
LEFT JOIN rdp_settings r ON r.host_id = h.id";

fn map_row(r: &Row<'_>) -> rusqlite::Result<HostRecord> {
    let endpoint_id: Option<String> = r.get(8)?;
    let ssh = match endpoint_id {
        None => None,
        Some(endpoint_id) => {
            let method: Option<String> = r.get(18)?;
            Some(SshEndpoint {
                endpoint_id,
                address: r.get(9)?,
                port: r.get(10)?,
                fingerprint: r.get(11)?,
                username: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
                auth: SshAuth::from_columns(
                    method.as_deref().unwrap_or(""),
                    r.get(13)?,
                    r.get(19)?,
                ),
                password_ref: r.get(20)?,
                passphrase_ref: r.get(14)?,
                startup_snippet: r.get(21)?,
            })
        }
    };
    let rdp_address: Option<String> = r.get(22)?;
    let rdp = match rdp_address {
        None => None,
        Some(address) => Some(RdpEndpoint {
            address,
            port: r.get(23)?,
            username: r.get(24)?,
            credential_target: r.get(25)?,
        }),
    };
    Ok(HostRecord {
        id: r.get(0)?,
        display_name: r.get(1)?,
        os_family: r.get(2)?,
        is_local: r.get::<_, i64>(3)? == 1,
        trust_state: r.get(4)?,
        capabilities_json: r.get(15)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        last_seen_at: r.get(7)?,
        protocol: Protocol::parse(&r.get::<_, String>(16)?),
        group_name: r.get(17)?,
        tags: Vec::new(),
        ssh,
        rdp,
        vpn: Vec::new(),
    })
}

fn write_tags(tx: &Transaction<'_>, host_id: &str, tags: &[String]) -> Result<()> {
    tx.execute("DELETE FROM host_tags WHERE host_id=?1", [host_id])?;
    for tag in tags {
        tx.execute(
            "INSERT INTO host_tags(host_id, tag) VALUES(?1, ?2)",
            params![host_id, tag],
        )?;
    }
    Ok(())
}

impl Db {
    /// Ensure the record for this Windows machine exists and return its id.
    pub fn ensure_local_host(&self, display_name: &str) -> Result<String> {
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM hosts WHERE is_local = 1", [], |r| r.get(0))
            .optional()?
        {
            return Ok(id);
        }
        let id = new_id("host");
        self.conn.execute(
            "INSERT INTO hosts(id, display_name, os_family, is_local, trust_state) VALUES(?1, ?2, 'windows', 1, 'local')",
            params![id, display_name],
        )?;
        Ok(id)
    }

    pub fn list_hosts(&self) -> Result<Vec<HostRecord>> {
        let mut st = self.conn.prepare(&format!(
            "{SELECT} ORDER BY h.is_local DESC, h.display_name COLLATE NOCASE"
        ))?;
        let rows = st
            .query_map([], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|h| self.with_extras(h)).collect()
    }

    pub fn get_host(&self, id: &str) -> Result<HostRecord> {
        let h = self
            .conn
            .query_row(&format!("{SELECT} WHERE h.id = ?1"), [id], map_row)
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("host {id}")))?;
        self.with_extras(h)
    }

    /// Fill in the VPN addresses and tags of a host read by [`SELECT`].
    fn with_extras(&self, mut h: HostRecord) -> Result<HostRecord> {
        let mut st = self.conn.prepare(
            "SELECT id, address, port, enabled FROM host_endpoints
             WHERE host_id=?1 AND transport='vpn' ORDER BY priority, rowid",
        )?;
        h.vpn = st
            .query_map([&h.id], |r| {
                Ok(VpnEndpoint {
                    endpoint_id: r.get(0)?,
                    address: r.get(1)?,
                    port: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? == 1,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = self
            .conn
            .prepare("SELECT tag FROM host_tags WHERE host_id=?1 ORDER BY tag COLLATE NOCASE")?;
        h.tags = st
            .query_map([&h.id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(h)
    }

    /// Register a VPN address for a remote SSH host (NET-03).
    pub fn add_vpn_endpoint(&self, host_id: &str, address: &str, port: u16) -> Result<HostRecord> {
        let address = address.trim();
        if !valid_address(address) {
            return Err(StorageError::Invalid(
                "address must be a hostname or IP without spaces".into(),
            ));
        }
        if port == 0 {
            return Err(StorageError::Invalid("port must be 1-65535".into()));
        }
        let host = self.get_host(host_id)?;
        let ssh = host
            .ssh
            .as_ref()
            .ok_or_else(|| StorageError::Invalid("only SSH hosts can have VPN addresses".into()))?;
        if ssh.address.eq_ignore_ascii_case(address) && ssh.port == port {
            return Err(StorageError::Invalid(
                "this is already the host's main address".into(),
            ));
        }
        if host
            .vpn
            .iter()
            .any(|v| v.address.eq_ignore_ascii_case(address) && v.port == port)
        {
            return Err(StorageError::Invalid(
                "this VPN address is already registered".into(),
            ));
        }
        if host.vpn.len() >= MAX_VPN_ENDPOINTS {
            return Err(StorageError::Invalid(format!(
                "at most {MAX_VPN_ENDPOINTS} VPN addresses per host"
            )));
        }
        self.conn.execute(
            "INSERT INTO host_endpoints(id, host_id, transport, address, port, priority) VALUES(?1, ?2, 'vpn', ?3, ?4, 200)",
            params![new_id("ep"), host_id, address, port],
        )?;
        self.get_host(host_id)
    }

    pub fn remove_vpn_endpoint(&self, host_id: &str, endpoint_id: &str) -> Result<HostRecord> {
        let n = self.conn.execute(
            "DELETE FROM host_endpoints WHERE id=?1 AND host_id=?2 AND transport='vpn'",
            params![endpoint_id, host_id],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("VPN address {endpoint_id}")));
        }
        self.get_host(host_id)
    }

    /// The keychain key must exist before a host can point at it.
    fn check_key_exists(&self, auth: &SshAuth) -> Result<()> {
        if let Some(key_id) = auth.key_id() {
            self.get_key(key_id)?;
        }
        Ok(())
    }

    pub fn create_host(&mut self, input: &HostUpsert) -> Result<HostRecord> {
        let v = input.validated()?;
        self.check_key_exists(&v.auth)?;
        let host_id = new_id("host");
        let tx = self.conn.transaction()?;
        let trust = match v.protocol {
            Protocol::Ssh => "untrusted",
            Protocol::Rdp => RDP_TRUST,
        };
        tx.execute(
            "INSERT INTO hosts(id, display_name, os_family, trust_state, protocol, group_name)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                host_id,
                v.display_name,
                v.os_family,
                trust,
                v.protocol.as_str(),
                v.group_name
            ],
        )?;
        match v.protocol {
            Protocol::Ssh => {
                let endpoint_id = new_id("ep");
                tx.execute(
                    "INSERT INTO host_endpoints(id, host_id, transport, address, port) VALUES(?1, ?2, 'ssh', ?3, ?4)",
                    params![endpoint_id, host_id, v.address, v.port],
                )?;
                tx.execute(
                    "INSERT INTO ssh_settings(endpoint_id, username, auth_method, private_key_path, key_id, startup_snippet)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        endpoint_id,
                        v.username,
                        v.auth.method(),
                        v.auth.key_path(),
                        v.auth.key_id(),
                        v.startup_snippet
                    ],
                )?;
            }
            Protocol::Rdp => {
                tx.execute(
                    "INSERT INTO rdp_settings(host_id, address, port, username) VALUES(?1, ?2, ?3, ?4)",
                    params![host_id, v.address, v.port, v.username],
                )?;
            }
        }
        write_tags(&tx, &host_id, &v.tags)?;
        tx.commit()?;
        self.get_host(&host_id)
    }

    /// Edit a host. The protocol cannot change: an SSH host and a Remote
    /// Desktop host keep different settings, so the user adds a new host.
    pub fn update_host(&mut self, id: &str, input: &HostUpsert) -> Result<HostRecord> {
        let v = input.validated()?;
        self.check_key_exists(&v.auth)?;
        let current = self.get_host(id)?;
        if current.is_local {
            return Err(StorageError::Invalid("this PC cannot be edited".into()));
        }
        if current.protocol != v.protocol {
            return Err(StorageError::Invalid(
                "the connection type of a host cannot change; add a new host instead".into(),
            ));
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE hosts SET display_name=?2, os_family=?3, group_name=?4,
                    updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, v.display_name, v.os_family, v.group_name],
        )?;
        match v.protocol {
            Protocol::Ssh => {
                let ep = current
                    .ssh
                    .ok_or_else(|| StorageError::Invalid("host has no SSH endpoint".into()))?;
                let endpoint_moved = ep.address != v.address || ep.port != v.port;
                tx.execute(
                    "UPDATE host_endpoints SET address=?2, port=?3 WHERE id=?1",
                    params![ep.endpoint_id, v.address, v.port],
                )?;
                if endpoint_moved {
                    // A different machine may answer at the new address: require re-trust.
                    tx.execute(
                        "UPDATE host_endpoints SET fingerprint=NULL WHERE id=?1",
                        [&ep.endpoint_id],
                    )?;
                    tx.execute("UPDATE hosts SET trust_state='untrusted' WHERE id=?1", [id])?;
                }
                tx.execute(
                    "UPDATE ssh_settings SET username=?2, auth_method=?3, private_key_path=?4,
                            key_id=?5, startup_snippet=?6 WHERE endpoint_id=?1",
                    params![
                        ep.endpoint_id,
                        v.username,
                        v.auth.method(),
                        v.auth.key_path(),
                        v.auth.key_id(),
                        v.startup_snippet
                    ],
                )?;
            }
            Protocol::Rdp => {
                tx.execute(
                    "UPDATE rdp_settings SET address=?2, port=?3, username=?4 WHERE host_id=?1",
                    params![id, v.address, v.port, v.username],
                )?;
            }
        }
        write_tags(&tx, id, &v.tags)?;
        tx.commit()?;
        self.get_host(id)
    }

    /// Pin a host-key fingerprint the user explicitly approved.
    pub fn trust_host_key(&self, id: &str, fingerprint: &str) -> Result<HostRecord> {
        if !fingerprint.starts_with("SHA256:") {
            return Err(StorageError::Invalid(
                "expected a SHA256 fingerprint".into(),
            ));
        }
        let n = self.conn.execute(
            "UPDATE host_endpoints SET fingerprint=?2 WHERE host_id=?1 AND transport='ssh'",
            params![id, fingerprint],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!(
                "ssh endpoint for host {id}"
            )));
        }
        self.conn.execute(
            "UPDATE hosts SET trust_state='trusted', updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            [id],
        )?;
        self.get_host(id)
    }

    fn ssh_endpoint_id(&self, id: &str) -> Result<String> {
        self.get_host(id)?
            .ssh
            .map(|ep| ep.endpoint_id)
            .ok_or_else(|| StorageError::Invalid("host has no SSH endpoint".into()))
    }

    pub fn set_passphrase_ref(&self, id: &str, reference: Option<&str>) -> Result<()> {
        let endpoint_id = self.ssh_endpoint_id(id)?;
        self.conn.execute(
            "UPDATE ssh_settings SET passphrase_ref=?2 WHERE endpoint_id=?1",
            params![endpoint_id, reference],
        )?;
        Ok(())
    }

    /// Point an SSH host at its login password in the OS store (CRED-01).
    pub fn set_password_ref(&self, id: &str, reference: Option<&str>) -> Result<()> {
        let endpoint_id = self.ssh_endpoint_id(id)?;
        self.conn.execute(
            "UPDATE ssh_settings SET password_ref=?2 WHERE endpoint_id=?1",
            params![endpoint_id, reference],
        )?;
        Ok(())
    }

    /// Record which `TERMSRV/...` entry holds a Remote Desktop host's
    /// credentials (DESK-IMPL-04).
    pub fn set_rdp_credential_target(&self, id: &str, target: Option<&str>) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE rdp_settings SET credential_target=?2 WHERE host_id=?1",
            params![id, target],
        )?;
        if n == 0 {
            return Err(StorageError::Invalid(
                "host is not a remote desktop host".into(),
            ));
        }
        Ok(())
    }

    /// Record capabilities detected on the last successful connection.
    pub fn set_capabilities(&self, id: &str, capabilities_json: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE hosts SET capabilities_json=?2, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, capabilities_json],
        )?;
        Ok(())
    }

    pub fn mark_seen(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE hosts SET last_seen_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            [id],
        )?;
        Ok(())
    }

    /// Every parent group in use, for suggestions in the host form.
    pub fn host_groups(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "SELECT DISTINCT group_name FROM hosts WHERE group_name IS NOT NULL
             ORDER BY group_name COLLATE NOCASE",
        )?;
        let rows = st
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Delete a remote host. The local machine record cannot be deleted.
    /// The caller removes the host's secrets from the OS store.
    pub fn delete_host(&self, id: &str) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM hosts WHERE id=?1 AND is_local=0", [id])?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("removable host {id}")));
        }
        Ok(())
    }
}
