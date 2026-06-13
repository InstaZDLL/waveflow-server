-- RFC-003 Phase A.2.2.0 — sibling counter table for user-scoped sync
-- entities (`liked_track`, `track_rating`).
--
-- A.1.2 shipped `metadata_digest_version (profile_id, entity, version)`
-- to back the §metadata_digest_version invariant, but its key is
-- profile-scoped — every entity row keyed on `(profile_id, entity)`
-- bumps its counter, and the digest endpoint reads it per
-- `(profile_id, entity)`. That works cleanly for the profile-scoped
-- entities (profile, library, track, playlist, playlist_track), all
-- of which carry a `profile_id` column.
--
-- The two user-scoped entities don't fit: `user_liked_track` and
-- `user_track_rating` are keyed `(user_id, file_hash)` per the apply
-- pipeline's "rating + liked as free-floating tables" design (see
-- the header on `20260604000000_apply_pipeline.sql`). There's no
-- profile boundary to project onto — the desktop fires likes /
-- ratings against the user as a whole, not a single profile, so a
-- digest endpoint that scopes those entities by profile_id would
-- either bloat (one counter row per profile × entity even though
-- the underlying state is shared) or break the desktop's "I liked
-- this file globally" semantics.
--
-- Two options considered:
--
--   A. Sibling table `user_metadata_digest_version (user_id, entity,
--      version)` — clean separation, query stays a plain composite
--      PK lookup, no NULL gymnastics. One extra table, one extra
--      bump path in the apply pipeline.
--   B. Make `metadata_digest_version.profile_id` nullable, add a
--      sibling `user_id` column, CHECK exactly-one-of, UNIQUE NULLS
--      NOT DISTINCT. One table to maintain, but every consumer site
--      pays the CHECK + branching cost, and the partial-index
--      semantics drift further from the "look up by composite key"
--      shape.
--
-- We pick A. The cost is a second monotone bump call in the apply
-- handlers for liked / rating, but the schema stays diagnosable on
-- inspection (every row carries an unambiguous owner), and the
-- digest endpoint can dispatch on entity name to read from the
-- right table without conditional-on-NULL SQL. Same shape /
-- behaviour as `metadata_digest_version` — monotone counter,
-- `ON CONFLICT DO UPDATE SET version = version + 1`, FK cascades on
-- owner delete — so the apply-pipeline bump helper can stay
-- generic over the two tables.
--
-- Cascade target is `users(id)` rather than `profile(id)` because the
-- entities themselves cascade from `users.id` (see
-- `user_liked_track.user_id REFERENCES users(id) ON DELETE CASCADE`
-- in the apply-pipeline migration). Tearing down a user wipes both
-- the entity rows AND their digest counters in one step.

CREATE TABLE user_metadata_digest_version (
    user_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entity   TEXT   NOT NULL,
    version  BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY (user_id, entity)
);
