CREATE TABLE account (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE instance_metadata (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    key_fingerprint BLOB NOT NULL CHECK (length(key_fingerprint) = 32),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE device (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;
CREATE INDEX device_user_idx ON device(user_id, last_seen_at DESC);

CREATE TABLE session (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    access_token_hash BLOB NOT NULL UNIQUE CHECK (length(access_token_hash) = 32),
    refresh_token_hash BLOB NOT NULL UNIQUE CHECK (length(refresh_token_hash) = 32),
    access_expires_at INTEGER NOT NULL,
    refresh_expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    revoked_at INTEGER,
    CHECK (refresh_expires_at > access_expires_at)
) STRICT;
CREATE INDEX session_user_active_idx
    ON session(user_id, refresh_expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE api_token (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    scopes_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes_json)),
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
) STRICT;
CREATE INDEX api_token_user_idx ON api_token(user_id, created_at DESC);

CREATE TABLE library (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES account(id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 160),
    root_path TEXT NOT NULL UNIQUE,
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'shared')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX library_owner_idx ON library(owner_user_id, created_at);

CREATE TABLE library_member (
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'manager', 'listener')),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (library_id, user_id)
) STRICT;
CREATE INDEX library_member_user_idx ON library_member(user_id, library_id);

CREATE TABLE subsonic_credential (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    password_nonce BLOB NOT NULL CHECK (length(password_nonce) = 12),
    password_ciphertext BLOB NOT NULL,
    api_key_hash BLOB NOT NULL UNIQUE CHECK (length(api_key_hash) = 32),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE audit_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id TEXT REFERENCES account(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    subject_id TEXT,
    occurred_at INTEGER NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(details_json))
) STRICT;
CREATE INDEX audit_event_time_idx ON audit_event(occurred_at DESC, id DESC);
