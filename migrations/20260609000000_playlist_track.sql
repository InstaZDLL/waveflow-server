-- =============================================================================
-- Phase 1.j.a — server-side `playlist_track` materialisation.
--
-- The desktop already emits sync ops on the wire shape
-- `entity: "playlist", field: "tracks", op: ("insert" | "delete" | "set")`
-- with a payload of `{ "track_ids": [N, …] }` (insert/delete) or
-- `{ "track_id": N, "position": M }` (set/reorder). Before this
-- migration the server accepted those ops into `sync_op` but had no
-- entity table to materialise them into, so `/api/v1/share/playlists/{token}`
-- always returned `tracks: []` regardless of the playlist's actual
-- content.
--
-- Shape mirrors the desktop SQLite mirror at
-- `src-tauri/migrations/profile/20260411120000_initial.sql:236` —
-- `(playlist_id, track_id)` PK + position index — so a future
-- `waveflow-core::repository::playlist_track` trait satisfies the
-- same shape against either backend (Postgres BIGINT ↔ SQLite
-- INTEGER, epoch-millis BIGINT for timestamps).
--
-- The `track_id` column carries the source desktop's local-i64
-- track id. The server can't resolve it cross-device (a track with
-- id=42 on device A is unrelated to id=42 on device B), so the
-- snapshot columns below carry the displayable values cross-device
-- — desktops on the 1.j.b wire bump populate them. A row with NULL
-- snapshot is still indexable + visible to the owner's other
-- devices on next sync, but is filtered out of the public share
-- preview (the public read has nothing displayable for it).
--
-- A future track-canonical migration can replace `track_id` with a
-- server-resolved foreign key once Phase 1.k plumbs the `track`
-- entity through the apply pipeline.
-- =============================================================================

CREATE TABLE playlist_track (
    playlist_id     BIGINT NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,

    -- Track id AS EMITTED BY THE SOURCE DESKTOP. Local-to-device
    -- BIGINT, NOT a server canonical reference. No FK because the
    -- server has no `track` table yet — see the migration header
    -- for the migration path to a real FK.
    track_id        BIGINT NOT NULL,

    position        INTEGER NOT NULL CHECK (position >= 0),

    added_at        BIGINT NOT NULL,

    -- Snapshot fields populated by desktops that emit the 1.j.b
    -- wire bump (a future PR). Older desktops continue to emit ops
    -- without these; the row is still tracked but invisible from
    -- the public share preview, which filters on
    -- `snapshot_title IS NOT NULL`.
    snapshot_title          TEXT,
    snapshot_artist         TEXT,
    snapshot_duration_ms    BIGINT CHECK (snapshot_duration_ms IS NULL OR snapshot_duration_ms > 0),

    PRIMARY KEY (playlist_id, track_id)
);

-- Mirrors the SQLite index: ordered scans for the share preview
-- and for the owner's listing of a playlist's tracks.
CREATE INDEX idx_playlist_track_position
    ON playlist_track(playlist_id, position);
