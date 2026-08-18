-- Admit `bookmark` to the sync journal's vocabulary.
--
-- entity_type is CHECK-constrained to an explicit list, which is the journal
-- saying that its vocabulary is part of the contract rather than free text. A
-- new user-data entity therefore has to be admitted deliberately, and this is
-- that decision: bookmarks converge like playlists, favorites, ratings, the
-- queue, scrobbles and shares, instead of existing only in whichever client set
-- them.
--
-- SQLite cannot alter a CHECK, so the table is rebuilt. Nothing references
-- sync_event by foreign key, so the drop is local to this table; sync_event's
-- own reference to sync_operation is recreated with it.
CREATE TABLE sync_event_next (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    origin_device_id TEXT REFERENCES device(id) ON DELETE SET NULL,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'playlist', 'favorite', 'rating', 'scrobble', 'queue', 'share', 'bookmark'
        )
    ),
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('upsert', 'delete', 'append')),
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
    changed_at INTEGER NOT NULL,
    UNIQUE (user_id, operation_id),
    FOREIGN KEY (user_id, operation_id)
        REFERENCES sync_operation(user_id, operation_id) ON DELETE CASCADE
) STRICT;

-- Cursors are copied verbatim. They are the client's resume point: renumbering
-- them would make every connected client's stored cursor mean something else,
-- and a client resuming from a cursor below the floor re-snapshots for nothing.
INSERT INTO sync_event_next
    (cursor, event_id, user_id, operation_id, origin_device_id, entity_type,
     entity_id, action, payload_json, changed_at)
SELECT cursor, event_id, user_id, operation_id, origin_device_id, entity_type,
       entity_id, action, payload_json, changed_at
FROM sync_event;

DROP TABLE sync_event;

ALTER TABLE sync_event_next RENAME TO sync_event;

CREATE INDEX sync_event_user_cursor_idx ON sync_event (user_id, cursor);
