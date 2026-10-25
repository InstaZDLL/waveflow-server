-- RFC-024 decision 4: what LRCLIB said about a track, instance-wide.
--
-- Keyed by provider and a BLAKE3 digest of what was sent (canonical artist,
-- title and album, and the duration in seconds), so the same recording in
-- two libraries is asked about once. Nothing here belongs to an account and
-- nothing records who asked. A lookup that never reached LRCLIB writes no row.
CREATE TABLE lyrics_lookup (
    provider TEXT NOT NULL CHECK (provider IN ('lrclib')),
    lookup_key TEXT NOT NULL CHECK (length(lookup_key) = 64),
    outcome TEXT NOT NULL CHECK (outcome IN ('hit', 'miss', 'instrumental')),
    synced_lyrics TEXT,
    plain_lyrics TEXT,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (provider, lookup_key),
    -- A hit carries words, and nothing else does.
    CHECK (outcome <> 'hit' OR synced_lyrics IS NOT NULL OR plain_lyrics IS NOT NULL),
    CHECK (outcome = 'hit' OR (synced_lyrics IS NULL AND plain_lyrics IS NULL))
) STRICT;
