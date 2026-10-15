-- What the metadata providers said about an artist — RFC-011 decisions 4 and 5.
--
-- **Instance-wide, and nothing in it is account data.** A biography is the same
-- text whoever asks, and `artist` rows are per library, so keying on
-- `artist.id` would fetch the same answer once per library. The key is the
-- name instead, in the form `artist.canonical_name` already stores
-- (`waveflow_core::scanner::canonical_name`), so the read path joins on that
-- column with nothing recomputed. RFC-011 first named
-- `name_match::normalize_name` here; the two disagree — "Björk" is `björk` in
-- the column and `bjork` through it — and its erratum says so.
--
-- **The provider is part of the key.** Each provider keeps its own row, its
-- own outcome and its own clock, and the fields are merged at read time in the
-- operator's order. One row per name would have to pick a winner when written,
-- and would let one provider's "never heard of them" silence another that has
-- a biography.
--
-- `provider` carries no CHECK: the vocabulary is the server's `Provider` enum,
-- and widening a CHECK in SQLite means rebuilding the table. `outcome` does,
-- because its two words are decision 5's and do not grow — the third outcome,
-- unreached, is the absence of a row.
--
-- Album notes (`metadata_album`) wait for the RFC's open question on them.
-- Portraits add their columns with the fetch that fills them.
CREATE TABLE metadata_artist (
    provider TEXT NOT NULL,
    name_canonical TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('hit', 'miss')),
    bio_summary TEXT,
    bio_full TEXT,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (provider, name_canonical)
) STRICT, WITHOUT ROWID;

-- The ranked neighbours, as the provider gave them. Kept by name and resolved
-- to local artists only when read, through the caller's own libraries
-- (decision 7): an id here would name a row in somebody's library.
--
-- Rewritten whole with its parent row, in the same transaction, so a list is
-- never half one answer and half the next. The cascade only matters to a row
-- deleted outright; a refresh updates the parent in place.
CREATE TABLE metadata_artist_similar (
    provider TEXT NOT NULL,
    name_canonical TEXT NOT NULL,
    rank INTEGER NOT NULL,
    similar_name TEXT NOT NULL,
    similar_canonical TEXT NOT NULL,
    score REAL NOT NULL,
    PRIMARY KEY (provider, name_canonical, rank),
    FOREIGN KEY (provider, name_canonical)
        REFERENCES metadata_artist(provider, name_canonical) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
