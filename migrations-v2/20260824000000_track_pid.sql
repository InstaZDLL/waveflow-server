-- The relocation hint a re-encoded file leaves behind.
--
-- A scan matches a track by path, then by content hash. A file that is both
-- moved and re-encoded answers neither: the path is gone and the bytes are
-- different, so it lands as a new track and the old row goes unavailable —
-- taking its favourites, ratings, play history and playlist membership with it.
--
-- `pid` is the track spec evaluated over the file's tags. It is a hint, never
-- an identity: track ids stay drawn at random because six tables cascade off
-- them, and a hint that turns out ambiguous simply declines to match. That is
-- why the column is nullable and unconstrained — a boot that finds the track
-- spec changed clears every value in it, because a hint written under one spec
-- must never be matched against a lookup computed under another.
--
-- Added empty and filled by the next scan, like every other derived column.
ALTER TABLE track ADD COLUMN pid TEXT;
CREATE INDEX track_library_pid_idx ON track(library_id, pid);
