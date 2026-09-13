-- Where an account's authorisation to submit a listen lives, and what is
-- waiting to be submitted under it. RFC-010 carries the reasoning; what follows
-- is the shape.

-- One generation of one account's authorisation at one destination.
--
-- The row *is* the generation, which is why relinking inserts a new one rather
-- than updating this. RFC-010 decision 4: twenty listens are waiting, the
-- account unlinks Last.fm and links a different one — the account and the
-- destination are the same pair, the authorisation is not, and a queue keyed on
-- that pair would send the twenty to the second profile. `scrobble_outbox`
-- references this id, so the old rows can only ever name the authorisation they
-- were queued under.
--
-- The secret is sealed by `SecretBox` under the instance key, exactly as the
-- dedicated Subsonic password is: `data/waveflow.db` and `data/instance.key`
-- are one backup or neither is.
--
-- No destination URL here. Decision 10: a base URL is the operator's setting,
-- never an account's — a member picks among the destinations the server knows
-- and does not get to describe one.
CREATE TABLE scrobble_link (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('listenbrainz', 'maloja', 'lastfm')),
    -- `unlinked` is terminal and keeps the generation readable; `broken` is what
    -- an adapter's `AuthBroken` verdict writes, and it still holds its secret so
    -- the account is told which link stopped working rather than finding it
    -- gone.
    status TEXT NOT NULL CHECK (status IN ('active', 'broken', 'unlinked')),
    credential_nonce BLOB NOT NULL CHECK (length(credential_nonce) = 12),
    credential_ciphertext BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_success_at INTEGER,
    -- A normalised cause, never the destination's own words. Decision 12: what
    -- the API shows is a state, not an echo.
    last_failure TEXT
) STRICT;

-- At most one live generation per account and destination, held by the schema
-- rather than by whoever remembers to look first. Past generations stay
-- readable because `unlinked` is excluded.
CREATE UNIQUE INDEX scrobble_link_live_idx
    ON scrobble_link(user_id, provider) WHERE status <> 'unlinked';

-- One listen, frozen as it was heard, waiting for one destination.
--
-- The envelope is a copy and that is the point. RFC-010 decision 2: since #186 a
-- member corrects titles and artists, and a correction rewrites
-- `track_participant` — reading the track at drain time would submit what it has
-- become rather than what was played. A track deleted between the listen and the
-- send no longer empties the row either.
CREATE TABLE scrobble_outbox (
    -- The rowid stays the internal identity: `retry_of` points at it, the drain
    -- orders by it, and the jitter that keeps two rows from returning together
    -- is derived from it. None of that wants a UUID.
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- What the outside is allowed to name. Public ids are UUIDs here, and the
    -- two gestures decision 13 gives a person — discard this entry, retry that
    -- one — are the only things that ever name a row from outside.
    --
    -- A sequential integer would work and would also say how many listens this
    -- whole server has ever queued, to anyone holding one of their own. Added
    -- now rather than when the routes arrive: while this migration is unmerged
    -- it is one column, and afterwards it is a second migration and a backfill.
    public_id TEXT NOT NULL UNIQUE,
    link_id TEXT NOT NULL REFERENCES scrobble_link(id) ON DELETE CASCADE,
    -- Provenance, and nullable on purpose: `ON DELETE SET NULL` is what lets the
    -- envelope outlive the track it came from. A cascade here would undo the
    -- whole reason the envelope is a copy.
    play_event_id INTEGER REFERENCES play_event(id) ON DELETE SET NULL,
    -- Set only by the one deliberate duplicate this design accepts: decision 13,
    -- where a person retries an `uncertain` entry knowing the destination may
    -- already hold it. Nothing else ever writes it, so `retry_of IS NOT NULL`
    -- *means* a person asked for this row.
    retry_of INTEGER REFERENCES scrobble_outbox(id) ON DELETE SET NULL,
    played_at INTEGER NOT NULL,
    title TEXT NOT NULL,
    artists_json TEXT NOT NULL CHECK (json_valid(artists_json)),
    album TEXT,
    album_artist TEXT,
    duration_ms INTEGER,
    musicbrainz_recording_id TEXT,
    -- `sent`, `rejected`, `abandoned`, `cancelled`, `uncertain` and `discarded`
    -- are all terminal. They are six words rather than one because each names a
    -- different thing that happened, and the counters decision 12 describes
    -- cannot be computed from a single `done`.
    --
    -- `sending` is the one transient state, and it is what makes a submission
    -- at-most-once rather than merely usually-once. A row is moved into it by
    -- an `UPDATE … WHERE state='pending'` before anything is emitted, so two
    -- passes cannot both take it — and a row found in it long after any
    -- deadline could have expired belongs to a process that died mid-flight,
    -- which is `uncertain` by decision 5: nobody knows whether the destination
    -- recorded it. Left as `pending`, that row would simply be sent again.
    state TEXT NOT NULL CHECK (
        state IN (
            'pending', 'sending', 'sent', 'uncertain', 'discarded', 'rejected',
            'abandoned', 'cancelled'
        )
    ),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at INTEGER NOT NULL,
    last_failure TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- Exactly one queueing per listen and per authorisation — decision 5, held by
-- the schema and not only by the code.
--
-- `WHERE retry_of IS NULL` is what leaves room for the single exception:
-- a retry a person asked for names the same listen on purpose, and refusing it
-- here would make decision 13 unimplementable. Rows whose `play_event_id` has
-- been nulled by a deleted track are all distinct to SQLite, which is correct:
-- there is no longer a listen anything could queue a second time.
CREATE UNIQUE INDEX scrobble_outbox_once_idx
    ON scrobble_outbox(play_event_id, link_id) WHERE retry_of IS NULL;

-- One deliberate duplicate per ambiguous entry, and not two.
--
-- Decision 13 lets a person retry a listen whose fate is unknown, accepting
-- that the destination may already hold it. That acceptance is given once. The
-- original stays `uncertain` afterwards — it must, because erasing it would
-- falsify the only trace explaining why a duplicate exists — so nothing in the
-- row itself says it has already been answered, and a second call would queue a
-- second copy. Here rather than only in the service, for the reason decision 5
-- gives about the index above: the one path in this design that can manufacture
-- duplicates on demand should be shut by the schema.
CREATE UNIQUE INDEX scrobble_outbox_one_retry_idx
    ON scrobble_outbox(retry_of) WHERE retry_of IS NOT NULL;

-- What the drain asks for: the due rows, in the order they became due.
CREATE INDEX scrobble_outbox_due_idx ON scrobble_outbox(state, next_attempt_at);

-- What the counters ask for: one link's queue, by state.
CREATE INDEX scrobble_outbox_link_idx ON scrobble_outbox(link_id, state);

-- What the foreign key asks for when a listen is deleted.
--
-- `play_event_id` is `ON DELETE SET NULL`, so removing a `play_event` row makes
-- SQLite look for the outbox rows pointing at it. The only other index leading
-- with this column is `scrobble_outbox_once_idx`, which is partial and so cannot
-- answer for a retry row — leaving a scan. Deleting tracks is not a rare event
-- here: an ordinary rescan that finds files gone cascades track -> play_event
-- -> this lookup.
CREATE INDEX scrobble_outbox_play_event_idx ON scrobble_outbox(play_event_id);
