-- Schema v2: saved workspaces (WORK-01/02). Panels reference *targets*
-- (host/route ids, shell descriptors), never live process handles.
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    layout_version INTEGER NOT NULL DEFAULT 1,
    layout_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE workspace_panels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    panel_type TEXT NOT NULL CHECK(panel_type IN ('terminal','mirror','desktop','files','logcat')),
    target_ref_json TEXT NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL DEFAULT 0,
    UNIQUE(workspace_id, position)
);
