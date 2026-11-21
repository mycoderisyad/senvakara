-- Senvakara schema v1. Based on docs/DATABASE.md §2.
-- Never store passwords, private keys, passphrases or ADB pairing secrets here.

CREATE TABLE hosts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    os_family TEXT NOT NULL CHECK(os_family IN ('windows','linux','macos','unknown')),
    is_local INTEGER NOT NULL DEFAULT 0 CHECK(is_local IN (0,1)),
    trust_state TEXT NOT NULL DEFAULT 'unpaired',
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_seen_at TEXT
);

CREATE TABLE host_endpoints (
    id TEXT PRIMARY KEY,
    host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    transport TEXT NOT NULL CHECK(transport IN ('ssh','agent','vpn','local')),
    address TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 0, -- 0 = not applicable (avoids NULL in UNIQUE)
    fingerprint TEXT,                -- verified public host-key fingerprint, e.g. SHA256:...
    priority INTEGER NOT NULL DEFAULT 100,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    UNIQUE(host_id, transport, address, port)
);

-- SSH login settings per endpoint. `passphrase_ref` is an opaque key into the
-- OS credential store, never the passphrase itself.
CREATE TABLE ssh_settings (
    endpoint_id TEXT PRIMARY KEY REFERENCES host_endpoints(id) ON DELETE CASCADE,
    username TEXT NOT NULL,
    private_key_path TEXT NOT NULL,
    passphrase_ref TEXT
);

CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    device_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    manufacturer TEXT,
    model TEXT,
    identity_status TEXT NOT NULL DEFAULT 'provisional',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE device_routes (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    connection_type TEXT NOT NULL CHECK(connection_type IN ('usb_adb','wifi_adb')),
    transport_identifier TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'unavailable',
    last_seen_at TEXT,
    UNIQUE(host_id, connection_type, transport_identifier)
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    host_id TEXT REFERENCES hosts(id) ON DELETE SET NULL,
    device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    route_id TEXT REFERENCES device_routes(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    last_active_at TEXT,
    closed_at TEXT
);

CREATE INDEX idx_device_routes_host ON device_routes(host_id, state);
CREATE INDEX idx_sessions_state ON sessions(state);
