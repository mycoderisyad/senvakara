//! The last security posture check of each host (plan WF §5, schema v7). The
//! result is opaque JSON here; `crates/workflow` reads and writes it. Only
//! the summary of the check is kept, never command output.

use crate::{Db, Result, StorageError};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// A posture result is a short list of controls; anything larger is refused.
pub const MAX_POSTURE_JSON_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PostureRecord {
    pub host_id: String,
    /// When the check ran (UTC, ISO 8601).
    pub checked_at: String,
    /// Seconds since then, as the database clock sees it.
    pub age_secs: i64,
    pub results: serde_json::Value,
}

const SELECT: &str = "SELECT host_id, checked_at, \
     CAST((julianday('now') - julianday(checked_at)) * 86400 AS INTEGER), results_json \
     FROM host_posture";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PostureRecord> {
    let json: String = r.get(3)?;
    Ok(PostureRecord {
        host_id: r.get(0)?,
        checked_at: r.get(1)?,
        age_secs: r.get::<_, Option<i64>>(2)?.unwrap_or(i64::MAX).max(0),
        results: serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
    })
}

impl Db {
    /// Stores the newest posture of a host, replacing the previous one.
    pub fn save_posture(
        &self,
        host_id: &str,
        results: &serde_json::Value,
    ) -> Result<PostureRecord> {
        let json = results.to_string();
        if !results.is_object() || json.len() > MAX_POSTURE_JSON_LEN {
            return Err(StorageError::Invalid(
                "a posture result must be a small object".into(),
            ));
        }
        self.conn.execute(
            "INSERT INTO host_posture(host_id, results_json) VALUES(?1, ?2) \
             ON CONFLICT(host_id) DO UPDATE SET results_json=excluded.results_json, \
             checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            params![host_id, json],
        )?;
        self.posture(host_id)?
            .ok_or_else(|| StorageError::NotFound(format!("posture of host {host_id}")))
    }

    pub fn posture(&self, host_id: &str) -> Result<Option<PostureRecord>> {
        Ok(self
            .conn
            .query_row(&format!("{SELECT} WHERE host_id=?1"), [host_id], row)
            .optional()?)
    }

    /// Every host's last posture, for the canvas.
    pub fn list_postures(&self) -> Result<Vec<PostureRecord>> {
        let mut st = self.conn.prepare(&format!("{SELECT} ORDER BY host_id"))?;
        let rows = st
            .query_map([], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, HostUpsert, Protocol, SshAuth};
    use serde_json::json;

    fn host(db: &mut Db) -> String {
        db.create_host(&HostUpsert {
            display_name: "VPS".into(),
            os_family: "linux".into(),
            protocol: Protocol::Ssh,
            address: "192.0.2.20".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuth::Agent,
            group_name: None,
            tags: Vec::new(),
            startup_snippet: None,
        })
        .unwrap()
        .id
    }

    #[test]
    fn the_newest_posture_replaces_the_last_and_goes_with_its_host() {
        let mut db = Db::open_in_memory().unwrap();
        let id = host(&mut db);
        assert_eq!(db.posture(&id).unwrap(), None);
        db.save_posture(&id, &json!({"controls": [1]})).unwrap();
        let saved = db.save_posture(&id, &json!({"controls": [2]})).unwrap();
        assert_eq!(saved.results, json!({"controls": [2]}));
        assert!((0..60).contains(&saved.age_secs));
        assert_eq!(db.list_postures().unwrap(), vec![saved]);
        db.delete_host(&id).unwrap();
        assert!(db.list_postures().unwrap().is_empty());
    }

    #[test]
    fn postures_need_a_known_host_and_a_small_object() {
        let mut db = Db::open_in_memory().unwrap();
        assert!(db.save_posture("host-missing", &json!({})).is_err());
        let id = host(&mut db);
        assert!(db.save_posture(&id, &json!([1, 2])).is_err());
        let big = "x".repeat(super::MAX_POSTURE_JSON_LEN);
        assert!(db.save_posture(&id, &json!({ "big": big })).is_err());
    }
}
