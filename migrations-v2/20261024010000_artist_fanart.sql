-- An automatic artist banner, from TheAudioDB's fanart (RFC-014 decision 3).
--
-- The cache records the fanart a provider answered, as `image_hash` records a
-- portrait, and loses it the same way when the sweeper takes the artwork row.
ALTER TABLE metadata_artist
    ADD COLUMN fanart_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL;

-- Every artist row with the name carries it, in every library, written the way
-- `artwork_hash` is: by the fill, only where it changes, with the artist event.
-- It is the provider's and never a choice — a banner chosen by hand stays in
-- `artist_image_choice`. `ON DELETE SET NULL` as on `artwork_hash`: the sweep
-- keeps what an artist names, so this only ever clears a link to artwork that
-- is already gone.
ALTER TABLE artist
    ADD COLUMN fanart_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL;

CREATE INDEX artist_fanart ON artist(fanart_hash);
