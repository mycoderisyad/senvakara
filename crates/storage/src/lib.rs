//! SQLite persistence (task CORE-02). Stores metadata only; secrets live in
//! the OS credential store and are referenced by opaque ids (ADR-009).
//!
//! Migrations are versioned with `PRAGMA user_version` and applied in a
//! transaction (docs/DATABASE.md §5).

mod devices;
mod hosts;
mod keychain;
mod recipes;
mod workflows;
mod workspaces;

pub use devices::DeviceRouteRecord;
pub use hosts::{
    HostRecord, HostUpsert, Protocol, RdpEndpoint, SshAuth, SshEndpoint, VpnEndpoint,
    MAX_GROUP_LEN, MAX_SNIPPET_LEN, MAX_TAGS, MAX_TAG_LEN, MAX_VPN_ENDPOINTS,
};
pub use keychain::{KeyKind, KeyRecord, KeyUpsert, MAX_KEY_LABEL_LEN};
pub use recipes::{RecipePermissions, RecipeRecord, RecipeSave, RecipeStep};
pub use workflows::{RunRecord, StepRecord};
pub use workspaces::{PanelRecord, WorkspaceRecord, WorkspaceSave};

use rusqlite::Connection;
use std::path::Path;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_workspaces.sql"),
    include_str!("../migrations/0003_workflows.sql"),
    include_str!("../migrations/0004_recipes.sql"),
    include_str!("../migrations/0005_credentials.sql"),
];

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("database schema v{found} is newer than this app supports (v{supported})")]
    TooNew { found: u32, supported: u32 },
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// How long a statement waits for a lock held by another connection.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // Wait instead of failing with SQLITE_BUSY when another connection
        // (a second front end, a probe on the same file) holds the write lock.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()? as usize;
        if current > MIGRATIONS.len() {
            return Err(StorageError::TooNew {
                found: current as u32,
                supported: MIGRATIONS.len() as u32,
            });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as u32)?;
            tx.commit()?;
        }
        Ok(())
    }
}

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_db_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("svk-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        assert_eq!(Db::open(&path).unwrap().schema_version().unwrap(), 5);
        // Re-opening must not re-run migrations.
        assert_eq!(Db::open(&path).unwrap().schema_version().unwrap(), 5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn waits_for_locks_instead_of_failing_at_once() {
        let db = Db::open_in_memory().unwrap();
        let ms: i64 = db
            .conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(ms, BUSY_TIMEOUT.as_millis() as i64);
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn upgrading_from_v4_keeps_hosts_signing_in_with_their_key_file() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in &MIGRATIONS[..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4).unwrap();
        conn.execute_batch(
            "INSERT INTO hosts(id, display_name, os_family, trust_state) VALUES('h1','Fedora','linux','trusted');
             INSERT INTO host_endpoints(id, host_id, transport, address, port, fingerprint)
                 VALUES('e1','h1','ssh','192.0.2.10',22,'SHA256:abc');
             INSERT INTO ssh_settings(endpoint_id, username, private_key_path, passphrase_ref)
                 VALUES('e1','dev','C:\\keys\\id','secret-p');",
        )
        .unwrap();

        let db = Db::init(conn).unwrap();
        assert_eq!(db.schema_version().unwrap(), 5);
        let host = db.get_host("h1").unwrap();
        assert_eq!(host.protocol, Protocol::Ssh);
        assert_eq!(host.trust_state, "trusted");
        assert!(host.tags.is_empty() && host.group_name.is_none() && host.rdp.is_none());
        let ssh = host.ssh.unwrap();
        assert_eq!(ssh.fingerprint.as_deref(), Some("SHA256:abc"));
        assert_eq!(ssh.username, "dev");
        assert_eq!(
            ssh.auth,
            SshAuth::KeyFile {
                path: r"C:\keys\id".into()
            }
        );
        assert_eq!(ssh.passphrase_ref.as_deref(), Some("secret-p"));
        assert_eq!(ssh.password_ref, None);
        // The rebuilt table still cascades from its endpoint.
        db.delete_host("h1").unwrap();
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM ssh_settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Db::open_in_memory().unwrap();
        let err = db
            .conn
            .execute(
                "INSERT INTO host_endpoints(id,host_id,transport,address) VALUES('e','missing','ssh','x')",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"));
    }
}
