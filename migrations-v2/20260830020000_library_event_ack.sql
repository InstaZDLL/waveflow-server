-- How far a device has read one library's feed. RFC-007 decision 8.
--
-- `sync_ack` minus its account column, and the difference is the whole reason
-- this is a second table rather than a wider first one: the user journal is
-- keyed per account, this feed is keyed per library. A device belongs to
-- exactly one account (`device.user_id`), so an account column here would be a
-- third value derivable from the other two — a second truth to keep in
-- agreement with the first, and the sort that goes stale in one place only.
--
-- The account is therefore re-read from the device and checked against
-- `library_member` at write time, which is the rule every other read in this
-- server follows: tenancy lives in the query.
--
-- Both foreign keys cascade. A revoked device and a deleted library each leave
-- nothing behind, and neither needs a sweeper to notice.
--
-- What this table deliberately does not do is hold back the purge. A device
-- that never comes back would pin a feed forever, and a shared library would
-- lose retention entirely the moment one phone was thrown away. Retention is
-- decided by RFC-007 decision 7 and reported against these rows; it is not
-- bounded by them.
CREATE TABLE library_event_ack (
    library_id TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES device(id) ON DELETE CASCADE,
    cursor INTEGER NOT NULL CHECK (cursor >= 0),
    acknowledged_at INTEGER NOT NULL,
    PRIMARY KEY (library_id, device_id)
) STRICT;
