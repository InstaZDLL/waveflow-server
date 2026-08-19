-- `sortName` for AlbumID3 and ArtistID3.
--
-- The scanner already reads the track's own sort title; the album's and the
-- artist's had nowhere to go, so both fields were absent — which under the
-- presence rule said "not supported" rather than "unknown". Now they can say
-- the true thing.
--
-- Added empty and filled by the next scan, like every other tag column: an
-- instance that never rescans reports the field supported and unset rather
-- than reporting something wrong.
ALTER TABLE album ADD COLUMN sort_name TEXT;
ALTER TABLE artist ADD COLUMN sort_name TEXT;
