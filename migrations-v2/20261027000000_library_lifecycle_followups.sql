-- Libraries and scanning: what RFC-021 left open.
--
-- `track.purged_at`: when a purge took a missing track out of sight. A purge
-- no longer deletes at once; the rows stay, hidden from the missing-tracks
-- page as they already are from every read, until the grace window
-- (`WAVEFLOW_PURGE_GRACE_HOURS`) has passed, and can be put back until then.
-- NULL for every track nobody purged, which is every track today.
ALTER TABLE track ADD COLUMN purged_at INTEGER;
CREATE INDEX track_purged_idx ON track(library_id, purged_at) WHERE purged_at IS NOT NULL;

-- `library.watch`: whether the watcher observes this library (decision 6). On
-- for every existing library, as the global switch had it: upgrading must not
-- change which libraries are watched.
ALTER TABLE library ADD COLUMN watch INTEGER NOT NULL DEFAULT 1
    CHECK (watch IN (0, 1));

-- `scan_job.scope_json`: the folders a watcher-triggered scan walks, relative
-- to the library root, as a JSON array; NULL for a scan of the whole library,
-- which is every scan already recorded.
ALTER TABLE scan_job ADD COLUMN scope_json TEXT
    CHECK (scope_json IS NULL OR json_valid(scope_json));
