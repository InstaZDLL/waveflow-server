-- Correcting the two fields that are also rows.
--
-- The first pass of `track_override` carried only columns of `track`, because
-- an artist and a genre are more than that: they are rows in
-- `track_participant` and `track_genre`, rebuilt by every scan from the file.
-- Overriding the display string alone would have a track answer `artist` and
-- `artists[]` differently until the next scan reconciled the two.
--
-- Stored as JSON rather than the `;`-joined form the tag columns use, and the
-- difference is the point. That form exists because a file writes its artists
-- however it likes and the tag mapper has to guess where one name ends. An
-- override is a list someone typed on purpose — re-parsing it with a heuristic
-- would be inventing ambiguity that was never there, and would lose any name
-- holding the separator.
ALTER TABLE track_override ADD COLUMN artists TEXT
    CHECK (artists IS NULL OR json_valid(artists));
ALTER TABLE track_override ADD COLUMN genres TEXT
    CHECK (genres IS NULL OR json_valid(genres));
