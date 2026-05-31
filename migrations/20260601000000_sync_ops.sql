-- Multi-device sync log. Phase 1.f schema per RFC-001 §6.6.
--
-- The contract:
--   - `sync_op` is an append-only log keyed on `BIGSERIAL id`, the
--     single watermark every device pulls against. A per-(user, device)
--     `operation_id` UUID is the idempotency key so a client retry of
--     a successfully-pushed batch yields the same `id` instead of a
--     duplicate row, and a per-(user, device) `lamport_ts` enforces
--     ordering so a delayed-then-late replay is rejected as a regression
--     instead of silently re-ordering history.
--
--   - `device_sync_cursor` tracks how far each device has confirmed
--     reading. Only the explicit `POST /sync/ack` path writes here —
--     the `GET /sync/ops` pull is read-only on purpose, so a client
--     can re-pull without committing to having processed the rows.
--
--   - `sync_compaction_watermark` is the "below this id, history has
--     been collapsed" floor. The compaction job updates it in the same
--     transaction as the row deletes; a client requesting `since < N`
--     where N = compacted_up_to gets 410 Gone (resurrected-device case).
--
-- All three tables ON DELETE CASCADE via `user_id` so a user delete
-- fan-outs cleanly. No cross-table FKs beyond `users(id)` — the log
-- is intentionally decoupled from the entity tables (`playlist`,
-- `library`, etc.) so server-side schema evolution doesn't require
-- migrating historic ops.

CREATE TABLE sync_op (
    id              BIGSERIAL   PRIMARY KEY,
    user_id         BIGINT      NOT NULL
                    REFERENCES users(id) ON DELETE CASCADE,

    -- Stable identifier of the originating client. Free-form TEXT so
    -- the desktop can use a UUID generated at first launch, the web
    -- can use a per-tab UUID, and the test harness can use friendly
    -- names like "device-a" without round-tripping through a uuid
    -- parser. The (user_id, device_id) pair is what makes a Lamport
    -- clock meaningful — two unrelated devices can use the same
    -- lamport sequence without colliding.
    device_id       TEXT        NOT NULL,

    -- Client-generated UUID, the idempotency key. A POST replaying a
    -- previously-accepted op with the same `operation_id` short-
    -- circuits via `ON CONFLICT (user_id, device_id, operation_id)
    -- DO NOTHING` and returns the existing row, so a network blip
    -- mid-push never inflates the log.
    operation_id    UUID        NOT NULL,

    -- Per-(user, device) Lamport clock. The unique constraint below
    -- makes monotonicity inviolable at the storage layer — a stale
    -- replay attempting to land at an already-used lamport position
    -- surfaces as a 23505 unique violation, which the handler maps
    -- to 409 + the stored max so the client can resync its clock.
    lamport_ts      BIGINT      NOT NULL,

    -- Op shape. `entity` names the type ("playlist", "library", …);
    -- `entity_id` is a TEXT to accommodate cross-type id schemes (the
    -- desktop uses BIGINT ids for some tables and UUID-ish strings
    -- for others). `field` is NULL on whole-entity ops (insert /
    -- delete) and named on partial updates ("set name", "set color").
    entity          TEXT        NOT NULL,
    entity_id       TEXT        NOT NULL,
    field           TEXT,
    op              TEXT        NOT NULL,
    payload         JSONB,

    -- ms-epoch wall-clock the server recorded the op at. Used by
    -- operators to correlate the log with external timelines; the
    -- compaction job uses `sync_op.id` (not `created_at`) as the
    -- horizon so a backwards-running wall clock never breaks the
    -- monotonic-by-id invariant.
    created_at      BIGINT      NOT NULL,

    UNIQUE (user_id, device_id, operation_id),
    UNIQUE (user_id, device_id, lamport_ts)
);

-- The pull path is `WHERE user_id = $1 AND id > $since ORDER BY id`.
-- The (user_id, id) composite covers both the equality + range; a
-- user_id-only index would still work but would force a heap scan
-- across the matching range for the id sort.
CREATE INDEX sync_op_user_id_idx ON sync_op (user_id, id);

CREATE TABLE device_sync_cursor (
    user_id         BIGINT      NOT NULL
                    REFERENCES users(id) ON DELETE CASCADE,
    device_id       TEXT        NOT NULL,

    -- Highest `sync_op.id` this device has confirmed processing.
    -- Compaction MIN reads across every active device for the user;
    -- a single lagging device pins the floor for the whole tenant,
    -- which is intentional — we'd rather keep history than drop ops
    -- a device still needs.
    last_seen_id    BIGINT      NOT NULL,

    -- Wall-clock of the last ACK. Drives the stale-device cutoff so
    -- a device that hasn't talked to the server in > 90 days stops
    -- pinning the compaction floor (resurrected-device guard then
    -- handles the rejoin via 410 Gone).
    last_seen_at    BIGINT      NOT NULL,

    PRIMARY KEY (user_id, device_id)
);

-- Index for the compaction MIN query — `MIN(last_seen_id)` over
-- `WHERE user_id = $1 AND last_seen_at >= $stale_threshold`. The
-- column order matches the predicate shape (eq → range → projected
-- column) so the index can answer the query without a heap visit.
CREATE INDEX device_sync_cursor_user_recent_idx
    ON device_sync_cursor (user_id, last_seen_at, last_seen_id);

CREATE TABLE sync_compaction_watermark (
    user_id         BIGINT      PRIMARY KEY
                    REFERENCES users(id) ON DELETE CASCADE,

    -- All ops with `id <= compacted_up_to` have been collapsed; pulls
    -- starting `since < compacted_up_to` MUST 410 Gone instead of
    -- returning a partial history that would let the client converge
    -- on a state inconsistent with peers. Monotonically increasing;
    -- the compaction UPSERT refuses to lower it.
    compacted_up_to BIGINT      NOT NULL DEFAULT 0,
    updated_at      BIGINT      NOT NULL
);
