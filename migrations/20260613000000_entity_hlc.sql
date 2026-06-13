-- RFC-003 Phase A.1.2 — additive HLC + payload_hash columns on every
-- materialised sync entity, plus the `metadata_digest_version`
-- monotonic counter that backs the §metadata_digest_version
-- invariant.
--
-- Phase A is documented as "wire shape additive, no behaviour change"
-- (RFC §Migration plan/Phase A) — so this migration only adds nullable
-- columns + a new empty table. The apply pipeline updates that fill
-- the new columns ship in Phase A.2, and the OR-Set tombstone shape
-- (`add_at_*` / `delete_at_*`) on `playlist_track` + `user_liked_track`
-- arrives in Phase C alongside the activation of the OR-Set semantics
-- (§3). The single (`hlc_wall`, `hlc_logical`, `origin_device_id`) tuple
-- added here represents the row's CURRENT HLC — the last apply that
-- touched it — which is what the Phase A "LWW everywhere" rule needs.
--
-- Column shapes mirror `sync_op`'s A.1.1 migration
-- (`20260612000000_sync_op_hlc.sql`):
--   * `hlc_wall    BIGINT`     — epoch-millis wall component
--   * `hlc_logical INTEGER`    — u32-shaped per-tick counter
--   * `origin_device_id UUID`  — narrowed UUID at this layer because
--     entity tables ARE the canonical identity (see A.1.1 header for
--     the rationale on why `sync_op.device_id` stays TEXT).
--   * `payload_hash BYTEA`     — BLAKE3-256 over the canonical wire
--     form of the entity (§4 digest invariant). NULL on legacy rows;
--     A.2's apply pipeline populates it on every write.
--
-- ## Backfill strategy
--
-- Entity rows materialised before A.1.1 have no `lamport_ts` of their
-- own — the legacy counter lives only on `sync_op`. The cleanest
-- backfill for those rows is `(hlc_wall, hlc_logical) = (0, 0)`:
-- any v2 op with a non-zero wall (the common case once Phase A.4
-- desktops start emitting) strictly outranks the row under §2's
-- total order, so the apply pipeline always treats it as "newer than
-- the materialised state" — exactly the LWW behaviour Phase A.2
-- promises.
--
-- The `NOT NULL DEFAULT 0` shape lets the ALTER COLUMN ADD happen
-- without a separate UPDATE step. `origin_device_id` and
-- `payload_hash` stay nullable: there's no honest pre-A.1.2 value
-- for either (the originating device is unknowable from the
-- materialised row alone, and `payload_hash` is recomputed by the
-- apply path at write time), so a NOT NULL would force a
-- meaningless synthesised value into every legacy row. A.2's apply
-- pipeline tightens these to NOT NULL on a per-entity basis once
-- it's been deployed long enough for every row to have round-
-- tripped through it at least once — the standard "land schema
-- now, tighten in a follow-up migration" two-step.
--
-- ## Scope
--
-- Entities that ship HLC columns now:
--   profile, library, track, playlist, playlist_track,
--   user_liked_track, user_track_rating
--
-- Entities deliberately NOT touched here:
--   * `album` / `artist` / `track_artist` — auto-materialised by
--     the apply pipeline FROM `track` ops. They carry no
--     independent canonical_id on the wire (the apply path
--     groups them per-library by `(canonical_title,
--     album_artist_id)` and `name`). Their HLC effectively
--     piggybacks on the source track row, so a dedicated column
--     here would be dead weight.
--   * `library_folder` — doesn't exist server-side yet. Lands as
--     a first-class entity in Phase C (RFC §Phase C bullet 4).
--   * `metadata_artwork` / `metadata_artwork_variant` — cache
--     tables, not synced state.
--   * `playlist_share_token` — playlist sub-state, not an
--     independent entity. Sync of share state is out of scope
--     for RFC-003.
--
-- ## metadata_digest_version
--
-- The table is keyed `(profile_id, entity, version)` exactly as the
-- RFC §metadata_digest_version code block documents. User-scoped
-- entities (`liked_track`, `track_rating`) need a different routing
-- key — they don't belong to a profile — and the apply pipeline's
-- resolution of that key (per-user pseudo-profile, NULL-distinct
-- partial index, or a sibling `user_metadata_digest_version`) is
-- explicitly deferred to A.2 when the digest endpoint wiring lands.
-- Adding the table now gives that work a target to bump against
-- without a follow-up migration.

-- ---------------------------------------------------------------
-- 1. Entity tables — HLC + payload_hash columns
-- ---------------------------------------------------------------

ALTER TABLE profile
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE library
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE track
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE playlist
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE playlist_track
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE user_liked_track
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

ALTER TABLE user_track_rating
    ADD COLUMN hlc_wall         BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN hlc_logical      INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN origin_device_id UUID,
    ADD COLUMN payload_hash     BYTEA;

-- ---------------------------------------------------------------
-- 2. metadata_digest_version — per-(profile, entity) monotonic
--    counter backing the §metadata_digest_version invariant.
-- ---------------------------------------------------------------
--
-- ON DELETE CASCADE on `profile_id` so a profile delete fan-outs
-- the counter rows alongside the entity rows themselves. No index
-- on `entity` alone — every read scopes to a known `profile_id`
-- so the composite PK is the only access path.

CREATE TABLE metadata_digest_version (
    profile_id BIGINT NOT NULL REFERENCES profile(id) ON DELETE CASCADE,
    entity     TEXT   NOT NULL,
    version    BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY (profile_id, entity)
);
