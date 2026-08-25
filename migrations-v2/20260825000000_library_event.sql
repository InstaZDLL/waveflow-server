-- The change feed the library half of the server never had.
--
-- User state converges through `sync_event`; library state had no feed at all,
-- so a client's only way to learn a catalogue moved was to poll it — and a poll
-- that compares counts catches an added track and misses every retag. RFC-007
-- has the reasoning; what follows is the shape.
--
-- Its own sequence, deliberately. `sync_event.cursor` is a single global
-- AUTOINCREMENT filtered per user at read, so sharing it would have a
-- fifty-thousand-track rescan advance the cursor every account's favourites are
-- measured against.
--
-- No `operation_id` and no origin device. Those exist on `sync_event` to make a
-- client's replay idempotent, and nothing here is client-originated yet — a
-- scan writes every row. The metadata write that changes this brings its own
-- migration rather than leaving unused columns in the meantime.
--
-- The vocabulary is CHECK-constrained for the same reason the journal's is: the
-- list is part of the contract, so a new kind is admitted deliberately.
CREATE TABLE library_event (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist')),
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('upsert', 'delete')),
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
    changed_at INTEGER NOT NULL
) STRICT;

-- Every read is "this library, after this cursor", which is exactly this index.
CREATE INDEX library_event_library_cursor_idx ON library_event(library_id, cursor);
