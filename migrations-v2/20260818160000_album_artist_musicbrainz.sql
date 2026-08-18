-- MusicBrainz identifiers on the album and artist rows.
--
-- The three MBIDs a file can carry are stored on `track` since the extended
-- tags migration, because RFC-004 needs them for reconciliation. Only the
-- recording id was ever emitted, as `song.musicBrainzId`, since on a media item
-- `musicBrainzId` means the recording. OpenSubsonic also expects one on
-- `AlbumID3` and `ArtistID3`, and there was nowhere to put it: an album row is
-- not a track row, and a release id is a property of the release.
--
-- The column is derived rather than tagged. Nothing writes an album MBID
-- directly; the scanner recomputes it from the album's own tracks at the end of
-- every pass, because tracks of one album routinely disagree and the answer is
-- whatever most of them say. Storing the result instead of resolving it in the
-- projection keeps every album listing a plain column read.
ALTER TABLE album ADD COLUMN musicbrainz_id TEXT;
ALTER TABLE artist ADD COLUMN musicbrainz_id TEXT;
