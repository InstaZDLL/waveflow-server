-- Internet radio stations, one list per account.
--
-- getInternetRadioStations answered an empty container from the beginning, and
-- that container was the one absence in the facade that could be misread: every
-- other missing feature declares a role `false`, so a client does not offer the
-- button, while "zero stations" and "no radio here" are the same answer on the
-- wire. There was nowhere to keep a station; this is that place.
--
-- **The server never fetches stream_url.** It is stored, listed and handed to
-- whichever client asked, which is what keeps this table from being a request
-- forger: a row here would otherwise be an arbitrary address the server could be
-- made to reach from inside the network it runs in. The value is validated on
-- the way in — absolute, http or https, with a host — so what is handed out is
-- at least a URL a client can refuse on its own terms.
--
-- Ownership is per account, like a bookmark and unlike the reference server,
-- where the list is global and only the administrator may change it. That
-- deviation is deliberate: WaveFlow has no global library setting of any kind,
-- and a per-account list needs no new privilege to be safe — each account sees
-- and edits its own, tenancy enforced in the queries.
--
-- Deliberately *not* in the sync journal. Stations sit beside the API tokens,
-- the Subsonic credential and the scrobbling links — per-account configuration,
-- not playback state — and `sync_event.entity_type` is CHECK-constrained, so
-- admitting a new entity type is a contract change the desktop client has to be
-- ready for. Nothing asked for it yet.
CREATE TABLE radio_station (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    stream_url TEXT NOT NULL,
    homepage_url TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- Stations are listed by name, per account, so the index carries the ordering's
-- collation as well as its columns. Nothing is unique beyond the id: two rows
-- may carry the same address under two names, because the protocol gives
-- `updateInternetRadioStation` no way to report a conflict, and a client that
-- creates a duplicate would be told the write failed for no reason it could act
-- on.
CREATE INDEX radio_station_user_name_idx
    ON radio_station (user_id, name COLLATE NOCASE, id);
