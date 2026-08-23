-- Who is credited on a file, and in what capacity.
--
-- The catalogue held one relation, `track_artist`, meaning "the artists of
-- this track", ordered by a single position. That shape cannot answer what
-- OpenSubsonic asks: an album credited to two artists hung off the first of
-- them alone, so browsing to the second found nothing, and `contributors[]`,
-- `displayComposer` and `ArtistID3.roles[]` were declared unsupported because
-- the columns they need did not exist. These are those columns.
--
-- Three things change shape, and all three need the table rebuilt rather than
-- altered: SQLite cannot drop a column that carries an index, and a UNIQUE
-- written inside CREATE TABLE leaves an implicit index that DROP INDEX will
-- not touch.
--
-- `defer_foreign_keys` is what makes the rebuilds legal here. `foreign_keys`
-- is a no-op inside a transaction and every migration runs in one, so
-- enforcement is postponed to the commit instead — by which point every
-- parent is back under its own name with its rows intact.
PRAGMA defer_foreign_keys = ON;

-- The credits are carried out of the way first.
--
-- SQLite performs an implicit `DELETE FROM` before dropping a table when
-- foreign keys are on, and that delete fires `ON DELETE CASCADE` on every
-- child. `track_artist` cascades off `artist`, so the rebuild below would
-- empty the very relation the new one is copied from — and `defer_foreign_keys`
-- does not help: it defers constraint *checking*, not referential *actions*.
-- A table made by CREATE ... AS SELECT carries no constraints, so it survives
-- the drop and the credits with it.
CREATE TABLE track_artist_carry AS SELECT * FROM track_artist;

-- `artist`: the canonical name stops being an identity.
--
-- Identity moves to a hash of the name as written, folded only for
-- typographic punctuation. That is strictly finer than the canonical form,
-- which maps every non-alphanumeric character to a space — so "AC/DC" and
-- "AC DC" are now two artists and the old UNIQUE would refuse the second.
-- The column stays, indexed, because search still matches on it.
CREATE TABLE artist_rebuilt (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    canonical_name TEXT NOT NULL CHECK (length(canonical_name) > 0),
    artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL,
    musicbrainz_id TEXT,
    sort_name TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (id, library_id)
) STRICT;
INSERT INTO artist_rebuilt (id, library_id, name, canonical_name, artwork_hash,
                            musicbrainz_id, sort_name, created_at, updated_at)
SELECT id, library_id, name, canonical_name, artwork_hash,
       musicbrainz_id, sort_name, created_at, updated_at
  FROM artist;
DROP TABLE artist;
ALTER TABLE artist_rebuilt RENAME TO artist;
CREATE INDEX artist_library_canonical_idx ON artist (library_id, canonical_name);

-- `album`: the identity key disappears, because the identifier is the identity.
--
-- `identity_key` existed to be the album's cross-scan name while `id` was a
-- number drawn at random. It embedded the row id of the first credited artist,
-- which is what made `A; B` and `A; C` collide and needed a special case to
-- tell them apart. The album's id is now derived from the tags themselves, so
-- the second name has nothing left to say.
CREATE TABLE album_rebuilt (
    id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    canonical_title TEXT NOT NULL CHECK (length(canonical_title) > 0),
    album_artist_id TEXT,
    album_artist_name TEXT,
    is_compilation INTEGER NOT NULL DEFAULT 0 CHECK (is_compilation IN (0, 1)),
    year INTEGER,
    artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL,
    musicbrainz_id TEXT,
    sort_name TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (id, library_id),
    FOREIGN KEY (album_artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE RESTRICT
) STRICT;
INSERT INTO album_rebuilt (id, library_id, title, canonical_title, album_artist_id,
                           album_artist_name, is_compilation, year, artwork_hash,
                           musicbrainz_id, sort_name, created_at, updated_at)
SELECT id, library_id, title, canonical_title, album_artist_id,
       album_artist_name, is_compilation, year, artwork_hash,
       musicbrainz_id, sort_name, created_at, updated_at
  FROM album;
DROP TABLE album;
ALTER TABLE album_rebuilt RENAME TO album;
CREATE INDEX album_library_artist_idx ON album (library_id, album_artist_id);

-- `track_artist` becomes `track_participant`, because it no longer means what
-- its name said.
--
-- Its primary key was `(track_id, artist_id)`: one artist, once per track. A
-- producer who also plays on the record needs two rows, so the role and the
-- instrument join the key. `position` becomes per role — one flat ordered list
-- for each — which is what carries tag order without the denormalised JSON
-- column the reference keeps beside its own join tables.
CREATE TABLE track_participant (
    track_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (length(trim(role)) > 0),
    sub_role TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL CHECK (position >= 0),
    sort_name TEXT,
    PRIMARY KEY (track_id, artist_id, role, sub_role),
    UNIQUE (track_id, role, position),
    FOREIGN KEY (track_id, library_id)
        REFERENCES track(id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE CASCADE
) STRICT;
INSERT INTO track_participant (track_id, artist_id, library_id, role, sub_role,
                               position, sort_name)
SELECT track_id, artist_id, library_id, 'artist', '', position, sort_name
  FROM track_artist_carry;
DROP TABLE track_artist;
DROP TABLE track_artist_carry;
-- Leads with the artist so the queries that ask "what has this artist done"
-- get a seek. The track lookups keep the primary key.
CREATE INDEX track_participant_artist_idx
    ON track_participant (artist_id, role, library_id);

-- The album's own credits, derived from its tracks after every scan rather
-- than written per track: a track-by-track write would be a last-writer-wins
-- race between files that disagree.
CREATE TABLE album_participant (
    album_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (length(trim(role)) > 0),
    sub_role TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (album_id, artist_id, role, sub_role),
    UNIQUE (album_id, role, position),
    FOREIGN KEY (album_id, library_id)
        REFERENCES album(id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE CASCADE
) STRICT;
CREATE INDEX album_participant_artist_idx
    ON album_participant (artist_id, role, library_id);

-- What an artist has to show for each capacity they are credited in.
--
-- `albumCount` used to be a correlated count over `album.album_artist_id`,
-- which could only ever see the first credit of an album. Answering it per
-- role is also what makes `ArtistID3.roles[]` answerable at all: the roles an
-- artist appears in are the keys of this table.
CREATE TABLE artist_role_stats (
    artist_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    role TEXT NOT NULL,
    album_count INTEGER NOT NULL DEFAULT 0,
    song_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (artist_id, role),
    FOREIGN KEY (artist_id, library_id)
        REFERENCES artist(id, library_id) ON DELETE CASCADE
) STRICT;

-- Every library rescans in full: the identifiers, the credits and the album
-- relation all change meaning, and no file timestamp can reveal that.
UPDATE library SET full_scan_requested_at = unixepoch() * 1000
 WHERE full_scan_requested_at IS NULL;
