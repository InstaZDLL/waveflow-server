-- What a scan may be told to ignore, and where an instance remembers its own
-- settings between boots.
--
-- The scanner skips any file whose size, modification time and hashes match the
-- row it already holds. That is the right default and it is why a rescan of a
-- large library costs seconds. It is also unconditional, so nothing can ask for
-- the work to be done again — and a change to how the catalogue derives its
-- identifiers needs exactly that.
--
-- `library.full_scan_requested_at` is the request, and it is cleared only when
-- a scan completes. A scan interrupted halfway therefore resumes as a full scan
-- rather than skipping every file it had already rewritten, which is the only
-- thing standing between an interrupted migration and a catalogue permanently
-- split between two identity schemes.
--
-- `scan_job.full_scan` records what a run actually did, as a column rather than
-- a new `trigger` value: that column's CHECK is closed, and widening it would
-- mean rebuilding the table.
--
-- `server_property` is the instance's key/value memory. `instance_metadata` is
-- not it: that is a single row pinning the encryption key's fingerprint, with a
-- CHECK forbidding a second one.
CREATE TABLE server_property (
    key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(key)) > 0),
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

ALTER TABLE library ADD COLUMN full_scan_requested_at INTEGER;
ALTER TABLE scan_job ADD COLUMN full_scan INTEGER NOT NULL DEFAULT 0 CHECK (full_scan IN (0, 1));
