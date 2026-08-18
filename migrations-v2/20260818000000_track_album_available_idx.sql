-- album_select! now sizes every album it returns: COUNT(*) and SUM(duration_ms)
-- over the album's available tracks. Those subqueries used to run only when a
-- whole catalogue snapshot was materialised; they now run for each row of an
-- ordered, paged album listing on both the native and Subsonic surfaces.
--
-- track_album_idx(album_id) seeks the right tracks but leaves both aggregates to
-- read every matching row for the availability flag and the duration. Carrying
-- them in the index answers both without touching the table. The narrow index is
-- dropped rather than kept alongside: its leading column is unchanged here, so
-- every query it served still has a usable index and the write cost of a second
-- one on the scanner's hottest table buys nothing.
CREATE INDEX track_album_available_idx ON track (album_id, is_available, duration_ms);

DROP INDEX track_album_idx;
