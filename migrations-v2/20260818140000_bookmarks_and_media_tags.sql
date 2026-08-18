-- Audiobook and long-form playback positions.
--
-- getBookmarks answered an empty container from the beginning, because there was
-- nowhere to keep a position. One row per (account, track): a bookmark is where
-- *you* stopped in a given file, so a second bookmark on the same track would be
-- two answers to one question.
--
-- position_ms is milliseconds like every other timestamp in this schema, even
-- though the Subsonic wire field is also milliseconds and the queue stores the
-- same unit: keeping one unit everywhere is what stops a conversion being
-- forgotten at a boundary.
CREATE TABLE bookmark (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    track_id TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    position_ms INTEGER NOT NULL CHECK (position_ms >= 0),
    comment TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, track_id)
) STRICT;

-- Bookmarks are listed most recently changed first, per account.
CREATE INDEX bookmark_user_changed_idx ON bookmark (user_id, updated_at DESC);

-- The last two OpenSubsonic media fields the scanner did not read.
--
-- moods is multi-valued and stored joined on ';' like artist_display and
-- genre_display, so it splits the same way on the way out. explicit_status holds
-- the normalised vocabulary the specification defines — 'explicit' or 'clean' —
-- rather than the per-format spelling the tag used, because a client compares it
-- against those two words and nothing else.
ALTER TABLE track ADD COLUMN moods TEXT;
ALTER TABLE track ADD COLUMN explicit_status TEXT;
