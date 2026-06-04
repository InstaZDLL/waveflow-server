-- Phase 1.g.0 — Server-side apply pipeline for desktop sync ops.
--
-- Until now `sync_op` was append-only: desktop pushed ops, the server
-- stored them, but no consumer materialised them into the entity
-- tables. This migration adds the columns + side-tables the apply
-- pipeline (see `src/apply.rs`) needs to land playlist / library /
-- rating / liked-track ops into queryable rows.
--
-- ## Canonical ids
--
-- Desktop entities have BIGINT ids that are unique only within a
-- per-profile SQLite file. Cross-device stability needs a server-
-- visible identity: a UUID minted on the desktop at insert time and
-- carried in every subsequent op for that entity. This migration adds
-- `canonical_id TEXT` to `profile`, `library`, and `playlist`. The
-- column is nullable so the legacy rows (server-created via the REST
-- API in 1.b.5 / 1.c) keep working; the apply path only writes the
-- column for rows it materialises from sync_ops, so the two creation
-- paths can coexist.
--
-- The `WHERE canonical_id IS NOT NULL` partial UNIQUE is the same
-- pattern already used by `playlist.share_token` in 1.g.1 — it gives
-- O(1) lookup by canonical_id without forbidding multiple legacy
-- NULL rows.
--
-- ## File hash on track
--
-- Tracks have no canonical_id because the audio file content itself
-- is the cross-device identity: same BLAKE3 hash → same track. The
-- desktop already stores `file_hash` per row. Adding it server-side
-- gives the apply path a join key for future track-metadata sync;
-- for now it's a non-UNIQUE index because the same content can be
-- imported into multiple libraries on the same server.
--
-- ## Profile routing
--
-- `sync_op` rows arrive keyed on `user_id` only — the desktop's
-- profile boundary doesn't cross the wire yet. Phase 1.g.0 adds
-- `sync_op.profile_canonical_id` so each op carries the source
-- profile's UUID. The apply pipeline resolves it to a server
-- `profile.id` (auto-provisioning the profile row if it's the first
-- op for that canonical id), so a multi-profile desktop user lands
-- their ops in distinct server profiles instead of collapsing them
-- into one bucket.
--
-- Nullable because legacy ops without it survive — they're
-- explicitly skipped by the apply path.
--
-- ## Rating + liked as free-floating tables
--
-- `user_liked_track` and `user_track_rating` are keyed on
-- `(user_id, file_hash)` rather than on `track.id`. Rationale: the
-- server has no track-metadata sync in Phase 1.g, so a liked / rating
-- op for a file the server has never seen has nowhere to land if
-- we FK to `track`. Decoupling lets the apply path always succeed,
-- and a future track sync can JOIN these tables by `file_hash` to
-- merge the rating back into the `track` row.
--
-- ## playlist_track deferred
--
-- The desktop emits `{ field: "tracks", op: "insert"|"delete"|"set" }`
-- ops carrying local `track_id` BIGINTs that have no meaning on the
-- server (different per-profile id space). Resolving them needs a
-- desktop-side change to emit `file_hash` instead, plus a server
-- track-metadata sync to land tracks. Until both ship, the apply
-- path logs these ops and skips them. The table itself isn't
-- created — adding it before the apply path can populate it would
-- only invite drift.

-- ---------------------------------------------------------------
-- 1. canonical_id columns (profile / library / playlist)
-- ---------------------------------------------------------------

-- All three uniqueness scopes are TENANT-relative rather than
-- global. UUIDs make global collisions astronomically unlikely,
-- but scoping the unique index to the parent tenant keeps the
-- invariant honest: "two users can't see each other's canonical
-- ids" is a property the lookup queries already rely on (a
-- canonical-id lookup always scopes to the resolved tenant).

ALTER TABLE profile ADD COLUMN canonical_id TEXT;
CREATE UNIQUE INDEX idx_profile_user_canonical_id
    ON profile (user_id, canonical_id)
    WHERE canonical_id IS NOT NULL;

ALTER TABLE library ADD COLUMN canonical_id TEXT;
CREATE UNIQUE INDEX idx_library_profile_canonical_id
    ON library (profile_id, canonical_id)
    WHERE canonical_id IS NOT NULL;

ALTER TABLE playlist ADD COLUMN canonical_id TEXT;
CREATE UNIQUE INDEX idx_playlist_profile_canonical_id
    ON playlist (profile_id, canonical_id)
    WHERE canonical_id IS NOT NULL;

-- ---------------------------------------------------------------
-- 2. track.file_hash (non-unique — content can live in many libs)
-- ---------------------------------------------------------------

ALTER TABLE track ADD COLUMN file_hash TEXT;
CREATE INDEX idx_track_file_hash
    ON track (file_hash)
    WHERE file_hash IS NOT NULL;

-- ---------------------------------------------------------------
-- 3. sync_op.profile_canonical_id (routing key for apply)
-- ---------------------------------------------------------------

ALTER TABLE sync_op ADD COLUMN profile_canonical_id TEXT;

-- ---------------------------------------------------------------
-- 4. user_liked_track — file-hash keyed, decoupled from track
-- ---------------------------------------------------------------
--
-- ON DELETE CASCADE via user_id so a user delete fan-outs cleanly.
-- The `(user_id, file_hash)` composite primary key is the natural
-- query shape: "is this file liked by this user?".

CREATE TABLE user_liked_track (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_hash  TEXT   NOT NULL,
    liked_at   BIGINT NOT NULL,
    PRIMARY KEY (user_id, file_hash)
);

-- ---------------------------------------------------------------
-- 5. user_track_rating — file-hash keyed POPM byte
-- ---------------------------------------------------------------
--
-- Raw POPM byte (0-255). Same shape as `track.rating` so a future
-- migration that backfills `track.rating` from this table is a plain
-- INNER JOIN on file_hash. NULL `rating` is expressed by deleting
-- the row, matching the desktop's "clear rating" op.

CREATE TABLE user_track_rating (
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_hash   TEXT   NOT NULL,
    rating      BIGINT NOT NULL CHECK (rating BETWEEN 0 AND 255),
    updated_at  BIGINT NOT NULL,
    PRIMARY KEY (user_id, file_hash)
);
