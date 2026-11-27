//! Saved workflows of the Workflows canvas (plan WF). The graph is stored as
//! JSON and is opaque here: `crates/workflow` validates it before a run.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MAX_WORKFLOW_NAME_LEN: usize = 120;
/// Large enough for hundreds of nodes with edited scripts.
pub const MAX_GRAPH_JSON_LEN: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowRecord {
    pub id: String,
    pub name: String,
    pub graph: serde_json::Value,
    pub from_recipe_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowSave {
    /// `None` creates a workflow.
    pub id: Option<String>,
    pub name: String,
    pub graph: serde_json::Value,
}

fn checked(name: &str, graph: &serde_json::Value) -> Result<(String, String)> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_WORKFLOW_NAME_LEN {
        return Err(StorageError::Invalid(format!(
            "a workflow name needs 1 to {MAX_WORKFLOW_NAME_LEN} characters"
        )));
    }
    if !graph.is_object() {
        return Err(StorageError::Invalid(
            "a workflow graph must be an object".into(),
        ));
    }
    let json = graph.to_string();
    if json.len() > MAX_GRAPH_JSON_LEN {
        return Err(StorageError::Invalid(
            "the workflow is too large to save".into(),
        ));
    }
    Ok((name.to_string(), json))
}

impl Db {
    pub fn list_workflows(&self) -> Result<Vec<WorkflowRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, name, graph_json, from_recipe_id, updated_at FROM workflows \
             ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = st
            .query_map([], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_workflow(&self, id: &str) -> Result<WorkflowRecord> {
        self.conn
            .query_row(
                "SELECT id, name, graph_json, from_recipe_id, updated_at FROM workflows WHERE id=?1",
                [id],
                row,
            )
            .optional()?
            .ok_or_else(|| StorageError::NotFound(format!("workflow {id}")))
    }

    /// Creates or replaces a workflow; returns what was stored.
    pub fn save_workflow(&self, input: &WorkflowSave) -> Result<WorkflowRecord> {
        let (name, json) = checked(&input.name, &input.graph)?;
        let id = match &input.id {
            Some(id) => {
                let n = self.conn.execute(
                    "UPDATE workflows SET name=?2, graph_json=?3, \
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                    params![id, name, json],
                )?;
                if n == 0 {
                    return Err(StorageError::NotFound(format!("workflow {id}")));
                }
                id.clone()
            }
            None => {
                let id = new_id("wf");
                self.conn.execute(
                    "INSERT INTO workflows(id, name, graph_json) VALUES(?1, ?2, ?3)",
                    params![id, name, json],
                )?;
                id
            }
        };
        self.get_workflow(&id)
    }

    /// Stores the workflow converted from a recipe, once: a second call for
    /// the same recipe returns `None` and changes nothing.
    pub fn import_recipe_workflow(
        &self,
        recipe_id: &str,
        name: &str,
        graph: &serde_json::Value,
    ) -> Result<Option<String>> {
        let (name, json) = checked(name, graph)?;
        let id = new_id("wf");
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO workflows(id, name, graph_json, from_recipe_id) \
             VALUES(?1, ?2, ?3, ?4)",
            params![id, name, json, recipe_id],
        )?;
        Ok((n == 1).then_some(id))
    }

    /// Deletes a workflow. Its runs stay in the history without the link.
    pub fn delete_workflow(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM workflows WHERE id=?1", [id])?;
        Ok(())
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRecord> {
    let json: String = r.get(2)?;
    Ok(WorkflowRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        graph: serde_json::from_str(&json).unwrap_or_else(|_| serde_json::json!({})),
        from_recipe_id: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::{Db, StorageError, WorkflowSave};
    use serde_json::json;

    fn save(db: &Db, id: Option<&str>, name: &str) -> crate::Result<crate::WorkflowRecord> {
        db.save_workflow(&WorkflowSave {
            id: id.map(str::to_string),
            name: name.into(),
            graph: json!({"version": 1, "nodes": []}),
        })
    }

    #[test]
    fn workflows_are_created_updated_listed_and_deleted() {
        let db = Db::open_in_memory().unwrap();
        let a = save(&db, None, "  VPS baseline ").unwrap();
        assert_eq!(a.name, "VPS baseline");
        assert_eq!(a.graph["version"], 1);
        save(&db, None, "app stack").unwrap();
        let renamed = save(&db, Some(&a.id), "Baseline").unwrap();
        assert_eq!(renamed.id, a.id);
        let names: Vec<String> = db
            .list_workflows()
            .unwrap()
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(names, ["app stack", "Baseline"]);
        db.delete_workflow(&a.id).unwrap();
        assert!(matches!(
            db.get_workflow(&a.id),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn bad_names_graphs_and_missing_ids_are_refused() {
        let db = Db::open_in_memory().unwrap();
        assert!(save(&db, None, "   ").is_err());
        assert!(save(&db, None, &"x".repeat(121)).is_err());
        assert!(save(&db, Some("wf-missing"), "x").is_err());
        let not_object = db.save_workflow(&WorkflowSave {
            id: None,
            name: "x".into(),
            graph: json!([1, 2]),
        });
        assert!(not_object.is_err());
    }

    #[test]
    fn a_recipe_is_converted_only_once() {
        let db = Db::open_in_memory().unwrap();
        let g = json!({"nodes": []});
        let first = db
            .import_recipe_workflow("rec-1", "Smoke test", &g)
            .unwrap();
        assert!(first.is_some());
        assert_eq!(
            db.import_recipe_workflow("rec-1", "Smoke test", &g)
                .unwrap(),
            None
        );
        let all = db.list_workflows().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].from_recipe_id.as_deref(), Some("rec-1"));
    }

    #[test]
    fn deleting_a_workflow_keeps_its_runs() {
        let mut db = Db::open_in_memory().unwrap();
        let wf = save(&db, None, "x").unwrap();
        let run = db
            .create_workflow_run(&wf.id, &json!({}), &[("n1", "automation.wait")])
            .unwrap();
        db.delete_workflow(&wf.id).unwrap();
        let runs = db.list_runs(5).unwrap();
        assert_eq!(runs[0].id, run);
        assert_eq!(runs[0].workflow_id, None);
        assert_eq!(runs[0].steps[0].node_id.as_deref(), Some("n1"));
    }
}
