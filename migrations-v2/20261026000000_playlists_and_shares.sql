-- RFC-020 decision 1: a playlist, and a share, may hold one track more than
-- once. Both lists were keyed on the pair, so a second copy violated the key and
-- an ordinary edit answered a database failure. They are keyed on the position
-- instead, as the play queue already was (20260809150000).
--
-- The rows are copied as they are: positions were already unique within each
-- list, so the new key cannot refuse one, and the order is the stored order.
CREATE TABLE playlist_track_by_position (
    playlist_id TEXT NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    added_at INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
) STRICT;

INSERT INTO playlist_track_by_position (playlist_id, track_id, position, added_at)
SELECT playlist_id, track_id, position, added_at FROM playlist_track;

DROP TABLE playlist_track;
ALTER TABLE playlist_track_by_position RENAME TO playlist_track;
-- The cascade from `track` reads this table by track; without an index each
-- deleted track was a scan of every playlist entry.
CREATE INDEX playlist_track_track_idx ON playlist_track(track_id);

CREATE TABLE share_track_by_position (
    share_id TEXT NOT NULL REFERENCES share(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (share_id, position)
) STRICT;

INSERT INTO share_track_by_position (share_id, track_id, position)
SELECT share_id, track_id, position FROM share_track;

DROP TABLE share_track;
ALTER TABLE share_track_by_position RENAME TO share_track;
CREATE INDEX share_track_track_idx ON share_track(track_id);

-- RFC-020 decision 2: the public playlists of other accounts are listed to
-- everyone, newest first.
CREATE INDEX playlist_public_idx ON playlist(updated_at DESC) WHERE public = 1;

-- RFC-020 decision 6: how many times a share may be opened. NULL is no cap.
ALTER TABLE share ADD COLUMN max_visits INTEGER CHECK (max_visits IS NULL OR max_visits >= 1);
