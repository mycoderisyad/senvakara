//! Saved workspaces (WORK-01/02): a name, a layout and ordered panels whose
//! targets are references (host id, route id, shell descriptor), so a
//! workspace can be reopened later and explain what is no longer reachable.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const PANEL_TYPES: &[&str] = &["terminal", "mirror", "desktop", "files", "logcat"];
pub const MAX_PANELS: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelRecord {
    pub panel_type: String,
    /// Opaque JSON target reference, validated by the app when opened.
    pub target: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub layout: serde_json::Value,
    pub panels: Vec<PanelRecord>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceSave {
    /// `None` creates a new workspace.
    pub id: Option<String>,
    pub name: String,
    pub layout: serde_json::Value,
    pub panels: Vec<PanelRecord>,
}

impl Db {
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        let ids: Vec<String> = {
            let mut st = self
                .conn
                .prepare("SELECT id FROM workspaces ORDER BY updated_at DESC")?;
            let rows = st
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        ids.iter().map(|id| self.get_workspace(id)).collect()
    }

    pub fn get_workspace(&self, id: &str) -> Result<WorkspaceRecord> {
        let (name, layout, updated_at): (String, String, String) = self
            .conn
            .query_row(
                "SELECT name, layout_json, updated_at FROM workspaces WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("workspace {id}")))?;
        let mut st = self
            .conn
            .prepare("SELECT panel_type, target_ref_json FROM workspace_panels WHERE workspace_id=?1 ORDER BY position")?;
        let panels = st
            .query_map([id], |r| {
                let t: String = r.get(1)?;
                Ok(PanelRecord {
                    panel_type: r.get(0)?,
                    target: serde_json::from_str(&t).unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(WorkspaceRecord {
            id: id.to_string(),
            name,
            layout: serde_json::from_str(&layout).unwrap_or_default(),
            panels,
            updated_at,
        })
    }

    /// Create or replace a workspace and its panels in one transaction.
    pub fn save_workspace(&mut self, w: &WorkspaceSave) -> Result<WorkspaceRecord> {
        let name = w.name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err(StorageError::Invalid(
                "workspace name must be 1-80 characters".into(),
            ));
        }
        if w.panels.len() > MAX_PANELS {
            return Err(StorageError::Invalid(format!(
                "at most {MAX_PANELS} panels"
            )));
        }
        if let Some(p) = w
            .panels
            .iter()
            .find(|p| !PANEL_TYPES.contains(&p.panel_type.as_str()))
        {
            return Err(StorageError::Invalid(format!(
                "unknown panel type {}",
                p.panel_type
            )));
        }
        let id = w.id.clone().unwrap_or_else(|| new_id("ws"));
        let tx = self.conn.transaction()?;
        let exists: bool =
            tx.query_row("SELECT COUNT(*) FROM workspaces WHERE id=?1", [&id], |r| {
                r.get::<_, i64>(0)
            })? > 0;
        if exists {
            tx.execute(
                "UPDATE workspaces SET name=?2, layout_json=?3, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                params![id, name, w.layout.to_string()],
            )?;
        } else if w.id.is_some() {
            return Err(StorageError::NotFound(format!("workspace {id}")));
        } else {
            tx.execute(
                "INSERT INTO workspaces(id, name, layout_json) VALUES(?1, ?2, ?3)",
                params![id, name, w.layout.to_string()],
            )?;
        }
        tx.execute("DELETE FROM workspace_panels WHERE workspace_id=?1", [&id])?;
        for (pos, p) in w.panels.iter().enumerate() {
            tx.execute(
                "INSERT INTO workspace_panels(id, workspace_id, panel_type, target_ref_json, position) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![new_id("panel"), id, p.panel_type, p.target.to_string(), pos as i64],
            )?;
        }
        tx.commit()?;
        self.get_workspace(&id)
    }

    pub fn delete_workspace(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM workspaces WHERE id=?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn save(id: Option<String>, panels: Vec<PanelRecord>) -> WorkspaceSave {
        WorkspaceSave {
            id,
            name: "Flutter testing".into(),
            layout: json!({"layout": "columns"}),
            panels,
        }
    }

    #[test]
    fn save_reopen_update_delete() {
        let mut db = Db::open_in_memory().unwrap();
        let panels = vec![
            PanelRecord {
                panel_type: "terminal".into(),
                target: json!({"kind": "ssh", "host_id": "h1", "tmux": "senvakara-x"}),
            },
            PanelRecord {
                panel_type: "mirror".into(),
                target: json!({"route_id": "r1"}),
            },
        ];
        let w = db.save_workspace(&save(None, panels.clone())).unwrap();
        assert_eq!(w.panels, panels, "order and targets survive a round trip");
        assert_eq!(db.list_workspaces().unwrap().len(), 1);

        let w2 = db
            .save_workspace(&save(Some(w.id.clone()), panels[..1].to_vec()))
            .unwrap();
        assert_eq!(w2.id, w.id);
        assert_eq!(w2.panels.len(), 1);

        db.delete_workspace(&w.id).unwrap();
        assert!(db.list_workspaces().unwrap().is_empty());
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM workspace_panels", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "panels cascade");
    }

    #[test]
    fn rejects_bad_input() {
        let mut db = Db::open_in_memory().unwrap();
        let bad = PanelRecord {
            panel_type: "browser".into(),
            target: json!({}),
        };
        assert!(db.save_workspace(&save(None, vec![bad])).is_err());
        let many = vec![
            PanelRecord {
                panel_type: "files".into(),
                target: json!({})
            };
            5
        ];
        assert!(db.save_workspace(&save(None, many)).is_err());
        assert!(db
            .save_workspace(&WorkspaceSave {
                name: " ".into(),
                ..save(None, vec![])
            })
            .is_err());
        assert!(db
            .save_workspace(&save(Some("ws-missing".into()), vec![]))
            .is_err());
    }
}
