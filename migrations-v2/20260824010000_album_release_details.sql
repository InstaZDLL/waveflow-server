-- The album output fields OpenSubsonic names and this server did not answer.
--
-- `originalReleaseDate`, `releaseDate`, `releaseTypes[]` and `recordLabels[]`
-- describe the release rather than the recording, so they sit on the album and
-- are filled the way `year` and `musicbrainz_id` already are: the first track
-- to carry a value writes it, and later tracks do not overwrite it.
--
-- `discTitles[]` cannot be stored that way — it holds one title per disc — so
-- the tag lands on the track and the album derives the list from its available
-- tracks, the way it already derives its genres and its credits.
--
-- Added empty and filled by the next scan, like every other tag column: an
-- instance that never rescans reports the fields supported and unset rather
-- than reporting something wrong.
ALTER TABLE album ADD COLUMN original_release_date TEXT;
ALTER TABLE album ADD COLUMN release_date TEXT;
ALTER TABLE album ADD COLUMN release_types TEXT;
ALTER TABLE album ADD COLUMN record_labels TEXT;
ALTER TABLE track ADD COLUMN disc_subtitle TEXT;
