//! Device and route registry (INV-02, INV-04, AND-03, task AND-IMPL-03).
//!
//! A *route* is (host, connection type, host-local serial). A *device* is
//! the user-facing identity. In this version each newly seen route creates a
//! new provisional device: devices are never merged by model, name or serial
//! across hosts (docs/ARCHITECTURE.md §3, docs/DATABASE.md §5).

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceRouteRecord {
    pub device_id: String,
    pub display_name: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub identity_status: String,
    pub route_id: String,
    pub host_id: String,
    pub host_name: String,
    pub connection_type: String,
    /// Host-local ADB serial.
    pub serial: String,
    /// `available`, `unauthorized`, `offline` or `unavailable`.
    pub state: String,
    pub last_seen_at: Option<String>,
}

const SELECT: &str = "
SELECT d.id, d.display_name, d.manufacturer, d.model, d.identity_status,
       r.id, r.host_id, h.display_name, r.connection_type, r.transport_identifier, r.state, r.last_seen_at
FROM device_routes r
JOIN devices d ON d.id = r.device_id
JOIN hosts h ON h.id = r.host_id";

fn map_row(r: &Row<'_>) -> rusqlite::Result<DeviceRouteRecord> {
    Ok(DeviceRouteRecord {
        device_id: r.get(0)?,
        display_name: r.get(1)?,
        manufacturer: r.get(2)?,
        model: r.get(3)?,
        identity_status: r.get(4)?,
        route_id: r.get(5)?,
        host_id: r.get(6)?,
        host_name: r.get(7)?,
        connection_type: r.get(8)?,
        serial: r.get(9)?,
        state: r.get(10)?,
        last_seen_at: r.get(11)?,
    })
}

const STATES: &[&str] = &["available", "unauthorized", "offline", "unavailable"];

impl Db {
    /// Record a route observed on `host_id`. Returns the route record,
    /// creating a provisional device the first time the route is seen.
    pub fn observe_route(
        &mut self,
        host_id: &str,
        connection_type: &str,
        serial: &str,
        state: &str,
        model: Option<&str>,
    ) -> Result<DeviceRouteRecord> {
        if !["usb_adb", "wifi_adb"].contains(&connection_type) {
            return Err(StorageError::Invalid("unknown connection type".into()));
        }
        if !STATES.contains(&state) {
            return Err(StorageError::Invalid(format!(
                "unknown route state {state}"
            )));
        }
        if serial.trim().is_empty() {
            return Err(StorageError::Invalid("empty serial".into()));
        }
        let tx = self.conn.transaction()?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT id, device_id FROM device_routes WHERE host_id=?1 AND connection_type=?2 AND transport_identifier=?3",
                params![host_id, connection_type, serial],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let route_id = match existing {
            Some((route_id, device_id)) => {
                tx.execute(
                    "UPDATE device_routes SET state=?2, last_seen_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                    params![route_id, state],
                )?;
                // Fill in the model once it becomes known (e.g. after authorisation);
                // never overwrite a user-chosen display name.
                if let Some(m) = model {
                    tx.execute(
                        "UPDATE devices SET model=?2, display_name=CASE WHEN display_name=?3 THEN ?2 ELSE display_name END,
                         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND (model IS NULL OR model<>?2)",
                        params![device_id, m, serial],
                    )?;
                }
                route_id
            }
            None => {
                let device_id = new_id("dev");
                let route_id = new_id("route");
                tx.execute(
                    "INSERT INTO devices(id, device_type, display_name, model) VALUES(?1, 'android', ?2, ?3)",
                    params![device_id, model.unwrap_or(serial), model],
                )?;
                tx.execute(
                    "INSERT INTO device_routes(id, device_id, host_id, connection_type, transport_identifier, state, last_seen_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                    params![route_id, device_id, host_id, connection_type, serial, state],
                )?;
                route_id
            }
        };
        tx.commit()?;
        self.get_route(&route_id)
    }

