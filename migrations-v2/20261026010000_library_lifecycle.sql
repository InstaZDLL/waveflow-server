-- Libraries and scanning (RFC-021).
--
-- `library.follow_links`: whether the walk follows symbolic links whose target
-- stays inside the library root (decision 3). Off for every existing library:
-- upgrading must not change what a library reads.
ALTER TABLE library ADD COLUMN follow_links INTEGER NOT NULL DEFAULT 0
    CHECK (follow_links IN (0, 1));

-- `track.unavailable_since`: when a track stopped being available, so the
-- missing-tracks page can say since when and the optional retention can purge
-- by age (decision 4). NULL for every available track. The tracks already
-- unavailable take the time their row was last written, which is when the scan
-- marked them missing unless something touched them since.
ALTER TABLE track ADD COLUMN unavailable_since INTEGER;
UPDATE track SET unavailable_since = updated_at WHERE is_available = 0;

-- `scan_job` gains two counters and two triggers (decision 7). SQLite cannot
-- alter a CHECK, so the table is rebuilt. Migrations run with foreign keys
-- off, so `scan_error.scan_id` and `track.last_seen_scan_id`, which name this
-- table, are neither cascaded nor nulled by the drop; they resolve to the
-- rebuilt table by name, and the boot's foreign_key_check answers for it.
CREATE TABLE scan_job_rebuilt (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    requested_by TEXT REFERENCES account(id) ON DELETE SET NULL,
    trigger TEXT NOT NULL CHECK (
        trigger IN ('manual', 'boot', 'scheduled', 'library_added', 'watch', 'library_moved')
    ),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    total_files INTEGER NOT NULL DEFAULT 0,
    processed_files INTEGER NOT NULL DEFAULT 0,
    added INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL DEFAULT 0,
    moved INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    unavailable INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    current_path TEXT,
    message TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    full_scan INTEGER NOT NULL DEFAULT 0 CHECK (full_scan IN (0, 1)),
    excluded INTEGER NOT NULL DEFAULT 0 CHECK (excluded >= 0),
    skipped_links INTEGER NOT NULL DEFAULT 0 CHECK (skipped_links >= 0)
) STRICT;

INSERT INTO scan_job_rebuilt
    (id, library_id, requested_by, trigger, status, total_files, processed_files,
     added, updated, moved, skipped, unavailable, errors, current_path, message,
     created_at, started_at, completed_at, full_scan)
SELECT id, library_id, requested_by, trigger, status, total_files, processed_files,
       added, updated, moved, skipped, unavailable, errors, current_path, message,
       created_at, started_at, completed_at, full_scan
FROM scan_job;

DROP TABLE scan_job;
ALTER TABLE scan_job_rebuilt RENAME TO scan_job;

CREATE INDEX scan_job_library_idx ON scan_job(library_id, created_at DESC);
