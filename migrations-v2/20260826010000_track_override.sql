-- Corrections that survive a scan, without a byte of the file being rewritten.
--
-- Writing a track's tags has two obvious routes and both are wrong. Rewriting
-- the file breaks the invariant that makes a scan safe — audio files are read
-- only, and a server that rewrites someone else's collection has to be
-- faultless on every write, forever. Writing the track row instead is erased by
-- the next scan, which applies `title=excluded.title` over it.
--
-- So the correction lives beside the row and the projection merges the two.
-- The scanner never reads this table and never writes it, which is what makes
-- surviving a rescan a property of the shape rather than of remembering to.
--
-- Every column is nullable and NULL means "no correction here, use what the
-- file said". A row exists only while at least one correction does.
--
-- Keyed by track and scoped to a library on purpose: a tag describes the file,
-- not the listener. Two members of one library see the same correction, and
-- only a member who may already spend the owner's disk on a rescan may make
-- one.
--
-- Only the fields that are columns on `track` and nothing else. An artist or a
-- genre is also a row in `track_participant` or `track_genre`, rebuilt by every
-- scan from the file, so correcting the string alone would have a track say two
-- different things until the next scan reconciled them. Album and album artist
-- are further out still: `album_id` is *derived* from them, so changing one
-- moves the track to another album rather than relabelling it.
CREATE TABLE track_override (
    track_id TEXT PRIMARY KEY NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    title TEXT,
    sort_title TEXT,
    year INTEGER,
    track_number INTEGER,
    disc_number INTEGER,
    musicbrainz_recording_id TEXT,
    comment TEXT,
    updated_at INTEGER NOT NULL
) STRICT;
