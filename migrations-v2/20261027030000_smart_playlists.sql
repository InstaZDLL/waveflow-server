-- Smart playlists (RFC-025 decisions 1 to 6).
--
-- A playlist defined by rules rather than by a list of tracks: the server
-- evaluates the rules every time the playlist is read, so it stays current
-- without anybody rescanning or rebuilding it.
--
-- **Its own table, not a flag on `playlist`.** A smart playlist holds no
-- `playlist_track` rows, cannot be appended to or reordered, and is not in the
-- synchronization journal; a flag on the shared table would have every write
-- to `playlist` ask which kind it was holding, and a desktop client mirroring
-- playlists would receive one it cannot edit and does not understand.
--
-- `rules_json` is the canonical serialization of rules the server has already
-- validated against its allowlist (`src/services/smart_playlists.rs`). It is
-- never interpolated into SQL: evaluation binds it as one value, and SQLite's
-- own `json_each` takes it apart inside a fixed statement. The CHECK below is a
-- floor, not the validation — it keeps a row a hand edit damaged from being
-- read as rules at all.
--
-- Deliberately *not* in the sync journal, like radio stations: the
-- `entity_type` vocabulary is CHECK-constrained and admitting one is a contract
-- change the desktop client has to be ready for.
CREATE TABLE smart_playlist (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    comment TEXT,
    public INTEGER NOT NULL DEFAULT 0 CHECK (public IN (0, 1)),
    rules_json TEXT NOT NULL CHECK (json_valid(rules_json)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- An account's own, newest first, the order every listing reads them in.
CREATE INDEX smart_playlist_owner_idx ON smart_playlist (owner_user_id, updated_at DESC);

-- Other accounts' public ones, which every listing that asks for them scans.
CREATE INDEX smart_playlist_public_idx ON smart_playlist (updated_at DESC) WHERE public = 1;
