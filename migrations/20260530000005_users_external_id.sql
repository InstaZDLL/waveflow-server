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
    ADD COLUMN external_id TEXT UNIQUE;
