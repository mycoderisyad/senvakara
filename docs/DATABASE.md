# Database Design — Senvakara

**Status:** migration v1 implemented in `crates/storage/migrations/0001_init.sql` (hosts, host_endpoints, ssh_settings, devices, device_routes, sessions); migration v2 in `0002_workspaces.sql` adds `workspaces` and `workspace_panels` (panel targets stored as JSON references, max 4 panels); migration v3 in `0003_workflows.sql` adds `workflow_runs` and `workflow_steps` (status checks, started/finished times, summary JSON only); migration v4 in `0004_recipes.sql` adds `recipes` (name, target route, allowlisted step list and per-recipe permissions as JSON) and runs record their `recipe_id`. SQLite stores app metadata, **not** plaintext passwords, private keys or Android pairing secrets. OS credential manager/keyring is the designated secret boundary.

## 1. Entity model

```text
hosts 1 ── * host_endpoints
hosts 1 ── * device_routes * ── 1 devices
hosts 1 ── * sessions
workspaces 1 ── * workspace_panels
sessions 0..1 ── * workspace_panels
workspaces 1 ── * workflow_runs 1 ── * workflow_steps
```

**Hosts** and **devices** are distinct. An Android can have multiple routes; a route attaches it to the host with ADB access. A device's persistent ID is not its volatile ADB transport ID.

## 2. Initial proposed schema (illustrative SQL)

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE hosts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    os_family TEXT NOT NULL CHECK(os_family IN ('windows','linux','macos','unknown')),
    is_local INTEGER NOT NULL DEFAULT 0 CHECK(is_local IN (0,1)),
    trust_state TEXT NOT NULL DEFAULT 'unpaired',
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_seen_at TEXT
);

CREATE TABLE host_endpoints (
    id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    transport TEXT NOT NULL, -- ssh, agent, vpn, local
    address TEXT NOT NULL,
    port INTEGER,
    fingerprint TEXT, -- non-secret verified public identity; exact representation TBD
    priority INTEGER NOT NULL DEFAULT 100,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    UNIQUE(host_id,transport,address,port)
);

CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    device_type TEXT NOT NULL, -- initially android; extensible
    display_name TEXT NOT NULL,
    manufacturer TEXT,
    model TEXT,
    identity_status TEXT NOT NULL DEFAULT 'provisional',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE device_routes (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    connection_type TEXT NOT NULL, -- usb_adb, wifi_adb
    transport_identifier TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'unavailable',
    last_seen_at TEXT,
    UNIQUE(host_id,connection_type,transport_identifier)
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    host_id TEXT REFERENCES hosts(id) ON DELETE SET NULL,
    device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    route_id TEXT REFERENCES device_routes(id) ON DELETE SET NULL,
    kind TEXT NOT NULL, -- ssh, local_pty, wsl, sftp, rdp, vnc, scrcpy, logcat
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_active_at TEXT,
    closed_at TEXT
);

CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    layout_version INTEGER NOT NULL DEFAULT 1,
    layout_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE workspace_panels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    panel_type TEXT NOT NULL,
    target_ref_json TEXT NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE workflow_runs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    recipe_id TEXT, -- set for runs started from a recipe (v4); no FK so history survives recipe deletion
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    summary_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE workflow_steps (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    action_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    safe_result_json TEXT,
    UNIQUE(run_id,ordinal)
);

