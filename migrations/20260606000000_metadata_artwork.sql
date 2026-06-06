-- =============================================================================
-- Phase 1.h.1 — shared artwork cache (Postgres mirror of the desktop's
-- on-disk metadata_artwork directory). Identical images uploaded from
-- different tenants dedupe to a single row keyed by the BLAKE3 hash of
-- the bytes, so a million users sharing the same album cover only
-- store one copy.
--
-- This table tracks the MIME type + byte size + first-seen timestamp.
-- The bytes themselves live in object_store (LocalFileSystem in 1.h.1,
-- S3 in 1.h.2) at the key `artwork/<hash>`. Splitting metadata from
-- payload lets the existence check (a GET / HEAD against the table)
-- stay a single Postgres round-trip instead of round-tripping S3.
--
-- No foreign keys: the desktop references this hash from playlist,
-- album, artist tables across two separate databases (app.db +
-- per-profile data.db). We mirror that "soft" linkage server-side by
-- letting playlist.cover_hash / album.cover_hash / artist.picture_hash
-- carry the hash without a referential constraint.
-- =============================================================================

CREATE TABLE metadata_artwork (
    -- BLAKE3 of the raw bytes, hex-encoded. 64 characters; we enforce
    -- the length + alphabet so a malformed value can't slip through
    -- a hand-crafted POST.
    hash        TEXT PRIMARY KEY CHECK (
        char_length(hash) = 64
        AND hash ~ '^[0-9a-f]{64}$'
    ),

    -- Content-Type the client originally uploaded with. We accept
    -- image/jpeg, image/png, image/webp at the application layer;
    -- mirrored as a CHECK so a bypass of the app layer can't pollute
    -- the table with arbitrary strings.
    mime        TEXT NOT NULL CHECK (mime IN ('image/jpeg', 'image/png', 'image/webp')),

    -- Original byte count. Cached so a HEAD response or a listing can
    -- vend Content-Length without round-tripping object_store.
    byte_size   BIGINT NOT NULL CHECK (byte_size > 0),

    -- First-seen timestamp. Not "updated_at" — rows are immutable
    -- (the BLAKE3 hash IS the row identity). Useful for cron-driven
    -- garbage collection of artwork no entity references anymore.
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- No additional indexes: the primary key already covers the only
-- lookup pattern (GET /api/v1/artwork/{hash}) and the table is
-- write-once / read-many / append-only.
