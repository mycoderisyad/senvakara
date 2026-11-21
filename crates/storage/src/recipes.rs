//! Automation recipes (FLOW-04): named, allowlisted step lists with
//! explicit per-recipe permissions. Validation of step parameters happens in
//! the app before a run; storage checks structure and limits.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const STEP_KINDS: &[&str] = &[
    "install_apk",
    "uninstall_package",
    "launch_app",
    "wait",
    "device_shell",
    "screenshot",
    "flutter_deploy",
];
pub const MAX_STEPS: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeStep {
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecipePermissions {
    /// install_apk, uninstall_package, flutter_deploy.
    #[serde(default)]
    pub install: bool,
    /// device_shell.
    #[serde(default)]
    pub shell: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeRecord {
    pub id: String,
    pub name: String,
    pub target_route_id: Option<String>,
    pub steps: Vec<RecipeStep>,
    pub permissions: RecipePermissions,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecipeSave {
    pub id: Option<String>,
    pub name: String,
    pub target_route_id: Option<String>,
    pub steps: Vec<RecipeStep>,
    pub permissions: RecipePermissions,
}

impl Db {
    pub fn list_recipes(&self) -> Result<Vec<RecipeRecord>> {
        let ids: Vec<String> = {
            let mut st = self
                .conn
                .prepare("SELECT id FROM recipes ORDER BY name COLLATE NOCASE")?;
            let v = st
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            v
        };
        ids.iter().map(|id| self.get_recipe(id)).collect()
    }

    pub fn get_recipe(&self, id: &str) -> Result<RecipeRecord> {
        let row: Option<(String, Option<String>, String, String, String)> = self
            .conn
            .query_row(
                "SELECT name, target_route_id, steps_json, permissions_json, updated_at FROM recipes WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (name, target_route_id, steps, perms, updated_at) =
            row.ok_or_else(|| StorageError::NotFound(format!("recipe {id}")))?;
        Ok(RecipeRecord {
            id: id.to_string(),
            name,
            target_route_id,
            steps: serde_json::from_str(&steps).unwrap_or_default(),
            permissions: serde_json::from_str(&perms).unwrap_or_default(),
            updated_at,
        })
    }

    pub fn save_recipe(&mut self, r: &RecipeSave) -> Result<RecipeRecord> {
        let name = r.name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err(StorageError::Invalid(
                "recipe name must be 1-80 characters".into(),
            ));
        }
        if r.steps.is_empty() || r.steps.len() > MAX_STEPS {
            return Err(StorageError::Invalid(format!(
                "a recipe needs 1-{MAX_STEPS} steps"
            )));
        }
        if let Some(s) = r
            .steps
            .iter()
            .find(|s| !STEP_KINDS.contains(&s.kind.as_str()))
        {
            return Err(StorageError::Invalid(format!(
                "unknown step kind {}",
                s.kind
            )));
        }
        let steps =
            serde_json::to_string(&r.steps).map_err(|e| StorageError::Invalid(e.to_string()))?;
        let perms = serde_json::to_string(&r.permissions)
            .map_err(|e| StorageError::Invalid(e.to_string()))?;
        let id = match &r.id {
            Some(id) => {
                let n = self.conn.execute(
                    "UPDATE recipes SET name=?2, target_route_id=?3, steps_json=?4, permissions_json=?5,
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                    params![id, name, r.target_route_id, steps, perms],
                )?;
                if n == 0 {
                    return Err(StorageError::NotFound(format!("recipe {id}")));
                }
                id.clone()
            }
            None => {
                let id = new_id("recipe");
                self.conn.execute(
                    "INSERT INTO recipes(id, name, target_route_id, steps_json, permissions_json) VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![id, name, r.target_route_id, steps, perms],
                )?;
                id
            }
        };
        self.get_recipe(&id)
    }

    pub fn delete_recipe(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM recipes WHERE id=?1", [id])?;
        Ok(())
    }

    /// Link a workflow run to the recipe that started it.
    pub fn set_run_recipe(&self, run_id: &str, recipe_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workflow_runs SET recipe_id=?2 WHERE id=?1",
            params![run_id, recipe_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(id: Option<String>) -> RecipeSave {
        RecipeSave {
            id,
            name: "Smoke test".into(),
            target_route_id: Some("route-1".into()),
            steps: vec![
                RecipeStep {
                    kind: "launch_app".into(),
                    params: json!({"package": "dev.x.app"}),
                },
                RecipeStep {
                    kind: "wait".into(),
                    params: json!({"seconds": 2}),
                },
            ],
            permissions: RecipePermissions {
                install: false,
                shell: true,
            },
        }
    }

    #[test]
    fn save_update_list_delete() {
        let mut db = Db::open_in_memory().unwrap();
        let r = db.save_recipe(&input(None)).unwrap();
        assert_eq!(r.steps.len(), 2);
        assert!(r.permissions.shell && !r.permissions.install);
        let r2 = db
            .save_recipe(&RecipeSave {
                name: "Renamed".into(),
                ..input(Some(r.id.clone()))
            })
            .unwrap();
        assert_eq!(r2.id, r.id);
        assert_eq!(db.list_recipes().unwrap()[0].name, "Renamed");
        db.delete_recipe(&r.id).unwrap();
        assert!(db.list_recipes().unwrap().is_empty());
    }

    #[test]
    fn rejects_unknown_steps_and_limits() {
        let mut db = Db::open_in_memory().unwrap();
        let mut bad = input(None);
        bad.steps.push(RecipeStep {
            kind: "format_disk".into(),
            params: json!({}),
        });
        assert!(db.save_recipe(&bad).is_err());
        let empty = RecipeSave {
            steps: vec![],
            ..input(None)
        };
        assert!(db.save_recipe(&empty).is_err());
        let many = RecipeSave {
            steps: vec![
                RecipeStep {
                    kind: "wait".into(),
                    params: json!({"seconds": 1})
                };
                21
            ],
            ..input(None)
        };
        assert!(db.save_recipe(&many).is_err());
        assert!(db
            .save_recipe(&input(Some("recipe-missing".into())))
            .is_err());
    }
}
