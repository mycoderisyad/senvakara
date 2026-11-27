//! Workflow run history (FLOW-03): which build/deploy ran, against which
//! target, how each step ended. Only summaries are stored.

use crate::{new_id, Db, Result, StorageError};
use rusqlite::params;
use serde::Serialize;

pub const RUN_STATES: &[&str] = &["running", "succeeded", "failed", "cancelled"];
pub const STEP_STATES: &[&str] = &[
    "pending",
    "running",
    "succeeded",
    "failed",
    "skipped",
    "cancelled",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepRecord {
    pub ordinal: i64,
    pub action_kind: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub result: serde_json::Value,
    /// Workflow node the step ran (schema v6); `None` for older runs.
    pub node_id: Option<String>,
    pub exit_code: Option<i64>,
    /// Whether the step changed anything; `None` when unknown.
    pub changed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub recipe_id: Option<String>,
    pub workflow_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub summary: serde_json::Value,
    pub steps: Vec<StepRecord>,
}

impl Db {
    /// Start a run with its planned steps (all `pending`).
    pub fn create_run(&mut self, summary: &serde_json::Value, steps: &[&str]) -> Result<String> {
        let id = new_id("run");
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO workflow_runs(id, status, summary_json) VALUES(?1, 'running', ?2)",
            params![id, summary.to_string()],
        )?;
        for (i, kind) in steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO workflow_steps(id, run_id, ordinal, action_kind, status) VALUES(?1, ?2, ?3, ?4, 'pending')",
                params![new_id("step"), id, i as i64, kind],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Start a run of a saved workflow; one `pending` step per node, in plan
    /// order, given as (node id, module id).
    pub fn create_workflow_run(
        &mut self,
        workflow_id: &str,
        summary: &serde_json::Value,
        steps: &[(&str, &str)],
    ) -> Result<String> {
        let id = new_id("run");
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO workflow_runs(id, workflow_id, status, summary_json) VALUES(?1, ?2, 'running', ?3)",
            params![id, workflow_id, summary.to_string()],
        )?;
        for (i, (node, module)) in steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO workflow_steps(id, run_id, ordinal, action_kind, status, node_id)                  VALUES(?1, ?2, ?3, ?4, 'pending', ?5)",
                params![new_id("step"), id, i as i64, module, node],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Records what a step's script reported.
    pub fn set_step_outcome(
        &self,
        run_id: &str,
        ordinal: usize,
        exit_code: Option<i64>,
        changed: Option<bool>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE workflow_steps SET exit_code=?3, changed=?4 WHERE run_id=?1 AND ordinal=?2",
            params![run_id, ordinal as i64, exit_code, changed],
        )?;
        Ok(())
    }

    pub fn set_step(
        &self,
        run_id: &str,
        ordinal: usize,
        status: &str,
        result: Option<&serde_json::Value>,
    ) -> Result<()> {
        if !STEP_STATES.contains(&status) {
            return Err(StorageError::Invalid(format!(
                "unknown step status {status}"
            )));
        }
        let col = if status == "running" {
            "started_at"
        } else {
            "finished_at"
        };
        self.conn.execute(
            &format!(
                "UPDATE workflow_steps SET status=?3, {col}=strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
                 safe_result_json=COALESCE(?4, safe_result_json) WHERE run_id=?1 AND ordinal=?2"
            ),
            params![
                run_id,
                ordinal as i64,
                status,
                result.map(|r| r.to_string())
            ],
        )?;
        Ok(())
    }

    pub fn finish_run(
        &self,
        run_id: &str,
        status: &str,
        summary: &serde_json::Value,
    ) -> Result<()> {
        if !RUN_STATES.contains(&status) || status == "running" {
            return Err(StorageError::Invalid(format!(
                "invalid final status {status}"
            )));
        }
        // Steps that never started are recorded as skipped.
        self.conn.execute(
            "UPDATE workflow_steps SET status='skipped' WHERE run_id=?1 AND status='pending'",
            [run_id],
        )?;
        self.conn.execute(
            "UPDATE workflow_runs SET status=?2, summary_json=?3, finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            params![run_id, status, summary.to_string()],
        )?;
        Ok(())
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, status, created_at, finished_at, summary_json, recipe_id, workflow_id              FROM workflow_runs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let runs = st
            .query_map([limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::new();
        for (id, status, created_at, finished_at, summary, recipe_id, workflow_id) in runs {
            let mut ss = self.conn.prepare(
                "SELECT ordinal, action_kind, status, started_at, finished_at, safe_result_json, \
                 node_id, exit_code, changed FROM workflow_steps WHERE run_id=?1 ORDER BY ordinal",
            )?;
            let steps = ss
                .query_map([&id], |r| {
                    let res: Option<String> = r.get(5)?;
                    Ok(StepRecord {
                        ordinal: r.get(0)?,
                        action_kind: r.get(1)?,
                        status: r.get(2)?,
                        started_at: r.get(3)?,
                        finished_at: r.get(4)?,
                        result: res
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(serde_json::Value::Null),
                        node_id: r.get(6)?,
                        exit_code: r.get(7)?,
                        changed: r.get(8)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push(RunRecord {
                id,
                recipe_id,
                workflow_id,
                status,
                created_at,
                finished_at,
                summary: serde_json::from_str(&summary).unwrap_or_default(),
                steps,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::Db;
    use serde_json::json;

    #[test]
    fn run_lifecycle_and_skipped_steps() {
        let mut db = Db::open_in_memory().unwrap();
        let id = db
            .create_run(
                &json!({"target": "Redmi 5A"}),
                &["validate", "build", "install"],
            )
            .unwrap();
        db.set_step(&id, 0, "running", None).unwrap();
        db.set_step(&id, 0, "succeeded", Some(&json!({"flutter": "3.47.9"})))
            .unwrap();
        db.set_step(&id, 1, "running", None).unwrap();
        db.set_step(&id, 1, "failed", Some(&json!({"error": "gradle"})))
            .unwrap();
        db.finish_run(
            &id,
            "failed",
            &json!({"target": "Redmi 5A", "error": "build failed"}),
        )
        .unwrap();
        let runs = db.list_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.status, "failed");
        assert_eq!(
            r.steps
                .iter()
                .map(|s| s.status.as_str())
                .collect::<Vec<_>>(),
            ["succeeded", "failed", "skipped"]
        );
        assert_eq!(r.steps[0].result["flutter"], "3.47.9");
        assert!(r.steps[1].started_at.is_some() && r.steps[1].finished_at.is_some());
    }

    #[test]
    fn rejects_unknown_states() {
        let mut db = Db::open_in_memory().unwrap();
        let id = db.create_run(&json!({}), &["a"]).unwrap();
        assert!(db.set_step(&id, 0, "exploded", None).is_err());
        assert!(db.finish_run(&id, "running", &json!({})).is_err());
    }

    #[test]
    fn workflow_steps_remember_their_node_and_outcome() {
        let mut db = Db::open_in_memory().unwrap();
        let wf = db
            .save_workflow(&crate::WorkflowSave {
                id: None,
                name: "x".into(),
                graph: json!({}),
            })
            .unwrap();
        let id = db
            .create_workflow_run(&wf.id, &json!({}), &[("n1", "a.b"), ("n2", "c.d")])
            .unwrap();
        db.set_step(&id, 0, "succeeded", None).unwrap();
        db.set_step_outcome(&id, 0, Some(0), Some(false)).unwrap();
        db.finish_run(&id, "succeeded", &json!({})).unwrap();
        let run = &db.list_runs(1).unwrap()[0];
        assert_eq!(run.workflow_id.as_deref(), Some(wf.id.as_str()));
        let s = &run.steps[0];
        assert_eq!(
            (s.node_id.as_deref(), s.exit_code, s.changed),
            (Some("n1"), Some(0), Some(false))
        );
        assert_eq!(run.steps[1].action_kind, "c.d");
        assert_eq!(run.steps[1].status, "skipped");
    }
}
