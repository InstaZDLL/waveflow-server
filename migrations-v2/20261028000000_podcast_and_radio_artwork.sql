-- Artwork for podcast channels and internet radio stations (RFC-025, amended).
--
-- Both carried a cover address and nothing else, and the web client could not
-- show it: RFC-019's Content-Security-Policy keeps `img-src` to this origin,
-- and loading the image from the feed's host would hand that host the address
-- of every listener. So the server fetches the image once, under the rules of
-- `src/podcast.rs` (public addresses only, checked and pinned, redirects
-- followed by hand and checked before each hop, a size ceiling, the bytes
-- sniffed as JPEG, PNG or WebP), stores it in the artwork store, and a client
-- reads it as it reads any cover: `/api/v2/artwork/{hash}`.
--
-- `artwork_hash` references the store as the catalogue's columns do. The
-- artwork sweep counts both columns as references, so a cover named here is
-- never collected, and deleting the channel or the station is what releases
-- it.

-- `artwork_url` is the address the stored cover was fetched from, which is
-- not always the address the feed names now: when a feed changes its cover
-- and the new one cannot be fetched, the old cover stays shown and the next
-- refresh tries again, because the two addresses differ. `artwork_fetched_at`
-- is when it was last fetched, so an unchanged address is fetched again only
-- once its age passes the TTL. Neither is shown to anybody: a private feed's
-- cover can carry its subscriber's token as readily as the feed itself.
ALTER TABLE podcast_channel ADD COLUMN artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL;
ALTER TABLE podcast_channel ADD COLUMN artwork_url TEXT;
ALTER TABLE podcast_channel ADD COLUMN artwork_fetched_at INTEGER;

-- `image_url` is what the account typed, validated as `homepage_url` is:
-- absolute, http or https. It is kept when the image cannot be fetched, and
-- `artwork_hash` is then NULL: the station shows the default symbol until the
-- next save that carries an address tries again.
ALTER TABLE radio_station ADD COLUMN image_url TEXT;
ALTER TABLE radio_station ADD COLUMN artwork_hash TEXT REFERENCES artwork(hash) ON DELETE SET NULL;

-- What the sweep and the artwork route ask: "does anything name this hash?"
-- Partial, because most rows name nothing.
CREATE INDEX podcast_channel_artwork_idx
    ON podcast_channel (artwork_hash) WHERE artwork_hash IS NOT NULL;
CREATE INDEX radio_station_artwork_idx
    ON radio_station (artwork_hash) WHERE artwork_hash IS NOT NULL;
