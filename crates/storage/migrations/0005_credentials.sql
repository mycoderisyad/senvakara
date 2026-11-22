-- Senvakara schema v5: credentials (CRED-01..04, HOST-UI-02, DESK-IMPL-04).
-- Secrets still never live here: every *_ref column is an opaque key into the
-- OS credential store (ADR-009, ADR-025).

-- SSH keys kept in the keychain. The public key and an OpenSSH certificate are
-- public material; the private key and its passphrase are in the OS store.
CREATE TABLE keychain_keys (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    public_key TEXT NOT NULL DEFAULT '',
    certificate TEXT,
    private_ref TEXT,
    passphrase_ref TEXT,
    kind TEXT NOT NULL DEFAULT 'key' CHECK(kind IN ('key','fido2')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- ssh_settings is rebuilt because SQLite cannot drop NOT NULL from
-- private_key_path. Existing hosts keep signing in with their key file.
CREATE TABLE ssh_settings_v5 (
    endpoint_id TEXT PRIMARY KEY REFERENCES host_endpoints(id) ON DELETE CASCADE,
    username TEXT NOT NULL,
    auth_method TEXT NOT NULL DEFAULT 'key_file'
        CHECK(auth_method IN ('key_file','keychain','password','agent')),
    private_key_path TEXT,
    key_id TEXT REFERENCES keychain_keys(id) ON DELETE RESTRICT,
    password_ref TEXT,
    passphrase_ref TEXT,
    startup_snippet TEXT
);

INSERT INTO ssh_settings_v5(endpoint_id, username, auth_method, private_key_path, passphrase_ref)
SELECT endpoint_id, username, 'key_file', private_key_path, passphrase_ref FROM ssh_settings;

DROP TABLE ssh_settings;
ALTER TABLE ssh_settings_v5 RENAME TO ssh_settings;

CREATE INDEX idx_ssh_settings_key ON ssh_settings(key_id);

ALTER TABLE hosts ADD COLUMN protocol TEXT NOT NULL DEFAULT 'ssh'
    CHECK(protocol IN ('ssh','rdp'));
ALTER TABLE hosts ADD COLUMN group_name TEXT;

CREATE TABLE host_tags (
    host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY(host_id, tag)
);

-- RDP hosts are opened with mstsc.exe (ADR-026). `credential_target` names the
-- TERMSRV/<address> entry Senvakara wrote to the OS store, so it can be
-- removed with the host.
CREATE TABLE rdp_settings (
    host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 3389,
    username TEXT NOT NULL DEFAULT '',
    credential_target TEXT
);
