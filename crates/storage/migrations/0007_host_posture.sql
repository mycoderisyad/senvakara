-- Schema v7: host security posture (plan WF §5). The result of the last
-- read-only posture check per host, shared by every workflow. Only the
-- summary the check printed is kept: control ids, pass/fail and short
-- evidence, never file contents or command output.
CREATE TABLE host_posture (
    host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
    checked_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    results_json TEXT NOT NULL
);
