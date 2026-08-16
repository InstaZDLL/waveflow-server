ALTER TABLE track ADD COLUMN lyrics_hash TEXT CHECK (
    lyrics_hash IS NULL OR length(lyrics_hash) = 64
);

CREATE TABLE track_lyrics (
    track_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    source TEXT NOT NULL CHECK (source IN ('embedded', 'sidecar_lrc', 'sidecar_text')),
    lang TEXT NOT NULL CHECK (length(trim(lang)) > 0),
    synced INTEGER NOT NULL CHECK (synced IN (0, 1)),
    content TEXT NOT NULL CHECK (length(content) > 0),
    PRIMARY KEY (track_id, position),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX track_lyrics_library_track_idx
    ON track_lyrics(library_id, track_id, position);
