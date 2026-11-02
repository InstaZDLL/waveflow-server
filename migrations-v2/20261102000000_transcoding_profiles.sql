-- Transcoding profiles (RFC-027).
--
-- What one account wants for a client that does not ask for a format or a bit
-- rate. `client` is the `c` of a Subsonic request, compared without ASCII case;
-- the empty string is the account's default, the row a client with no override
-- falls back on and the only one the native API reads.
--
-- `raw` exists for an override that takes one client back to the original, and
-- so carries no bit rate; every other format carries the ceiling it is encoded
-- at, within the bounds a stream request is held to.
--
-- Deliberately *not* in the sync journal, like radio stations and smart
-- playlists: no client replicates this, and the server reads it on the stream.
CREATE TABLE transcoding_profile (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    client TEXT NOT NULL DEFAULT '' COLLATE NOCASE CHECK (length(client) <= 120),
    format TEXT NOT NULL CHECK (format IN ('raw', 'mp3', 'opus', 'aac')),
    bitrate INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, client),
    CHECK (
        (format = 'raw' AND bitrate IS NULL)
        OR (format <> 'raw' AND bitrate BETWEEN 32 AND 512)
    )
) STRICT, WITHOUT ROWID;
