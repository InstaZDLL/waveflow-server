-- The quarter of an hour in which somebody authorises this server at Last.fm.
--
-- RFC-010 decision 11: Last.fm hands out no pasteable secret. A person goes to
-- their site, authorises, and comes back carrying a request token which this
-- server exchanges for a session key. That round trip needs a state, and the
-- state has to survive a restart — it is the one journey in this design that
-- cannot be resumed from the beginning without the person noticing, so keeping
-- it in memory would lose it at exactly the worst moment.
--
-- Deliberately not `scrobble_outbox`, and deliberately its own table: a
-- fifteen-minute journey has nothing in common with a listen waiting for a
-- destination, and the retention window that governs the queue is three orders
-- of magnitude too long for this.
CREATE TABLE lastfm_authorization (
    -- The random that travels in the return path, and the row's own name.
    --
    -- In the **path** rather than in a query string: Last.fm documents that it
    -- appends `/?token=…` to the callback, which says nothing about what it
    -- would do with a callback that already carried a `?`. A path segment
    -- depends on no assumption about that concatenation.
    --
    -- Drawn from the cryptographic generator on at least 128 bits: it is the
    -- only thing separating a legitimate return from a fabricated one.
    state TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    -- Named on the way out and found again on the way back. Last.fm has one
    -- instance today, so it would be tempting to leave it implicit — but
    -- decision 10 has just refused every implicit default, and an exception for
    -- one recipient is the first step back onto the slide it forbids.
    destination TEXT NOT NULL,
    -- And the return checks *where* as well as *which*. A quarter of an hour
    -- separates the two halves and a server can restart in between — which is
    -- exactly why this row is in a database. Destination gone or moved, the
    -- return is refused and nothing is created: otherwise the journey would
    -- manufacture a link to a machine the person never chose, already wrong at
    -- birth.
    destination_fingerprint TEXT NOT NULL,
    -- What pairs the return to the browser that opened it.
    --
    -- The state alone would be enough if the return URL never left that
    -- browser — but it travels through Last.fm, and a URL that travels lands in
    -- a referrer and in a history. Whoever obtained it could finish the journey
    -- with *their* token, and somebody else's account would start scrobbling to
    -- their profile.
    --
    -- Hashed, never stored: this is a credential like every other one in this
    -- schema, and SHA-256 is what `api_token` and the OAuth codes keep.
    cookie_hash BLOB NOT NULL CHECK (length(cookie_hash) = 32),
    created_at INTEGER NOT NULL,
    -- Ten to fifteen minutes, well inside the sixty Last.fm grants its token:
    -- we refuse first, and an expired token never surprises us.
    --
    -- The purge task applies *this* instant, and not the queue's thirty-day
    -- window. Borrowing the wrong one of the two would keep a journey alive
    -- long after it should have died — and a revision that ends one table's
    -- unbounded growth would look poor introducing another.
    expires_at INTEGER NOT NULL
) STRICT;

-- What the purge asks for: the states that have run out, oldest first.
CREATE INDEX lastfm_authorization_expiry_idx ON lastfm_authorization(expires_at);

-- One journey at a time, per account.
--
-- The cookie carries a fixed name, so opening a second journey replaces the
-- first: the tab left behind can no longer conclude, and its state would expire
-- alone. Held here as well so the table cannot accumulate what the browser can
-- no longer finish — and so that "one journey" is a property of the data rather
-- than of whoever remembers to delete the previous row.
CREATE UNIQUE INDEX lastfm_authorization_one_per_account_idx
    ON lastfm_authorization(user_id);