    /// Mark every route of `host_id` not in `present` as unavailable
    /// (device unplugged, or the host itself went away when `present` is empty).
    pub fn mark_missing_routes(&self, host_id: &str, present: &[String]) -> Result<usize> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM device_routes WHERE host_id=?1 AND state<>'unavailable'")?;
        let ids: Vec<String> = st
            .query_map([host_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut n = 0;
        for id in ids.iter().filter(|id| !present.contains(id)) {
            n += self.conn.execute(
                "UPDATE device_routes SET state='unavailable' WHERE id=?1",
                [id],
            )?;
        }
        Ok(n)
    }

    pub fn list_device_routes(&self) -> Result<Vec<DeviceRouteRecord>> {
        let mut st = self.conn.prepare(&format!(
            "{SELECT} ORDER BY CASE r.state WHEN 'available' THEN 0 WHEN 'unauthorized' THEN 1 WHEN 'offline' THEN 2 ELSE 3 END,
             h.is_local DESC, d.display_name COLLATE NOCASE"
        ))?;
        let rows = st
            .query_map([], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_route(&self, route_id: &str) -> Result<DeviceRouteRecord> {
        self.conn
            .query_row(&format!("{SELECT} WHERE r.id=?1"), [route_id], map_row)
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("route {route_id}")))
    }

    pub fn rename_device(&self, device_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err(StorageError::Invalid("name must be 1-80 characters".into()));
        }
        let n = self.conn.execute(
            "UPDATE devices SET display_name=?2, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![device_id, name],
        )?;
        if n == 0 {
            return Err(StorageError::NotFound(format!("device {device_id}")));
        }
        Ok(())
    }

    /// Forget a device and all of its routes (e.g. a phone no longer used).
    pub fn forget_device(&self, device_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM devices WHERE id=?1", [device_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, HostUpsert};

    fn setup() -> (Db, String, String) {
        let mut db = Db::open_in_memory().unwrap();
        let win = db.ensure_local_host("PC").unwrap();
        let linux = db
            .create_host(&HostUpsert {
                display_name: "Linux".into(),
                os_family: "linux".into(),
                address: "192.0.2.10".into(),
                port: 22,
                username: "u".into(),
                private_key_path: "k".into(),
            })
            .unwrap()
            .id;
        (db, win, linux)
    }

    #[test]
    fn same_serial_on_two_hosts_is_two_devices() {
        let (mut db, win, linux) = setup();
        let a = db
            .observe_route(&win, "usb_adb", "SERIAL1", "available", Some("Pixel 7"))
            .unwrap();
        let b = db
            .observe_route(&linux, "usb_adb", "SERIAL1", "available", Some("Pixel 7"))
            .unwrap();
        assert_ne!(
            a.device_id, b.device_id,
            "never merge across hosts by serial/model"
        );
        assert_ne!(a.route_id, b.route_id);
        assert_eq!(db.list_device_routes().unwrap().len(), 2);
    }

    #[test]
    fn identical_models_stay_separate_and_reobserve_is_stable() {
        let (mut db, win, _) = setup();
        let a = db
            .observe_route(&win, "usb_adb", "S1", "available", Some("Redmi 5A"))
            .unwrap();
        let b = db
            .observe_route(&win, "usb_adb", "S2", "available", Some("Redmi 5A"))
            .unwrap();
        assert_ne!(a.device_id, b.device_id);
        let again = db
            .observe_route(&win, "usb_adb", "S1", "offline", None)
            .unwrap();
        assert_eq!(again.device_id, a.device_id);
        assert_eq!(again.state, "offline");
    }

    #[test]
    fn unplug_keeps_user_label_and_model_fills_in_later() {
        let (mut db, win, _) = setup();
        let r = db
            .observe_route(&win, "usb_adb", "S1", "unauthorized", None)
            .unwrap();
        assert_eq!(r.display_name, "S1");
        let r = db
            .observe_route(&win, "usb_adb", "S1", "available", Some("Pixel 7"))
            .unwrap();
        assert_eq!(
            r.display_name, "Pixel 7",
            "serial placeholder replaced by model"
        );
        db.rename_device(&r.device_id, "Test phone A").unwrap();
        db.observe_route(&win, "usb_adb", "S1", "available", Some("Pixel 8"))
            .unwrap();
        assert_eq!(db.mark_missing_routes(&win, &[]).unwrap(), 1);
        let after = db.get_route(&r.route_id).unwrap();
        assert_eq!(after.state, "unavailable");
        assert_eq!(
            after.display_name, "Test phone A",
            "user label survives unplug and model change"
        );
    }

    #[test]
    fn rejects_unknown_state_and_type() {
        let (mut db, win, _) = setup();
        assert!(db
            .observe_route(&win, "usb_adb", "S", "bogus", None)
            .is_err());
        assert!(db
            .observe_route(&win, "bluetooth", "S", "available", None)
            .is_err());
    }
}
