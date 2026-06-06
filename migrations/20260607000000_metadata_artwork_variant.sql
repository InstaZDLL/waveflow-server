-- =============================================================================
-- Phase 1.h.3 — resized variants of the shared artwork cache.
--
-- Every upload that lands in `metadata_artwork` triggers two
-- synchronous re-encodes:
-- - `thumb` (≤ 128px on the long edge) for sidebars + list views
-- - `preview` (≤ 480px) for popovers + mini-cards
--
-- The original ("full") is kept in `metadata_artwork` byte-perfect
-- so a future high-DPI consumer can fall back to it. Variants are
-- always JPEG q85: covers are opaque, and JPEG buys roughly 60% file
-- size over PNG / WebP-lossless for visually identical output.
--
-- Parent → variant is FK-tracked with `ON DELETE CASCADE` so a
-- future garbage-collection sweep on `metadata_artwork` reclaims
-- the variants in the same transaction. The `variant` column is
-- string-typed so adding a new size later (`hero`, `large`, …)
-- doesn't need an ALTER TYPE.
-- =============================================================================

CREATE TABLE metadata_artwork_variant (
    -- Hash of the ORIGINAL bytes the user uploaded. Maps 1:1 to
    -- `metadata_artwork.hash`.
    parent_hash TEXT NOT NULL REFERENCES metadata_artwork(hash) ON DELETE CASCADE,

    -- Size bucket. Currently `thumb` (≤ 128px) or `preview` (≤ 480px).
    -- Add new values to the CHECK list when a new size ships.
    variant     TEXT NOT NULL CHECK (variant IN ('thumb', 'preview')),

    -- BLAKE3 of the re-encoded variant bytes. Same shape as the
    -- parent — 64 lowercase hex chars — because the variant is
    -- stored in object_store under the same `artwork/<hash>` key
    -- shape (the GET handler treats the variant hash as a
    -- first-class object reference; clients that already cache the
    -- variant hash hit the bare `GET /api/v1/artwork/{hash}` route
    -- without paying the parent-lookup detour).
    hash        TEXT NOT NULL CHECK (
        char_length(hash) = 64
        AND hash ~ '^[0-9a-f]{64}$'
    ),

    -- Always 'image/jpeg' today; mirrors the future-proofing on
    -- the parent table (a future WebP encoder would extend the
    -- CHECK list without a schema break).
    mime        TEXT NOT NULL CHECK (mime IN ('image/jpeg')),

    -- Cached so the GET handler can vend Content-Length without a
    -- backend HEAD.
    byte_size   BIGINT NOT NULL CHECK (byte_size > 0),

    -- Output dimensions (after the aspect-preserving resize). Useful
    -- for the client to pre-size the layout slot without measuring
    -- the bytes — and for picking the right variant for a target
    -- pixel density.
    width       INTEGER NOT NULL CHECK (width > 0),
    height      INTEGER NOT NULL CHECK (height > 0),

    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- (parent, variant) is the natural row identity. A second
    -- thumb for the same parent would imply two different re-encodes
    -- of identical input bytes, which can't happen with a
    -- deterministic resize pipeline.
    PRIMARY KEY (parent_hash, variant)
);

-- The handler resolves a `GET /api/v1/artwork/{hash}` against the
-- variant table too, so a client that cached the variant hash can
-- fetch directly. The PK doesn't cover the `hash` column, hence the
-- secondary index.
CREATE INDEX idx_metadata_artwork_variant_hash ON metadata_artwork_variant(hash);
