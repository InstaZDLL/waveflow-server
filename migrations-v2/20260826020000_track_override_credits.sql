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
--
-- The CHECK asserts an array and not merely valid JSON, so a scalar or an object
-- is refused at the door rather than decoded into a surprise. It stops there:
-- asserting that every element is a string needs `json_each`, and SQLite
-- prohibits subqueries in a CHECK outright. The service writes these columns
-- from a `Vec<String>` and nothing else does, so the element type is held by the
-- one writer rather than by the schema — worth knowing rather than assuming.
ALTER TABLE track_override ADD COLUMN artists TEXT
    CHECK (artists IS NULL OR json_type(artists) = 'array');
ALTER TABLE track_override ADD COLUMN genres TEXT
    CHECK (genres IS NULL OR json_type(genres) = 'array');
