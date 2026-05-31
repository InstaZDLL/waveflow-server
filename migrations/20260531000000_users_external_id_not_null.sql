-- Phase 1.d.2: Better Auth is now the only configured auth path,
-- which means every `users` row exists because a Better Auth JWT
-- minted it (lazy-provisioned by `find_or_provision_by_external_id`
-- in the middleware). A NULL `external_id` is therefore a dangling
-- row — no JWT can ever authenticate against it — so the column
-- gets the NOT NULL constraint the lookup invariant always wanted.
--
-- The previous migration (20260530000005_users_external_id) added
-- the column as nullable so the Phase 1.b `X-User-Id` shim could
-- mint users via `POST /api/v1/users` without an upstream account.
-- The shim retires with this PR, so the nullable variant is
-- unreachable from production code.
--
-- Backfill: there is no install where this server runs in
-- production yet (1.c hasn't deployed), so the only rows with NULL
-- `external_id` are dev-time / test artifacts that the next test
-- run would have wiped anyway. Delete them outright rather than
-- inventing a synthetic external_id that no JWT could ever
-- resolve to.

DELETE FROM users WHERE external_id IS NULL;

ALTER TABLE users ALTER COLUMN external_id SET NOT NULL;
