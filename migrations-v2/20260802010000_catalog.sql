ALTER TABLE library ADD COLUMN last_scan_started_at INTEGER;
ALTER TABLE library ADD COLUMN last_scan_completed_at INTEGER;

CREATE TABLE artwork (
    hash TEXT PRIMARY KEY NOT NULL CHECK (length(hash) = 64),
    format TEXT NOT NULL,
    source TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE artist (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) > 0),
    artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (library_id, canonical_name),
    UNIQUE (id, library_id)
) STRICT;

CREATE TABLE album (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    canonical_title TEXT NOT NULL CHECK (length(canonical_title) > 0),
    identity_key TEXT NOT NULL,
    album_artist_id TEXT,
    album_artist_name TEXT,
    is_compilation INTEGER NOT NULL DEFAULT 0 CHECK (is_compilation IN (0, 1)),
    year INTEGER,
    artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (library_id, identity_key),
    UNIQUE (id, library_id),
    FOREIGN KEY (album_artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE genre (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) > 0),
    created_at INTEGER NOT NULL,
    UNIQUE (library_id, canonical_name),
    UNIQUE (id, library_id)
) STRICT;

CREATE TABLE scan_job (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    requested_by TEXT REFERENCES account(id) ON DELETE SET NULL,
    trigger TEXT NOT NULL CHECK (trigger IN ('manual', 'boot', 'scheduled', 'library_added')),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    total_files INTEGER NOT NULL DEFAULT 0,
    processed_files INTEGER NOT NULL DEFAULT 0,
    added INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL DEFAULT 0,
    moved INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    unavailable INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    current_path TEXT,
    message TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER
) STRICT;
CREATE INDEX scan_job_library_idx ON scan_job(library_id, created_at DESC);

CREATE TABLE track (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    album_id TEXT,
    artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL,
    relative_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    file_modified_at INTEGER NOT NULL,
    quick_hash TEXT NOT NULL CHECK (length(quick_hash) = 64),
    full_hash TEXT NOT NULL CHECK (length(full_hash) = 64),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    album_title TEXT,
    artist_display TEXT,
    genre_display TEXT,
    year INTEGER,
    track_number INTEGER,
    disc_number INTEGER,
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    bitrate INTEGER,
    sample_rate INTEGER,
    channels INTEGER,
    bit_depth INTEGER,
    codec TEXT,
    musical_key TEXT,
    tag_rating INTEGER CHECK (tag_rating BETWEEN 0 AND 255),
    is_available INTEGER NOT NULL DEFAULT 1 CHECK (is_available IN (0, 1)),
    last_seen_scan_id TEXT REFERENCES scan_job(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (library_id, relative_path),
    UNIQUE (id, library_id),
    FOREIGN KEY (album_id, library_id)
        REFERENCES album(id, library_id) ON DELETE RESTRICT
) STRICT;
CREATE INDEX track_library_album_idx ON track(library_id, album_id, disc_number, track_number);
CREATE INDEX track_library_hash_idx ON track(library_id, full_hash, is_available);
CREATE INDEX track_library_available_idx ON track(library_id, is_available, updated_at DESC);

CREATE TABLE track_artist (
    track_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (track_id, artist_id),
    UNIQUE (track_id, position),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE CASCADE
) STRICT;

CREATE TABLE track_genre (
    track_id TEXT NOT NULL,
    genre_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    PRIMARY KEY (track_id, genre_id),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (genre_id, library_id)
        REFERENCES genre(id, library_id) ON DELETE CASCADE
) STRICT;

CREATE VIRTUAL TABLE track_fts USING fts5(
    track_id UNINDEXED,
    library_id UNINDEXED,
    title,
    album,
    artists,
    genres,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE scan_error (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES scan_job(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX scan_error_scan_idx ON scan_error(scan_id, id);
