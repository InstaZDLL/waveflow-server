-- RFC-011 decision 8: the portrait a provider pointed at, once fetched.
--
-- The hash is kept on the provider's cache row as well as on `artist`, so an
-- artist re-identified by a PID spec change is linked again without a second
-- download. `ON DELETE SET NULL` is what makes that safe: the sweeper counts
-- `artist`, `album` and `track` as references and not this column, so an
-- artwork row nobody shows may go, and the cache row then forgets it rather
-- than naming a file that no longer exists. The next refresh fetches it again.
ALTER TABLE metadata_artist
    ADD COLUMN image_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL;
