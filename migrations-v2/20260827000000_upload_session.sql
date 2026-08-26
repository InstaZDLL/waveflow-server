-- The two locks that decide whether the server will receive a file, and the
-- sessions it opens once it has. RFC-008 carries the reasoning; what follows is
-- the shape.
--
-- Receiving a file is not another route: it spends the owner's disk, and it
-- does so permanently. The role says who — the same pair that may already spend
-- that disk on a rescan — and this flag says where. False by default, because a
-- server that has only been upgraded must not have become a deposit.
ALTER TABLE library ADD COLUMN accepts_uploads INTEGER NOT NULL DEFAULT 0;

-- What a negotiation opens and a transfer resumes.
--
-- `declared_hash` and `declared_size` are what the client claimed, never what
-- the server believes: both are recomputed from the bytes at commit, and the
-- names say so on purpose.
CREATE TABLE upload_session (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    declared_hash TEXT NOT NULL CHECK (length(declared_hash) = 64),
    declared_size INTEGER NOT NULL CHECK (declared_size > 0),
    extension TEXT NOT NULL,
    received_bytes INTEGER NOT NULL DEFAULT 0 CHECK (received_bytes >= 0),
    next_chunk INTEGER NOT NULL DEFAULT 0 CHECK (next_chunk >= 0),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;

-- A session is found rather than duplicated, and this index is what makes that
-- structural instead of a matter of remembering to look first. A client that
-- restarts mid-transfer re-offers the same file; without the constraint it
-- would open a second session, strand the first one's staging area, and — since
-- an open session reserves quota — immobilise space nothing ever returns.
CREATE UNIQUE INDEX upload_session_claim_idx
    ON upload_session(library_id, user_id, declared_hash);

-- Expiry is swept on the write path, so the sweep wants an index rather than a
-- scan of every open session.
CREATE INDEX upload_session_expiry_idx ON upload_session(expires_at);
