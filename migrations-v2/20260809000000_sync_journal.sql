-- M4 user-data synchronization. The catalogue remains server-authoritative;
-- only mutations owned by an account enter this journal.
CREATE TABLE sync_operation (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    origin_device_id TEXT REFERENCES device(id) ON DELETE SET NULL,
    result_entity_id TEXT,
    event_cursor INTEGER,
    created_at INTEGER NOT NULL,
    applied_at INTEGER,
    PRIMARY KEY (user_id, operation_id)
) STRICT;

CREATE TABLE sync_event (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    origin_device_id TEXT REFERENCES device(id) ON DELETE SET NULL,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN ('playlist', 'favorite', 'rating', 'scrobble', 'queue', 'share')
    ),
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('upsert', 'delete', 'append')),
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
    changed_at INTEGER NOT NULL,
    UNIQUE (user_id, operation_id),
    FOREIGN KEY (user_id, operation_id)
        REFERENCES sync_operation(user_id, operation_id) ON DELETE CASCADE
) STRICT;
CREATE INDEX sync_event_user_cursor_idx ON sync_event(user_id, cursor);

CREATE TABLE sync_ack (
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    cursor INTEGER NOT NULL CHECK (cursor >= 0),
    acknowledged_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, device_id)
) STRICT;
