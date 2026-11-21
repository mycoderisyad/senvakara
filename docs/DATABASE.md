# Database Design — Senvakara

**Status:** migration v1 implemented in `crates/storage/migrations/0001_init.sql` (hosts, host_endpoints, ssh_settings, devices, device_routes, sessions); workspaces/workflows tables are still proposals below. SQLite stores app metadata, **not** plaintext passwords, private keys or Android pairing secrets. OS credential manager/keyring is the designated secret boundary.

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
    recipe_id TEXT, -- future migration adds recipe table / FK
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
- The local Windows machine is a singleton row with `is_local=1` and cannot be deleted.

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
