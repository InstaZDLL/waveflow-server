-- =============================================================================
-- Phase 4.d.0.1 — server-side `album` + `artist` + `track_artist` tables
-- plus a nullable `track.album_id` FK.
--
-- Mirrors the desktop's per-profile schema at
-- `src-tauri/migrations/profile/20260411120000_initial.sql` but
-- scoped per-library on the server, matching `track.library_id`'s
-- ON DELETE CASCADE chain. Same album in two different libraries =
-- two rows; the user can drop a library without orphaning anyone
-- else's album row.
--
-- Wire shape for this migration is schema-only — the sync apply
-- pipeline (4.d.0.2) and REST endpoints (4.d.0.4) will populate
-- the rows in follow-up PRs. The migration ships the empty tables
-- so 4.d.0.2 has a target to upsert into.
--
-- Album grouping policy mirrors the desktop:
-- `(library_id, canonical_title, album_artist_id)` is the natural
-- key. The desktop's "Album Artist tag → is_compilation → primary
-- artist fallback" derivation lives in the apply pipeline (4.d.0.2);
-- the schema only enforces uniqueness once the apply layer has
-- chosen the album_artist_id.
--
-- A NULL `album_artist_id` covers the compilation case (no single
-- artist owns the album). `UNIQUE NULLS NOT DISTINCT` (PG15+, our
-- target is PG17) makes NULL collapse to a single row per
-- (library, title) for the "Various Artists" case rather than
-- letting two compilation rows co-exist with NULL album_artist_id.
--
-- == Cross-library invariant ==
--
-- The per-library scope is enforced at the schema level via
-- composite FKs: every entity-to-entity link carries `library_id`
-- in BOTH columns of the FK, with the parent's `UNIQUE (id,
-- library_id)` as the target. A try to set
-- `album.album_artist_id` to an artist in a different library
-- fails the composite FK; same for `track.album_id` and
-- `track_artist (track_id, artist_id)`. Without these the schema
-- would silently allow inter-library cross-links, which the
-- per-library cascade chain assumes don't exist.
--
-- `ON DELETE SET NULL (col)` is the PG15+ column-level form: when
-- the referenced parent disappears, only the FK column listed gets
-- nulled out — `library_id` itself stays intact, so a track whose
-- album was scrubbed keeps its library membership. Targets PG17,
-- same version gate as `UNIQUE NULLS NOT DISTINCT`.
-- =============================================================================

-- Step 1: existing `track` table needs a composite UNIQUE (id,
-- library_id) so the new tables can use it as a composite FK
-- target. `track.id` is already PK so this is a one-time index
-- add, not a structural change.
ALTER TABLE track ADD CONSTRAINT track_id_library_uniq
    UNIQUE (id, library_id);

CREATE TABLE artist (
    id              BIGSERIAL   PRIMARY KEY,
    library_id      BIGINT      NOT NULL
                    REFERENCES library(id) ON DELETE CASCADE,

    name            TEXT        NOT NULL CHECK (length(name) > 0),

    -- BLAKE3 hex of the artist picture in the shared artwork
    -- cache (Deezer enrichment or local sidecar). NULL until the
    -- artist-picture pipeline ships server-side. Same shape as
    -- `playlist.cover_hash` so a future GC pass can dedupe
    -- against `metadata_artwork` uniformly.
    picture_hash    TEXT,

    created_at      BIGINT      NOT NULL,
    updated_at      BIGINT      NOT NULL,

    -- Per-library uniqueness on `name` matches the desktop's
    -- per-profile uniqueness. Re-emit of the same artist on
    -- sync = ON CONFLICT DO UPDATE (handled by 4.d.0.2's
    -- apply path).
    UNIQUE (library_id, name),

    -- Composite UNIQUE target for the cross-library guards on
    -- `album.album_artist_id` and `track_artist.artist_id`. Lets
    -- the dependent FK enforce that `(artist_id, library_id)`
    -- pairs resolve only to artists in the same library.
    UNIQUE (id, library_id)
);

CREATE TABLE album (
    id                 BIGSERIAL   PRIMARY KEY,
    library_id         BIGINT      NOT NULL
                       REFERENCES library(id) ON DELETE CASCADE,

    -- Display title is the source-of-truth `album.canonical_title`
    -- on the desktop — already normalised for grouping (case-
    -- folded, trimmed) by the scanner. We keep the column name
    -- so the wire shape stays trivially mappable.
    canonical_title    TEXT        NOT NULL CHECK (length(canonical_title) > 0),

    -- Album Artist FK. NULL for compilations (the apply pipeline
    -- sets this to NULL when the source ships `is_compilation`
    -- = true OR when `merge_implicit_compilations` collapses ≥ 3
    -- distinct-artist same-title rows). Composite FK below
    -- enforces same-library scope.
    album_artist_id    BIGINT,

    -- Release year if known. BIGINT for cross-backend parity
    -- with the desktop's `track.year` (which also uses BIGINT)
    -- AND because sqlx 0.9 refuses the SMALLINT → i64 narrowing
    -- on RETURNING — same reason `track.rating` is BIGINT (see
    -- `20260530000003_track.sql:54-61`). A 4-digit year wastes
    -- 6 bytes per row but an explicit `::bigint` cast on every
    -- read site would be louder friction.
    year               BIGINT,

    -- BLAKE3 hex of the album cover. NULL until the artwork
    -- pipeline (phase 1.h) ships the cover-extraction job for
    -- the server side. Same shape as `playlist.cover_hash` so
    -- the `metadata_artwork` cache covers both.
    cover_hash         TEXT,

    -- Sticky compilation flag. Once set by the apply pipeline,
    -- never flips back to false (the desktop's
    -- `merge_implicit_compilations` rule). Defaults to false so
    -- a fresh single-artist album doesn't accidentally flip the
    -- flag.
    is_compilation     BOOLEAN     NOT NULL DEFAULT FALSE,

    created_at         BIGINT      NOT NULL,
    updated_at         BIGINT      NOT NULL,

    -- Natural key. `NULLS NOT DISTINCT` collapses NULL
    -- album_artist_id (compilation case) to a single row per
    -- (library, title) — without it, two "Various Artists"
    -- albums with the same title could co-exist with NULL ids
    -- and the apply-time upsert would have to special-case the
    -- IS NULL match. PG15+ syntax; we target PG17.
    UNIQUE NULLS NOT DISTINCT (library_id, canonical_title, album_artist_id),

    -- Composite UNIQUE target for the cross-library guard on
    -- `track.album_id`. Same shape as `artist`.
    UNIQUE (id, library_id),

    -- Cross-library guard: `album_artist_id` must reference an
    -- artist row in the SAME library. The composite FK forces the
    -- second column (library_id) to match between album and
    -- artist, so an attempt to link an album in library A to an
    -- artist in library B fails. `SET NULL (album_artist_id)`
    -- (PG15+ column-level form) drops only the artist link when
    -- the artist is deleted — `album.library_id` stays intact.
    FOREIGN KEY (album_artist_id, library_id)
        REFERENCES artist (id, library_id)
        ON DELETE SET NULL (album_artist_id)
);

-- Multi-artist join. Position preserves the order the source
-- desktop ships in its multi-artist tag (semicolon-split per
-- the WaveFlow scanner convention — see WaveFlow CLAUDE.md
-- "Multi-artist queries"). Track's "primary artist" is the
-- `position = 0` row; the desktop's `ArtistLink` UI renders
-- every contributor as a separate clickable link.
--
-- The schema does NOT enforce that positions are strictly
-- increasing per track — that's an apply-pipeline contract.
-- Read sites that need a stable order on tied positions MUST
-- order by `(position ASC, artist_id ASC)` so the result set
-- is deterministic even if a future apply path emits two rows
-- at position 0 by mistake. The corresponding rule for the
-- apply pipeline lands in 4.d.0.2 alongside the upsert helper.
--
-- Cascade asymmetry is deliberate: this is a JOIN row with no
-- independent meaning, so it cascades on both sides (track or
-- artist gone → pairing gone). Entity rows (`album`, `track`)
-- use SET NULL on their FK target deletes — losing the album
-- cover or the audio file row would be a user-visible mistake,
-- but losing the (track ↔ artist) pairing is just garbage
-- collection.
--
-- `library_id` is denormalised onto this join so the composite
-- FKs to track + artist can enforce that BOTH ends live in the
-- same library. The apply pipeline (4.d.0.2) derives it from
-- `track.library_id` at upsert time.
CREATE TABLE track_artist (
    track_id     BIGINT     NOT NULL,
    artist_id    BIGINT     NOT NULL,
    library_id   BIGINT     NOT NULL,
    position     INTEGER    NOT NULL CHECK (position >= 0),

    PRIMARY KEY (track_id, artist_id),

    -- Cross-library guards: both composite FKs share the same
    -- `library_id` column on the join side, so the only way to
    -- INSERT a row is for track AND artist to already be in the
    -- referenced library. A pair that straddles two libraries
    -- can't satisfy both FKs at once.
    FOREIGN KEY (track_id, library_id)
        REFERENCES track (id, library_id) ON DELETE CASCADE,
    FOREIGN KEY (artist_id, library_id)
        REFERENCES artist (id, library_id) ON DELETE CASCADE
);

-- Reverse-direction index for the artist drill-down query
-- (`SELECT t.* FROM track t JOIN track_artist ta ON ta.track_id
-- = t.id WHERE ta.artist_id = $1`). The PK already covers the
-- forward direction.
CREATE INDEX track_artist_artist_idx
    ON track_artist (artist_id, track_id);

-- List-query indexes for the future
-- `GET /api/v1/profiles/{p}/libraries/{l}/{albums,artists}`
-- endpoints (4.d.0.4). The per-library UNIQUE constraints
-- already back a leading `library_id` scan, but those serve
-- the natural-key check, not the "Recently updated" sort. Same
-- shape as `library_profile_updated_idx`
-- (`20260530000002_library.sql:42-43`) and
-- `track_library_added_idx` (`20260530000003_track.sql:73-74`)
-- — every entity table in this repo ships its list-query
-- index alongside the table itself, so 4.d.0.4 can wire the
-- endpoint without a follow-up migration.
CREATE INDEX album_library_updated_idx
    ON album (library_id, updated_at DESC);

CREATE INDEX artist_library_updated_idx
    ON artist (library_id, updated_at DESC);

-- Track ↔ album link. Nullable because free-form rips (single
-- mp3s without album metadata) have no album row to point at.
-- Composite FK enforces that the linked album lives in the same
-- library as the track. `ON DELETE SET NULL (album_id)` (PG15+
-- column-level form) drops only the album link when the album is
-- deleted — `track.library_id` stays intact, so the audio file
-- row survives as an orphan discoverable via
-- `WHERE album_id IS NULL`.
ALTER TABLE track ADD COLUMN album_id BIGINT;

ALTER TABLE track ADD CONSTRAINT track_album_fk
    FOREIGN KEY (album_id, library_id)
        REFERENCES album (id, library_id)
        ON DELETE SET NULL (album_id);

-- Album drill-down: `SELECT * FROM track WHERE album_id = $1
-- ORDER BY disc_number, track_number`. Index on `(album_id,
-- disc_number, track_number)` keeps the per-album playlist
-- materialisation sorted without a heap sort.
CREATE INDEX track_album_idx
    ON track (album_id, disc_number, track_number);