CREATE INDEX idx_device_routes_host ON device_routes(host_id,state);
CREATE INDEX idx_sessions_state ON sessions(state);
CREATE INDEX idx_workflow_steps_run ON workflow_steps(run_id,ordinal);
```

The SQL is a **starting model**, not a migration to execute blindly. Validate timestamps, schema/check constraints, unique handling of `NULL` port, lifecycle and relevant indexes before using in production. Workspaces may reference *session descriptors*, not raw process handles, across restarts.

### Implementation notes (v1)

- `host_endpoints.port` is `NOT NULL DEFAULT 0` (0 = not applicable) so the `UNIQUE(host_id,transport,address,port)` constraint is not bypassed by `NULL`.
- `ssh_settings(endpoint_id, username, private_key_path, passphrase_ref)` holds SSH login settings; `passphrase_ref` is an opaque Windows Credential Manager entry name, never the passphrase.
- Changing a host's SSH address or port keeps the host id but clears the pinned fingerprint and sets `trust_state='untrusted'`.
- Each newly observed (host, connection type, serial) route creates its own provisional `devices` row; routes are never merged across hosts or by model. The model replaces a serial placeholder name, but never a user-chosen name. Transport ids are runtime-only.
- **Migration `0005_credentials` (Phase 9, CRED-01..04, HOST-UI-02, DESK-IMPL-04).** Schema version 5; existing rows are kept (covered by the storage test `upgrading_from_v4_keeps_hosts_signing_in_with_their_key_file`). `private_ref` is nullable because a FIDO2 key has no private text. RDP hosts have no `ssh` endpoint, `trust_state='external'` (mstsc checks the server itself), and their protocol cannot change on edit.
  - `keychain_keys(id, label, algorithm, public_key, certificate NULL, private_ref, passphrase_ref NULL, kind CHECK(kind IN ('key','fido2')), created_at, updated_at)`: `private_ref`/`passphrase_ref` are opaque Credential Manager names (the secret may be chunked, ADR-025); public key and certificate are not secret.
  - `ssh_settings` is rebuilt (SQLite cannot drop NOT NULL) as `(…, username, auth_method CHECK(auth_method IN ('key_file','keychain','password','agent')), private_key_path NULL, key_id NULL REFERENCES keychain_keys ON DELETE RESTRICT, password_ref NULL, passphrase_ref NULL, startup_snippet NULL)`; old rows are copied as `key_file`. A key still used by a host cannot be deleted.
  - `hosts` gains `protocol` (`ssh`|`rdp`, default `ssh`) and `group_name NULL`.
  - `host_tags(host_id, tag)`, primary key on both, cascades with the host.
  - `rdp_settings(host_id PK, address, port DEFAULT 3389, username, credential_target NULL)`; `credential_target` is the `TERMSRV/<address>` name (ADR-026).
- The local Windows machine is a singleton row with `is_local=1` and cannot be deleted.
- VPN addresses are `host_endpoints` rows with `transport='vpn'`, `priority=200` (max 4 per host). They carry no fingerprint of their own: connections through them must present the fingerprint pinned on the host's `ssh` endpoint (ADR-022).
- **Migration `0006_workflow_graphs` (Phase 10, WF-IMPL-01, ADR-028).** Schema version 6. New table `workflows` (`id`, `name`, `graph_json`, `from_recipe_id` UNIQUE, timestamps): the Workflows canvas graph as JSON, opaque to storage (name 1–120 characters, graph a JSON object of at most 4 MiB); `crates/workflow` checks it against the bundled module catalog before a run, and drafts with problems may be saved. `workflow_runs` gains `workflow_id` (`ON DELETE SET NULL`, so runs outlive their workflow) and `workflow_steps` gains `node_id`, `exit_code` and `changed`. At start, every recipe without a workflow is converted once into a straight-line workflow on its device (`from_recipe_id` prevents duplicates); recipes are kept until Automations is removed.
- **Migration `0007_host_posture` (Phase 10, WF-IMPL-04, ADR-031).** Schema version 7. New table `host_posture(host_id PK → hosts ON DELETE CASCADE, checked_at, results_json)`: the summary of the last read-only security check of a host (control id, pass/fail/unknown/info, evidence of at most 200 characters, `/etc/os-release` ID and version), a JSON object of at most 64 KiB, replaced by each check and shared by every workflow. No command output or file contents are stored. Moving sshd to a new port (WF-IMPL-03) updates `host_endpoints.port` of the host's `ssh` row and of `vpn` rows on the old port; the pinned fingerprint stays.
- `recipes.steps_json` holds at most 20 steps of the allowlisted kinds `install_apk`, `uninstall_package`, `launch_app`, `wait`, `device_shell`, `screenshot`, `flutter_deploy`; `permissions_json` is `{install, shell}`. Parameters are validated by the app before saving and again (with permissions) before every run. Deleting a recipe keeps its runs.

## 3. Runtime vs persisted state

Runtime live connections, ADB transport handles and stream sockets remain in Rust memory. Database retains preferred routes, host definitions, non-secret UI state and last-observed metadata. Never treat last-seen as current online proof.

## 4. Sensitive data policy

- No raw passwords/private keys/tokens/ADB pairing credentials in SQLite.
- Persist only opaque references to OS keychain entries (future `credential_refs` table if needed).
- Minimize logging of terminal output, APK paths and device identifiers.
- Encrypt backups at rest if user-configured backups are offered later; deletion/migration documented.

## 5. Migration strategy

- Version each migration, test upgrade and rollback/recovery pathway; `PRAGMA user_version` or formal migration manager.
- Transactional migrations, foreign key checks, backup before destructive change.
- Add indexes based on query/profile evidence.
- Never silently merge duplicate device records based on model/name; reconciliation requires verified evidence.

## 6. Data lifecycle

Unregistering host should ask whether to retain associated device metadata/workspace references; schema's cascade behavior is only a starting point and requires confirmation at implementation. Clear stale device_routes on host removal safely. Session history retention and privacy settings are TBD.

## 7. Test cases

- Same Android via USB and Wi-Fi does not silently create unverified merge.
- Two identical-model Android phones stay separate.
- Foreign keys and workspace restore survive restart.
- Unplugged Android route becomes unavailable without deleting user display labels.
- Concurrent writes and upgrades preserve data integrity.
