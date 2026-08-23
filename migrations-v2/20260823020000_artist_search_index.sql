-- An artist is searched by their own name.
--
-- `search3` had no index of artists, so it answered the artist half of a
-- search by deriving it: take the tracks the full-text index matched, return
-- everybody credited on them. That returns people whose names have nothing to
-- do with the query — searching a track title returned its whole session
-- crew — and the participants model made it worse, because a track can now
-- carry thirteen roles where it used to carry one list of artists.
--
-- The reference gives the artist its own full-text row and matches the name.
-- So do we. Same tokenizer as `track_fts`, for the same reason: it folds case
-- and diacritics, so "Beyonce" finds "Beyoncé" and a trailing prefix keeps
-- search-as-you-type working.
--
-- `sort_name` is indexed beside the name because it is where a leading article
-- goes: a catalogue that files "The Beatles" under "Beatles, The" should
-- answer to either.
CREATE VIRTUAL TABLE artist_fts USING fts5(
    artist_id UNINDEXED,
    library_id UNINDEXED,
    name,
    sort_name,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Populated here rather than left to the next scan: search would otherwise
-- answer nothing for artists between this migration and the first rescan, and
-- a search that silently returns nothing reads exactly like a broken server.
INSERT INTO artist_fts (artist_id, library_id, name, sort_name)
SELECT id, library_id, name, COALESCE(sort_name, '') FROM artist;
