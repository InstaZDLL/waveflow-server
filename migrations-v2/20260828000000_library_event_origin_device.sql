-- Who caused a library change, when it was a client rather than a scan.
--
-- The table was created without this column, and its migration said why:
-- "nothing here is client-originated yet — a scan writes every row". That
-- stopped being true when metadata correction landed, and receiving a file
-- moved the server further from it still.
--
-- Without it a client reads its own upload back off the feed as a track it has
-- just discovered, and treats it as one. `sync_event` has carried
-- `origin_device_id` since its first day for exactly this reason; the library
-- feed needs the same fact for the same reason.
--
-- NULL keeps its plain meaning: no client asked for this. A scan writes NULL,
-- and so does a client that did not name a device — the header is optional, and
-- a client that wants its own changes back gets them.
ALTER TABLE library_event
    ADD COLUMN origin_device_id TEXT REFERENCES device(id) ON DELETE SET NULL;
