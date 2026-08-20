-- Where the sort tags live so a scan can re-derive from them.
--
-- `album.sort_name` and `artist.sort_name` were written straight from whichever
-- track happened to be applied, under a COALESCE so a sibling file carrying no
-- tag would not erase what another supplied. That preserved a value the files
-- no longer carry: remove ALBUMSORT and rescan, and the old sort name stayed.
--
-- `consolidate_musicbrainz_ids` already answered this question the other way
-- for the identifiers — "a tag removed from the files has to disappear from the
-- catalogue too" — and it can only do that because the identifiers live on the
-- track. The sort tags now do the same.
--
-- The credit's sort form goes on `track_artist` rather than on `track`: a
-- joined ARTISTSORT is paired with its joined ARTIST position by position, and
-- that pairing is done where the names are split, not in SQL.
ALTER TABLE track ADD COLUMN sort_album TEXT;
ALTER TABLE track_artist ADD COLUMN sort_name TEXT;
