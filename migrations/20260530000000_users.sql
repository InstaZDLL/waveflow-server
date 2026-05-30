-- Users table — owns the multi-tenant boundary for every other
-- resource on this server. A profile (and later a library, a
-- playlist, …) belongs to exactly one user via a FK; the
-- `X-User-Id` middleware in `src/middleware/auth.rs` decides which
-- user a request acts on.
--
-- Phase 1.b is the *dev-only* auth shim — the FK + a manually-
-- supplied id are all we need to wire CRUD end-to-end. Phase 1.d
-- replaces the shim with JWT verification from Better Auth, at
-- which point the user id comes from the verified `sub` claim
-- instead of an arbitrary header. The `users` table stays as-is;
-- the source of the id is what changes.

CREATE TABLE users (
    id          BIGSERIAL PRIMARY KEY,
    -- Epoch milliseconds. Same shape as every other timestamp in this
    -- repo (profile.created_at, profile.last_used_at, …) so cross-
    -- backend code in `waveflow-core` can use a single `i64`
    -- everywhere.
    created_at  BIGINT    NOT NULL
);
