-- Library table — multi-tenant counterpart of the desktop's per-profile
-- `library` row (see `src-tauri/migrations/profile/20260411120000_initial.sql`
-- in the WaveFlow repo). On the desktop the profile boundary is the
-- database file itself, so `library` doesn't carry a `profile_id`;
-- here every profile lives in the same Postgres DB so the column is
-- mandatory and the FK threads tenancy down through the resource tree.
--
-- A library is a sandboxed collection of audio folders ("Bandes-son",
-- "Live", "Démos", …). The desktop renders them as sidebar shelves;
-- the web client (1.c) will surface them as the top-level navigation
-- under a profile.
--
-- ON DELETE CASCADE on `profile_id` so a profile delete fan-outs into
-- its libraries (and, transitively, into the future `library_folder` /
-- `track` rows). Every library must belong to a profile — an orphaned
-- library would violate the tenancy chain that `PostgresLibraryRepository`
-- enforces in `waveflow-core`.

CREATE TABLE library (
    id            BIGSERIAL    PRIMARY KEY,
    profile_id    BIGINT       NOT NULL
                  REFERENCES profile(id) ON DELETE CASCADE,
    name          TEXT         NOT NULL,
    description   TEXT,
    -- Brand-defined design-system tokens (default values mirror the
    -- desktop's `DEFAULT 'emerald'` / `DEFAULT 'library'` columns).
    -- Validated client-side; the server just stores the string.
    color_id      TEXT         NOT NULL DEFAULT 'emerald',
    icon_id       TEXT         NOT NULL DEFAULT 'library',
    -- Epoch milliseconds. Same shape as every other timestamp in this
    -- repo so cross-backend code in `waveflow-core` can stay i64-only.
    created_at    BIGINT       NOT NULL,
    updated_at    BIGINT       NOT NULL
);

-- The per-profile list query orders by `updated_at DESC` filtered by
-- `profile_id`; the composite index keeps the per-tenant MRU lookup
-- flat as the table grows across profiles. It also serves the equality
-- filter on its leading column for the ON DELETE CASCADE fan-out from
-- `profile`, so a `profile_id`-only index would be pure write
-- amplification.
CREATE INDEX library_profile_updated_idx
    ON library (profile_id, updated_at DESC);
