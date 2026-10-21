-- Lyrics uploaded to a track (RFC-012 decision 6).
--
-- A table of its own because `track_lyrics` belongs to the scanner: every
-- scan rebuilds a track's rows from what the file and its sidecars hold, and
-- would erase anything else on the next pass. An upload is a correction, and
-- corrections live beside the file; nothing here is ever written to disk.
--
-- One upload per track and format: uploading again replaces it. `content` is
-- the uploaded file as it came, decoded to UTF-8, so a download gives back
-- what was uploaded rather than a re-serialisation of it.
CREATE TABLE track_lyrics_upload (
    track_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('lrc', 'txt', 'ttml')),
    lang TEXT NOT NULL CHECK (length(trim(lang)) > 0),
    content TEXT NOT NULL CHECK (length(content) > 0),
    uploaded_by TEXT NOT NULL,
    uploaded_at INTEGER NOT NULL,
    PRIMARY KEY (track_id, format),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE
) STRICT;
