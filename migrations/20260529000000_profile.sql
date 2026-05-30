-- Profile table — mirror of the desktop `profile` row in `app.db`.
-- A profile is a sandboxed library: its own playlists, its own
-- listening history, its own scrobbler credentials. The desktop app
-- exposes a Netflix-style selector that switches between them.
--
-- Schema parity with the SQLite version that ships in the desktop
-- repo's `src-tauri/migrations/app/` ensures `PostgresProfileRepository`
-- (in `waveflow-core`) and `SqliteProfileRepository` can satisfy the
-- same `ProfileRepository` trait against identical-shaped rows.

CREATE TABLE profile (
    -- BIGSERIAL keeps the column an i64 on the wire — matches the
    -- desktop SQLite `INTEGER PRIMARY KEY AUTOINCREMENT` column and
    -- the `Profile.id: i64` Rust struct.
    id            BIGSERIAL    PRIMARY KEY,
    name          TEXT         NOT NULL,
    color_id      TEXT         NOT NULL,
    avatar_hash   TEXT,
    -- Resolved at profile-create time; stays an empty string for the
    -- brief window between `insert` and `set_data_dir` that
    -- `ProfileRepository` documents.
    data_dir      TEXT         NOT NULL DEFAULT '',
    -- Epoch milliseconds. Same shape as SQLite for cross-backend code
    -- (chrono's `Utc::now().timestamp_millis()` works either side).
    created_at    BIGINT       NOT NULL,
    last_used_at  BIGINT       NOT NULL
);

-- The `list_all` query orders by `last_used_at DESC`; the index keeps
-- the per-list-call cost flat as the profile count grows.
CREATE INDEX profile_last_used_idx
    ON profile (last_used_at DESC);
