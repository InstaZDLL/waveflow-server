-- RFC-003 Phase A.1 — additive HLC fields on `sync_op`.
--
-- Carries the v2 wire shape per RFC-003 §2 alongside the legacy v1
-- `lamport_ts`. Server prefers v2 when present, falls back to v1 — both
-- shapes are accepted during Phase A so a desktop running pre-v2 code
-- can still push, and a desktop running v2 code can still be served
-- catchup ops emitted by a server that only filled `lamport_ts` for its
-- pre-A rows.
--
-- The HLC pair is (wall: BIGINT epoch-millis, logical: INT counter).
-- The §2 total-order tiebreaker `origin_device_id` is the existing
-- `sync_op.device_id` column (TEXT — kept as-is rather than narrowed
-- to UUID at this layer because the entity tables that gain a stricter
-- `origin_device_id UUID` column in A.1.2 are the canonical identity,
-- while `sync_op.device_id` is the wire-shape carrier and a UUID-shaped
-- TEXT round-trips without loss).
--
-- The legacy `UNIQUE (user_id, device_id, lamport_ts)` stays in place —
-- a v1 desktop pushing a stale lamport replay must still 23505 the same
-- way it does today. A second `UNIQUE (user_id, device_id, hlc_wall,
-- hlc_logical)` enforces the same invariant on v2 wire-shape pushes;
-- per-device the HLC pair is monotonic by §2's construction so this is
-- the natural-key constraint, not an artificial extra row check.
--
-- Backfill stamps legacy rows with `(0, lamport_ts)` so the new
-- constraint holds without conflicting against the pre-existing
-- `lamport_ts` uniqueness. That ordering is consistent with the §2
-- total order (any v2 op with `hlc_wall > 0` strictly outranks every
-- legacy row), so a Phase-A server applying both legacy and v2 ops
-- against the same entity will pick the v2 op as more recent — the
-- intended LWW behaviour for any device that has upgraded.

ALTER TABLE sync_op
    ADD COLUMN hlc_wall    BIGINT,
    ADD COLUMN hlc_logical INTEGER;

-- Preflight: refuse to lossy-truncate `lamport_ts` into the
-- 32-bit `hlc_logical`. Per RFC-003 §2 the logical counter is
-- explicitly `u32` (the HLC paper's shape); the legacy
-- `lamport_ts` is `BIGINT` so in principle it could have grown
-- past 2^31-1 over a long-lived install. Any practical real-world
-- install today is in the low thousands at most (the counter is
-- reset every wall-clock tick under HLC; the legacy v1 Lamport
-- is monotonic but small in absolute terms), so this check is
-- expected to be a no-op — but if it ever fires, the operator
-- needs to manually decide whether to widen the column or reset
-- the affected device's counter rather than have the cast
-- silently drop a digit.
DO $$
DECLARE oversized INTEGER;
BEGIN
    SELECT COUNT(*) INTO oversized
      FROM sync_op
     WHERE lamport_ts > 2147483647 OR lamport_ts < 0;
    IF oversized > 0 THEN
        RAISE EXCEPTION
            'sync_op.lamport_ts is out of range for hlc_logical (INTEGER) in % row(s). Migration aborted to avoid silent truncation. Widen hlc_logical to BIGINT or reset the affected device counters before retrying.',
            oversized;
    END IF;
END $$;

UPDATE sync_op
   SET hlc_wall    = 0,
       hlc_logical = lamport_ts::INTEGER
 WHERE hlc_wall    IS NULL;

ALTER TABLE sync_op
    ALTER COLUMN hlc_wall    SET NOT NULL,
    ALTER COLUMN hlc_logical SET NOT NULL;

ALTER TABLE sync_op
    ADD CONSTRAINT sync_op_user_device_hlc_uniq
        UNIQUE (user_id, device_id, hlc_wall, hlc_logical);
