//! Host registry (INV-01, INV-04). A host's id never derives from its
//! address; changing the address keeps the identity but clears the pinned
//! host key, because a different machine may answer at the new address.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshEndpoint {
    pub endpoint_id: String,
    pub address: String,
    pub port: u16,
    /// Pinned host-key fingerprint. `None` until the user approves one.
    pub fingerprint: Option<String>,
    pub username: String,
    pub private_key_path: String,
    /// Opaque credential-store reference for the key passphrase, if any.
    pub passphrase_ref: Option<String>,
}

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
    pub ssh: Option<SshEndpoint>,
}

/// Input for creating or editing a remote SSH host.
#[derive(Debug, Clone, Deserialize)]
pub struct HostUpsert {
    pub display_name: String,
    pub os_family: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub private_key_path: String,
}

impl HostUpsert {
    fn validated(&self) -> Result<HostUpsert> {
        let t = |s: &str| s.trim().to_string();
        let v = HostUpsert {
            display_name: t(&self.display_name),
            os_family: t(&self.os_family),
            address: t(&self.address),
            port: self.port,
            username: t(&self.username),
            private_key_path: t(&self.private_key_path),
        };
        let bad = |m: &str| Err(StorageError::Invalid(m.into()));
        if v.display_name.is_empty() {
            return bad("display name is required");
        }
        if !["windows", "linux", "macos", "unknown"].contains(&v.os_family.as_str()) {
            return bad("unsupported OS family");
        }
        if v.address.is_empty() || v.address.chars().any(|c| c.is_whitespace() || c == '/') {
            return bad("address must be a hostname or IP without spaces");
        }
        if v.port == 0 {
            return bad("port must be 1-65535");
        }
        if v.username.is_empty() || v.username.chars().any(char::is_whitespace) {
            return bad("username is required and cannot contain spaces");
        }
        if v.private_key_path.is_empty() {
            return bad("private key path is required");
        }
        Ok(v)
    }
}

const SELECT: &str = "
SELECT h.id, h.display_name, h.os_family, h.is_local, h.trust_state, h.created_at, h.updated_at,
       h.last_seen_at, e.id, e.address, e.port, e.fingerprint, s.username, s.private_key_path,
       s.passphrase_ref, h.capabilities_json
FROM hosts h
LEFT JOIN host_endpoints e ON e.host_id = h.id AND e.transport = 'ssh'
LEFT JOIN ssh_settings s ON s.endpoint_id = e.id";

