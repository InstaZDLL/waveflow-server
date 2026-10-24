-- A Deezer search for an artist's portrait, and what it offered (RFC-013
-- decision 4).
--
-- The pick names a Deezer id and nothing else, so the server remembers what it
-- offered: a search per artist and query, and its candidates in Deezer's
-- order, each with the preview the server fetched. Keyed by the artist row,
-- which is per library, so any owner or manager of that library may pick from
-- it and no other library ever finds it.
CREATE TABLE artist_image_search (
    id TEXT PRIMARY KEY NOT NULL,
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    -- The query as typed, trimmed and case-folded: "Björk " and "björk" are
    -- one search.
    query_key TEXT NOT NULL,
    searched_by TEXT NOT NULL,
    searched_at INTEGER NOT NULL,
    -- An hour from the moment the search completed; NULL while it fetches.
    expires_at INTEGER,
    -- Set when every candidate's preview is stored. 0 means a task is
    -- fetching for it now: a failed search is deleted, a search past its
    -- fetch deadline is swept, and boot deletes any left at 0. It is what
    -- makes a second search by the account answer 429.
    complete INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1)),
    CHECK ((complete = 1) = (expires_at IS NOT NULL))
) STRICT;

CREATE INDEX artist_image_search_artist ON artist_image_search(artist_id, query_key);
CREATE INDEX artist_image_search_account ON artist_image_search(searched_by, complete);

-- The preview names its artwork with no `ON DELETE`, as a choice does: the
-- sweep keeps what a candidate names, and one that forgot would fail rather
-- than leave a candidate pointing at nothing.
CREATE TABLE artist_image_candidate (
    search_id TEXT NOT NULL REFERENCES artist_image_search(id) ON DELETE CASCADE,
    -- Deezer's order, so the answer reads as the search did.
    position INTEGER NOT NULL,
    deezer_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    fans INTEGER,
    artwork_hash TEXT NOT NULL REFERENCES artwork(hash),
    PRIMARY KEY (search_id, position),
    UNIQUE (search_id, deezer_id)
) STRICT;

CREATE INDEX artist_image_candidate_artwork ON artist_image_candidate(artwork_hash);
