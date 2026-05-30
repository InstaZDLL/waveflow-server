-- Track table — multi-tenant counterpart of the desktop's `track` row
-- (see `src-tauri/migrations/profile/20260411120000_initial.sql` in
-- the WaveFlow repo). One file on disk = one row. The server's 1.b.5b
-- schema is intentionally thin: just the columns that exist on every
-- audio file independently of the album / artist / artwork tables
-- (which haven't shipped on the server yet — the scanner pipeline
-- lands in a later phase).
--
-- ON DELETE CASCADE on `library_id` so deleting a library fan-outs
-- into every track that lived under it. The cascade chains further
-- back through `library.profile_id` → `profile.user_id`, so a
-- user delete eventually reaches every track row owned by that
-- tenant — verified by the `profile_delete_cascades_to_tracks` test
-- in `tests/tracks.rs`.

CREATE TABLE track (
    id              BIGSERIAL   PRIMARY KEY,
    library_id      BIGINT      NOT NULL
                    REFERENCES library(id) ON DELETE CASCADE,

    -- File identity. `(library_id, file_path)` is unique because a
    -- single library shouldn't have two rows pointing at the same
    -- path on disk; the desktop scanner relies on the same
    -- constraint to deduplicate during re-scans.
    file_path       TEXT        NOT NULL,
    file_size       BIGINT      NOT NULL,

    title           TEXT        NOT NULL,
    duration_ms     BIGINT      NOT NULL,

    -- Tracklist ordering. NULL on free-form rips that have no album.
    track_number    BIGINT,
    disc_number     BIGINT,
    year            BIGINT,

    -- Audio specs. NULL when the codec doesn't expose the value
    -- (e.g. bit_depth for lossy MP3 / AAC). Stored as BIGINT for
    -- cross-backend parity with the desktop's TrackRow shape, even
    -- though SMALLINT would be enough for channels / bit_depth.
    bitrate         BIGINT,
    sample_rate     BIGINT,
    channels        BIGINT,
    bit_depth       BIGINT,
    codec           TEXT,
    musical_key     TEXT,

    -- Epoch milliseconds for the original library import. Drives the
    -- "Recently added" sort. Same shape as every other timestamp in
    -- this repo.
    added_at        BIGINT      NOT NULL,

    -- Raw POPM byte (0-255). BIGINT (not SMALLINT) because the
    -- waveflow-core `TrackRow.rating: Option<i64>` projection
    -- decodes the column directly — sqlx 0.9 refuses the SMALLINT
    -- → i64 narrowing during RETURNING, and an explicit `::bigint`
    -- cast on every read site would be more friction than the
    -- 6-byte storage saving is worth. The CHECK constraint is
    -- defense in depth on top of the `TrackUpdate.rating:
    -- Option<u8>` type-level guarantee from waveflow-core — even
    -- a future handler that bypasses the type can't persist a
    -- value outside the POPM range.
    rating          BIGINT      CHECK (rating BETWEEN 0 AND 255),

    UNIQUE (library_id, file_path)
);

-- The per-library list query orders by `added_at DESC` filtered on
-- `library_id`; the composite index keeps the per-tenant MRU lookup
-- flat as the table grows across libraries. It also serves the
-- equality filter on its leading column for the ON DELETE CASCADE
-- fan-out from `library`, so a `library_id`-only index would be pure
-- write amplification.
CREATE INDEX track_library_added_idx
    ON track (library_id, added_at DESC);
