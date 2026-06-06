-- =============================================================================
-- Phase 1.i.1 / CR round 1 — starvation guard for the background scanner.
--
-- Without a per-row failure timestamp, the scanner's
-- `ORDER BY created_at ASC LIMIT N` would re-serve the same broken
-- parents (e.g. ones whose source bytes were lost from object_store)
-- in the head of every cycle. With enough irrecoverable parents the
-- batch fills with retries and recoverable parents starve.
--
-- `last_repair_failure_at` is updated to `now()` when a repair fails;
-- the scanner query then filters parents that failed inside the
-- backoff window (1 hour by default) and orders by
-- `last_repair_failure_at NULLS FIRST, created_at ASC` so untouched
-- parents always lead, freshly-failed ones recede until the cooldown
-- expires, and the queue keeps draining.
-- =============================================================================

ALTER TABLE metadata_artwork
    ADD COLUMN last_repair_failure_at TIMESTAMPTZ;

-- Index targets the scanner's query shape: filter on the backoff
-- predicate + sort by the same column. Postgres ignores NULL rows
-- under `last_repair_failure_at IS NULL OR ... < $`, so a partial
-- index on the "has failure timestamp" set keeps the index tight
-- while still covering the sort key.
CREATE INDEX idx_metadata_artwork_repair_pending
    ON metadata_artwork(last_repair_failure_at)
    WHERE last_repair_failure_at IS NOT NULL;
