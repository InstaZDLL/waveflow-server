-- Podcasts (RFC-025 decisions 7 to 14).
--
-- **One list per instance**, as Subsonic has it: an administrator subscribes,
-- every account reads and plays. A channel is fetched once whoever listens to
-- it, which is what keeps the outbound traffic proportional to the channels
-- rather than to the accounts.
--
-- `feed_url` is stored as the administrator typed it, after validation:
-- absolute, http or https, no user information. It is never written to a log,
-- and it is shown only to an account that may manage podcasts — a private
-- feed carries its token in its path as often as in its query string.
--
-- `error_code` is a word from a fixed vocabulary (`src/podcast.rs`), never text
-- out of a response: a feed that answers with a page of HTML must not have that
-- page stored and shown.
CREATE TABLE podcast_channel (
    id TEXT PRIMARY KEY NOT NULL,
    feed_url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT,
    site_url TEXT,
    image_url TEXT,
    status TEXT NOT NULL CHECK (status IN ('completed', 'error')),
    error_code TEXT,
    created_by TEXT REFERENCES account(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    refreshed_at INTEGER
) STRICT;

-- An episode, as the feed described it, and the file the server fetched for it.
--
-- `guid` is the feed's own identifier for the item, or its enclosure address
-- when it gave none; a refresh matches on it, so an episode a feed re-publishes
-- is updated rather than duplicated, and one the server was told to delete
-- stays deleted.
--
-- `file_name` is derived — the episode's own id and a suffix chosen from the
-- bytes — and never taken from the feed or the enclosure address, so nothing a
-- feed says can name a path. `content_hash` is BLAKE3 over the stored file, the
-- key the transcode cache files a conversion under.
CREATE TABLE podcast_episode (
    id TEXT PRIMARY KEY NOT NULL,
    channel_id TEXT NOT NULL REFERENCES podcast_channel(id) ON DELETE CASCADE,
    guid TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    published_at INTEGER,
    duration_ms INTEGER,
    enclosure_url TEXT NOT NULL,
    enclosure_type TEXT,
    enclosure_length INTEGER,
    status TEXT NOT NULL
        CHECK (status IN ('new', 'downloading', 'completed', 'error', 'deleted')),
    error_code TEXT,
    file_name TEXT,
    file_size INTEGER,
    content_hash TEXT,
    suffix TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (channel_id, guid),
    CHECK ((status = 'completed') = (file_name IS NOT NULL))
) STRICT;

-- A channel's episodes, newest first; and the newest across every channel.
CREATE INDEX podcast_episode_channel_idx
    ON podcast_episode (channel_id, published_at DESC);
CREATE INDEX podcast_episode_published_idx ON podcast_episode (published_at DESC);

-- Where an account stopped in an episode (RFC-025 decision 13).
--
-- Beside `bookmark` rather than in it: `bookmark.track_id` is a foreign key to
-- `track`, and a bookmark is a synchronization entity whose journal entry the
-- desktop client resolves against tracks it knows. An episode is neither. The
-- surfaces still treat the two as one — `PUT /api/v2/bookmarks/{id}` and
-- Subsonic `createBookmark` take an episode id as readily as a track id.
CREATE TABLE podcast_bookmark (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    episode_id TEXT NOT NULL REFERENCES podcast_episode(id) ON DELETE CASCADE,
    position_ms INTEGER NOT NULL CHECK (position_ms >= 0),
    comment TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, episode_id)
) STRICT;

CREATE INDEX podcast_bookmark_user_changed_idx
    ON podcast_bookmark (user_id, updated_at DESC);