fn map_row(r: &Row<'_>) -> rusqlite::Result<HostRecord> {
    let endpoint_id: Option<String> = r.get(8)?;
    let ssh = match endpoint_id {
        None => None,
        Some(endpoint_id) => Some(SshEndpoint {
            endpoint_id,
            address: r.get(9)?,
            port: r.get(10)?,
            fingerprint: r.get(11)?,
            username: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
            private_key_path: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
            passphrase_ref: r.get(14)?,
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
        ssh,
    })
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
        Ok(rows)
    }

    pub fn get_host(&self, id: &str) -> Result<HostRecord> {
        self.conn
            .query_row(&format!("{SELECT} WHERE h.id = ?1"), [id], map_row)
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("host {id}")))
    }

    pub fn create_host(&mut self, input: &HostUpsert) -> Result<HostRecord> {
        let v = input.validated()?;
        let host_id = new_id("host");
        let endpoint_id = new_id("ep");
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO hosts(id, display_name, os_family, trust_state) VALUES(?1, ?2, ?3, 'untrusted')",
            params![host_id, v.display_name, v.os_family],
        )?;
        tx.execute(
            "INSERT INTO host_endpoints(id, host_id, transport, address, port) VALUES(?1, ?2, 'ssh', ?3, ?4)",
            params![endpoint_id, host_id, v.address, v.port],
        )?;
        tx.execute(
            "INSERT INTO ssh_settings(endpoint_id, username, private_key_path) VALUES(?1, ?2, ?3)",
            params![endpoint_id, v.username, v.private_key_path],
        )?;
        tx.commit()?;
        self.get_host(&host_id)
    }

    pub fn update_host(&mut self, id: &str, input: &HostUpsert) -> Result<HostRecord> {
        let v = input.validated()?;
        let current = self.get_host(id)?;
        let ep = current
            .ssh
            .ok_or_else(|| StorageError::Invalid("host has no SSH endpoint".into()))?;
        let endpoint_moved = ep.address != v.address || ep.port != v.port;
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE hosts SET display_name=?2, os_family=?3, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![id, v.display_name, v.os_family],
        )?;
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
            "UPDATE ssh_settings SET username=?2, private_key_path=?3 WHERE endpoint_id=?1",
            params![ep.endpoint_id, v.username, v.private_key_path],
        )?;
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

    pub fn set_passphrase_ref(&self, id: &str, reference: Option<&str>) -> Result<()> {
        let host = self.get_host(id)?;
        let ep = host
            .ssh
            .ok_or_else(|| StorageError::Invalid("host has no SSH endpoint".into()))?;
        self.conn.execute(
            "UPDATE ssh_settings SET passphrase_ref=?2 WHERE endpoint_id=?1",
            params![ep.endpoint_id, reference],
        )?;
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

    /// Delete a remote host. The local machine record cannot be deleted.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(addr: &str) -> HostUpsert {
        HostUpsert {
            display_name: " Fedora laptop ".into(),
            os_family: "linux".into(),
            address: addr.into(),
            port: 22,
            username: "dev".into(),
            private_key_path: r"C:\keys\id".into(),
        }
    }

    #[test]
    fn create_list_and_trust() {
        let mut db = Db::open_in_memory().unwrap();
        let local = db.ensure_local_host("This PC").unwrap();
        assert_eq!(
            db.ensure_local_host("This PC").unwrap(),
            local,
            "local host is a singleton"
        );

        let h = db.create_host(&input("192.0.2.10")).unwrap();
        assert_eq!(h.display_name, "Fedora laptop");
        assert_eq!(h.trust_state, "untrusted");
        assert_eq!(h.ssh.as_ref().unwrap().fingerprint, None);

        let all = db.list_hosts().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].is_local);

        let t = db.trust_host_key(&h.id, "SHA256:abc").unwrap();
        assert_eq!(t.trust_state, "trusted");
        assert_eq!(t.ssh.unwrap().fingerprint.as_deref(), Some("SHA256:abc"));
    }

    #[test]
    fn changing_address_keeps_id_but_clears_pinned_key() {
        let mut db = Db::open_in_memory().unwrap();
        let h = db.create_host(&input("192.0.2.10")).unwrap();
        db.trust_host_key(&h.id, "SHA256:abc").unwrap();

        let renamed = db
            .update_host(
                &h.id,
                &HostUpsert {
                    display_name: "Box".into(),
                    ..input("192.0.2.10")
                },
            )
            .unwrap();
        assert_eq!(
            renamed.ssh.as_ref().unwrap().fingerprint.as_deref(),
            Some("SHA256:abc"),
            "rename keeps trust"
        );

        let moved = db.update_host(&h.id, &input("192.0.2.99")).unwrap();
        assert_eq!(moved.id, h.id);
        assert_eq!(moved.ssh.unwrap().fingerprint, None);
        assert_eq!(moved.trust_state, "untrusted");
    }

    #[test]
    fn rejects_invalid_input_and_local_delete() {
        let mut db = Db::open_in_memory().unwrap();
        assert!(matches!(
            db.create_host(&input("bad host")),
            Err(StorageError::Invalid(_))
        ));
        assert!(matches!(
            db.create_host(&HostUpsert {
                port: 0,
                ..input("192.0.2.1")
            }),
            Err(StorageError::Invalid(_))
        ));
        let local = db.ensure_local_host("This PC").unwrap();
        assert!(db.delete_host(&local).is_err());
        assert!(db.trust_host_key(&local, "MD5:xx").is_err());
    }

    #[test]
    fn delete_cascades_endpoint_and_settings() {
        let mut db = Db::open_in_memory().unwrap();
        let h = db.create_host(&input("192.0.2.10")).unwrap();
        db.delete_host(&h.id).unwrap();
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM ssh_settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
