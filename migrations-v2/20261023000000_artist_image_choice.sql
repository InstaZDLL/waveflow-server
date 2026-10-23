-- An artist's portrait and banner, chosen by hand (RFC-013 decision 2).
--
-- A table of its own and never `artist.artwork_hash`: RFC-011's portrait fill
-- rewrites that column whenever it differs, so a choice stored there would be
-- replaced by the provider's the next time anyone opened the page. Keyed by
-- the artist row, which is per library: each library's owner chooses for
-- their own. The artwork it names is kept by the sweep for as long as it is
-- named; no `ON DELETE` on that key, so a sweep that forgot would fail rather
-- than leave a choice pointing at nothing.
CREATE TABLE artist_image_choice (
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('portrait', 'banner')),
    artwork_hash TEXT NOT NULL REFERENCES artwork(hash),
    source TEXT NOT NULL CHECK (source IN ('upload', 'deezer')),
    set_by TEXT NOT NULL,
    set_at INTEGER NOT NULL,
    PRIMARY KEY (artist_id, kind)
) STRICT;

CREATE INDEX artist_image_choice_artwork ON artist_image_choice(artwork_hash);
