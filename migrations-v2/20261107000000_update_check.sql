-- RFC-028: whether this server may ask download.waveflow.app for the newest
-- version, as an administrator answered when WAVEFLOW_UPDATE_CHECK is `ask`.
-- No row means nobody has answered yet, and nothing is sent.
CREATE TABLE instance_update_check (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    decided_by TEXT REFERENCES account(id) ON DELETE SET NULL,
    decided_at INTEGER NOT NULL
) STRICT;
