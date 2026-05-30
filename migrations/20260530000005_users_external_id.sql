-- Add `external_id` to `users` — the seed for Phase 1.d.
--
-- 1.d.1 (this PR) just adds the column. The JWT middleware lands in
-- 1.d.1-PR2/PR3: it parses an inbound `Authorization: Bearer …`,
-- verifies it against Better Auth's JWKS, extracts the `sub` claim,
-- and resolves it to a `users.id` via this column.
--
-- NULLABLE on purpose: the dev `X-User-Id` shim mints users without
-- an external_id and keeps working through the 1.d transition. Once
-- Better Auth is the only auth path (1.d.2), an ALTER COLUMN ... SET
-- NOT NULL after a backfill closes the slot.
--
-- UNIQUE because the `sub` claim is meant to be globally unique
-- across the auth provider's user space — two `users` rows pointing
-- at the same external id would let two distinct tenant boundaries
-- collide. The constraint also gives the JWT middleware's
-- `SELECT id FROM users WHERE external_id = $1` lookup a free index
-- without needing a dedicated `CREATE INDEX` line.

ALTER TABLE users
    ADD COLUMN external_id TEXT UNIQUE
        CONSTRAINT users_external_id_non_blank
            CHECK (
                external_id IS NULL
                OR length(regexp_replace(external_id, '\s', '', 'g')) > 0
            );

-- The CHECK locks the boundary invariant at the storage layer —
-- defense in depth on top of the trim+reject path in
-- `POST /api/v1/users`. A future code path that bypasses the
-- handler (a manual SQL fix, an internal job, a backfill script)
-- still can't sit a blank `''` / whitespace-only row that no JWT
-- would ever match.
--
-- `regexp_replace(…, '\s', '', 'g')` strips every whitespace char
-- (space, tab, newline, CR, form-feed, vertical-tab) before the
-- length probe. Postgres' bare `trim(…)` only strips ASCII space
-- by default — `'\t\n '` survived the original predicate, and CI
-- caught it. The regex form matches Rust's `str::trim()` Unicode
-- definition so the storage CHECK and the handler stay in lockstep.
--
-- NULL stays allowed because the dev `X-User-Id` shim mints users
-- without an external_id during the 1.d transition; the explicit
-- `IS NULL` short-circuit just makes the intent obvious (Postgres
-- already treats `NULL` CHECK results as "doesn't fail"). Same
-- pattern as `track.rating BETWEEN 0 AND 255` in
-- `20260530000003_track.sql`.
