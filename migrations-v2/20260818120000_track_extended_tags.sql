-- Canonical identifiers and the remaining OpenSubsonic media tags.
--
-- RFC-004 states plainly that "le serveur ne stocke actuellement aucun MBID" and
-- that its MBID branch cannot begin before the canonical identifiers are added
-- to the scanner, the catalogue and the public contract. These columns are that
-- contract. Storing them decides nothing about reconciliation: RFC-004 keeps an
-- MBID match a candidate the user confirms, never an automatic link.
--
-- All three MusicBrainz identifiers live on the track because that is where the
-- tag carries them. They are distinct entities and must not be conflated: the
-- recording identifies the performance, which is what OpenSubsonic exposes as a
-- song's musicBrainzId, while release and artist identify the album and the
-- credited artist.
ALTER TABLE track ADD COLUMN musicbrainz_recording_id TEXT;
ALTER TABLE track ADD COLUMN musicbrainz_release_id TEXT;
ALTER TABLE track ADD COLUMN musicbrainz_artist_id TEXT;

-- Decibel gains and linear peaks, exactly as the tags carry them. REAL rather
-- than a scaled integer: the value is a measurement, and rounding it here would
-- be a decision the player should make, not the catalogue.
ALTER TABLE track ADD COLUMN replay_gain_track_gain REAL;
ALTER TABLE track ADD COLUMN replay_gain_track_peak REAL;
ALTER TABLE track ADD COLUMN replay_gain_album_gain REAL;
ALTER TABLE track ADD COLUMN replay_gain_album_peak REAL;

ALTER TABLE track ADD COLUMN bpm INTEGER;
ALTER TABLE track ADD COLUMN sort_title TEXT;
ALTER TABLE track ADD COLUMN comment TEXT;

-- Multi-valued like artist_display and genre_display, and split the same way.
-- A track can carry several ISRCs, and OpenSubsonic types the field as an array.
ALTER TABLE track ADD COLUMN isrc TEXT;

-- Existing rows keep NULL until the library is rescanned. Nothing reads these
-- as required, so a server that never rescans stays correct: it simply reports
-- the fields empty, which under the OpenSubsonic presence rule means "supported,
-- no value" rather than "unsupported".
