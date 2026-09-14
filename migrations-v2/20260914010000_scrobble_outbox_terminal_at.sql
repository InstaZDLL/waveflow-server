-- The instant a queued listen stopped moving, so retention has something to
-- count from.
--
-- Not `created_at`: that dates the queueing, and a row would leave thirty days
-- after being written whatever happened to it in between. Not `updated_at`
-- going forward either: nothing guarantees it will not move again, and the
-- moment it does the purge starts counting from a different event without
-- saying so. A row records the instant it became terminal, once, and that
-- instant does not get rewritten.
ALTER TABLE scrobble_outbox ADD COLUMN terminal_at INTEGER;

-- And it is filled on databases already in service.
--
-- Rows written before this column are `sent`, `rejected`, `abandoned`,
-- `cancelled` or `discarded` with nothing to count from, and the purge would
-- not know when to start. They take their `updated_at`, which is the write that
-- made them terminal.
--
-- That holds because of a property of the code at this commit, not of the
-- schema: all eight `UPDATE`s `scrobble_outbox` knows require a non-terminal
-- starting state — `pending`, `sending` or `uncertain` — in their `WHERE`.
-- Nothing writes on a row that is already in one of the five states below, so
-- its last write *is* its transition. That property is exactly why the column
-- exists for the future rather than the purge going on reading `updated_at`:
-- the first `UPDATE` written without a state guard would take it away without a
-- sound.
--
-- `uncertain` is left empty on purpose. It is terminal for the server and is
-- never purged — decision 13 keeps an unanswered listen for as long as the
-- person has not answered it — and handing it an instant would invite somebody
-- to count from it one day.
UPDATE scrobble_outbox
   SET terminal_at = updated_at
 WHERE state IN ('sent', 'rejected', 'abandoned', 'cancelled', 'discarded');

-- What the purge asks for: the terminal rows, oldest first.
--
-- Partial, because the rows it must never return — `pending`, `sending`,
-- `uncertain` — are exactly the ones with no instant, so they cost the index
-- nothing and can never be reached through it.
CREATE INDEX scrobble_outbox_terminal_idx
    ON scrobble_outbox(terminal_at) WHERE terminal_at IS NOT NULL;
