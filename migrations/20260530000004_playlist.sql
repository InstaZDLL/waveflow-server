-- Playlist table — multi-tenant counterpart of the desktop's `playlist`
-- row (see `src-tauri/migrations/profile/20260411120000_initial.sql`
-- in the WaveFlow repo). A playlist belongs directly to a profile —
-- different from `track`, which sits one tier deeper under `library`.
--
-- 1.b.5c ships custom playlists only. Smart playlists (`is_smart = 1`
-- with `smart_rules` JSON) and the playlist_track join still live
-- exclusively on the desktop until later phases port the smart-playlist
-- engine and the tracks-in-playlist routes. The columns are present so
-- the wire shape stays in lockstep with the desktop's `Playlist` DTO;
-- the server-side repo just hardcodes `is_smart = 0`, `smart_rules =
-- NULL` on inserts.
--
-- ON DELETE CASCADE on `profile_id` so a profile delete fan-outs to
-- its playlists. Every playlist must belong to a profile — an
-- orphaned playlist would violate the tenancy chain that
-- `PostgresPlaylistRepository` enforces in `waveflow-core`.

CREATE TABLE playlist (
    id              BIGSERIAL   PRIMARY KEY,
    profile_id      BIGINT      NOT NULL
                    REFERENCES profile(id) ON DELETE CASCADE,
    name            TEXT        NOT NULL,
    description     TEXT,

    -- Brand-defined design-system tokens; defaults mirror the desktop
    -- (`DEFAULT 'violet'` / `DEFAULT 'music'`). The repo writes the
    -- columns explicitly so the server stays authoritative when the
    -- client omits them.
    color_id        TEXT        NOT NULL DEFAULT 'violet',
    icon_id         TEXT        NOT NULL DEFAULT 'music',

    -- Smart-playlist discriminant + rule payload. BIGINT (not
    -- BOOLEAN / SMALLINT) so the column round-trips into
    -- `Playlist.is_smart: i64` from waveflow-core without a
    -- narrowing decode error — same lesson as `track.rating` in
    -- 1.b.5b. Today the server only writes 0 / NULL; the columns
    -- exist for forward parity with the desktop schema.
    is_smart        BIGINT      NOT NULL DEFAULT 0,
    smart_rules     TEXT,

    -- Cover management. `cover_hash` references the shared
    -- `metadata_artwork/<blake3>.jpg` blob (the cache table itself
    -- hasn't been ported to the server yet; the column is here for
    -- forward parity). `cover_is_auto = 1` means the auto-regen
    -- pipeline owns the slot — `0` is reserved for the case where
    -- the user uploaded their own image and the pipeline should
    -- leave the row alone, matching the desktop convention. Default
    -- mirrors the desktop's `DEFAULT 1`.
    cover_hash      TEXT,
    cover_is_auto   BIGINT      NOT NULL DEFAULT 1,

    -- Drag-and-drop sidebar order. `0` is fine as a default — the
    -- desktop already lives with collisions on this column (it
    -- orders by `position ASC, updated_at DESC` so ties resolve on
    -- recency), and the server's `list_for_profile` follows the
    -- same order.
    position        BIGINT      NOT NULL DEFAULT 0,
    created_at      BIGINT      NOT NULL,
    updated_at      BIGINT      NOT NULL
);

-- The per-profile list query orders by `(position ASC, updated_at
-- DESC)` filtered on `profile_id`; the composite index keeps the
-- per-tenant lookup flat as the table grows across profiles. It also
-- serves the equality filter on its leading column for the ON DELETE
-- CASCADE fan-out from `profile`, so a `profile_id`-only index would
-- be pure write amplification.
CREATE INDEX playlist_profile_position_idx
    ON playlist (profile_id, position ASC, updated_at DESC);
