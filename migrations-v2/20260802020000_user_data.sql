CREATE TABLE playlist (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
    comment TEXT,
    public INTEGER NOT NULL DEFAULT 0 CHECK (public IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX playlist_owner_idx ON playlist(owner_user_id, updated_at DESC);

CREATE TABLE playlist_track (
    playlist_id TEXT NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    added_at INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, track_id),
    UNIQUE (playlist_id, position)
) STRICT;

CREATE TABLE user_star (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist')),
    entity_id TEXT NOT NULL,
    starred_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, entity_type, entity_id)
) STRICT;

CREATE TABLE user_rating (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist')),
    entity_id TEXT NOT NULL,
    rating INTEGER NOT NULL CHECK (rating BETWEEN 0 AND 5),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, entity_type, entity_id)
) STRICT;

CREATE TABLE play_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    submission INTEGER NOT NULL CHECK (submission IN (0, 1)),
    played_at INTEGER NOT NULL
) STRICT;
CREATE INDEX play_event_user_time_idx ON play_event(user_id, played_at DESC);

CREATE TABLE now_playing (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE play_queue (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    current_track_id TEXT REFERENCES track(id) ON DELETE SET NULL,
    position_ms INTEGER NOT NULL DEFAULT 0 CHECK (position_ms >= 0),
    changed_by TEXT,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE play_queue_track (
    user_id TEXT NOT NULL REFERENCES play_queue(user_id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (user_id, track_id),
    UNIQUE (user_id, position)
) STRICT;

CREATE TABLE share (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    token_nonce BLOB NOT NULL CHECK (length(token_nonce) = 12),
    token_ciphertext BLOB NOT NULL,
    description TEXT,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_visited_at INTEGER,
    visit_count INTEGER NOT NULL DEFAULT 0 CHECK (visit_count >= 0)
) STRICT;
CREATE INDEX share_owner_idx ON share(owner_user_id, created_at DESC);

CREATE TABLE share_track (
    share_id TEXT NOT NULL REFERENCES share(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (share_id, track_id),
    UNIQUE (share_id, position)
) STRICT;
