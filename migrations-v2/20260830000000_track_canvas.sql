-- The short video loop a member attaches to a track. RFC-009.
--
-- Two tables because there are two facts. `canvas` describes a blob and is
-- keyed by its content, so the twelve tracks of an album that share one loop
-- share one row and one file. `track_canvas` says which track points at which
-- blob, and it is the only thing that makes a blob referenced at all.
--
-- The same quartet as `artwork`, minus `source`: a canvas is always given by a
-- human. Nothing in a file produces one, so no scan can ever find one, which is
-- why this sits beside the track like `track_override` rather than as a column
-- on `track` — a column would live in the `ON CONFLICT DO UPDATE SET` list of
-- `apply_catalog_track_in_transaction`, where someone would eventually have to
-- remember not to touch it.
CREATE TABLE canvas (
    hash TEXT PRIMARY KEY NOT NULL CHECK (length(hash) = 64),
    format TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    created_at INTEGER NOT NULL
) STRICT;

-- `library_id` is derived from the track inside the writing transaction and
-- never accepted from the request, which is what `track_override` already does.
-- It is here rather than read through a join because the quota counts per
-- library and would otherwise join `track` on every accounting query.
--
-- `canvas_hash` has no ON DELETE clause, so it restricts: SQLite refuses to
-- delete a `canvas` row while a link still names it. The lifecycle in the
-- service deletes the row only after counting the references to zero, and this
-- constraint is what turns a mistake there into an error rather than a dead
-- link.
CREATE TABLE track_canvas (
    track_id TEXT PRIMARY KEY NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    canvas_hash TEXT NOT NULL REFERENCES canvas(hash),
    created_at INTEGER NOT NULL
) STRICT;

-- Counting the remaining references to a blob is the question the removal path
-- asks every time, and it is the one the quota asks per library.
CREATE INDEX track_canvas_by_hash ON track_canvas(canvas_hash);
CREATE INDEX track_canvas_by_library ON track_canvas(library_id, canvas_hash);
