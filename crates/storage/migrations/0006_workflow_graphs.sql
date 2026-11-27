-- Schema v6: Workflows canvas (plan WF, ADR-028). A workflow is a graph the
-- user draws; storage keeps it as JSON and crates/workflow checks it. Runs
-- and steps gain the workflow and node they belong to. Recipes (v4) stay
-- readable until every one has been converted.
CREATE TABLE workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    graph_json TEXT NOT NULL DEFAULT '{}',
    -- Set when the workflow was converted from a recipe, so conversion runs
    -- once per recipe.
    from_recipe_id TEXT UNIQUE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

ALTER TABLE workflow_runs ADD COLUMN workflow_id TEXT REFERENCES workflows(id) ON DELETE SET NULL;
ALTER TABLE workflow_steps ADD COLUMN node_id TEXT;
-- Exit status of the node's last script, when it ran one.
ALTER TABLE workflow_steps ADD COLUMN exit_code INTEGER;
-- 1 = the node changed something, 0 = it was already up to date.
ALTER TABLE workflow_steps ADD COLUMN changed INTEGER;

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs(workflow_id, created_at);
