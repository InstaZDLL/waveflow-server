-- The joker decision 13 grants is counted on the entry, not on its descendant.
--
-- Until now "this ambiguous entry has already been retried" was read as a join:
-- an entry stayed answerable for as long as no other row named it through
-- `retry_of`. That holds only while rows are immortal, and RFC-010's retention
-- makes them mortal — a retry ends `sent`, so it becomes purgeable, and the
-- original then turns answerable a second time. Measured rather than feared:
-- with the retry row gone the entry reappears in `uncertain_scrobbles`,
-- `retry_uncertain_scrobble` accepts it again, and the link's `uncertain`
-- counter puts it back to `degraded` for a listen already answered.
--
-- So the fact moves onto the entry itself: null until it is not.
ALTER TABLE scrobble_outbox ADD COLUMN retried_at INTEGER;

-- And it is filled on databases already in service.
--
-- Retried entries exist since #191, recognisable by exactly the join this
-- migration abandons. Adding the column empty would make every one of them
-- answerable a second time: on the only servers that have any, the migration
-- would reintroduce the very defect it exists to correct. So it reads
-- `retry_of` one last time.
--
-- The instant taken is the retry row's `created_at`, which is when the joker
-- was spent. `scrobble_outbox_one_retry_idx` is unique over `retry_of`, so the
-- correlated subquery names at most one row and the value is not a choice
-- among several.
UPDATE scrobble_outbox
   SET retried_at = (
       SELECT r.created_at
         FROM scrobble_outbox r
        WHERE r.retry_of = scrobble_outbox.id
   )
 WHERE EXISTS (
       SELECT 1
         FROM scrobble_outbox r
        WHERE r.retry_of = scrobble_outbox.id
   );
