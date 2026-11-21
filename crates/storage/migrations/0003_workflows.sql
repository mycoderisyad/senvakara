-- Schema v3: developer workflow audit trail (FLOW-03). Results stored here
-- are summaries only (no secrets, no full build logs).
CREATE TABLE workflow_runs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    recipe_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','cancelled')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    finished_at TEXT,
    summary_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE workflow_steps (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    action_kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','running','succeeded','failed','skipped','cancelled')),
    started_at TEXT,
    finished_at TEXT,
    safe_result_json TEXT,
    UNIQUE(run_id, ordinal)
);

CREATE INDEX idx_workflow_steps_run ON workflow_steps(run_id, ordinal);
