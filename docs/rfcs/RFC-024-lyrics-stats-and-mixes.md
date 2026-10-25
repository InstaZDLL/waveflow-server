# RFC-024 — Discovery: LRCLIB lyrics, listening statistics, mixes and local similar songs

- **Status**: Proposed
- **Implemented by**: in progress. The *Status* field above never flips in
  this project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the frozen
  Subsonic contract, tenancy), [RFC-011](RFC-011-metadata-enrichment.md) (how
  an outbound lookup is switched on, cached, paced and logged),
  [RFC-012](RFC-012-library-browser.md) decision 6 (the order lyrics sheets
  are chosen in)
- **Amends**: RFC-002, on what `getSimilarSongs` and `getSimilarSongs2`
  answer when no metadata provider is configured, and RFC-011 decision 6, for
  lyrics only, on whether a lookup may be waited for

## Problem

Four things a listener reasonably expects of a music server are missing, and
each of them can be built from what the server already holds or from one
narrow, optional outbound source:

1. **A track without lyrics stays without them.** The scanner reads embedded
   tags and `.lrc`/`.txt` sidecars, and an owner can upload a sheet
   (RFC-012), but a library assembled from files that carry none shows none.
   The desktop already asks [LRCLIB](https://lrclib.net), a free database of
   lyrics that needs no key; `waveflow-core` has a client for it
   (`metadata::lrclib`).
2. **The listening history is write-only.** `play_event` records every listen
   on every surface, and nothing reads it back except play counts and
   `getTopSongs`. There is no answer to "what did I listen to this month?".
3. **Nothing is suggested.** The home page is the album grid. The catalogue
   knows what was played, what was starred, what was never played and what
   arrived last week, and composes none of it.
4. **`getSimilarSongs` is empty by default.** With no metadata provider
   configured, which is the default, both Subsonic methods answer an empty
   container (`src/services/artist_enrichment.rs`), although the catalogue
   holds genres, years, co-credits and the listener's own history.

Smart playlists with user-written rules are not part of this RFC; they belong
to their own.

## Decision 1 — LRCLIB is a lyrics source, off until an operator names it

LRCLIB is switched on the way RFC-011's providers are, by an ordered list that
is empty by default and is the only switch:

| Variable | Default | Meaning |
| --- | --- | --- |
| `WAVEFLOW_LYRICS_PROVIDERS` | empty | `lrclib`, or empty for off. A name the server does not know refuses to boot |
| `WAVEFLOW_LYRICS_TTL_DAYS` | 30 | How long lyrics found are kept before being asked again |
| `WAVEFLOW_LYRICS_MISS_TTL_DAYS` | 7 | How long "LRCLIB has nothing for this track" is kept |
| `WAVEFLOW_LYRICS_RATE_PER_SEC` | 2 | Requests per second |
| `WAVEFLOW_LYRICS_REQUEST_TIMEOUT_SECS` | 10 | How long one request may take before it counts as unanswered |

**A separate switch from `WAVEFLOW_METADATA_PROVIDERS`, on purpose.** An
artist lookup sends an artist's name. A lyrics lookup sends a track title, its
album and its duration, at the moment somebody plays it — a much closer signal
of what this server's users listen to. An operator who agreed to the first has
not agreed to the second, so `lrclib` is not a value of the metadata list.

**No key, so nothing to keep secret, and an honest User-Agent.** LRCLIB asks
clients to identify themselves with a name, a version and a link. The lookup
sends `WaveFlowServer/<version> (https://github.com/InstaZDLL/waveflow-server)`,
the outbound client's usual agent with the project's address appended. Nothing
in it names the instance.

The client is the server's own, not `waveflow_core::metadata::lrclib`: the
core's builds its own HTTP client, which follows redirects inside the origin,
fixes its address and reads a body of any size. The server's is built by
`src/outbound.rs` — no redirect, no proxy from the environment — reads at most
1 MiB, the cap lyrics uploads and sidecars already have, and is pointed at a
listener of the test suite's own.

## Decision 2 — the file and the uploads always win

LRCLIB is asked only when the track has **no lyrics of its own**: no scanned
sheet with text in it (embedded or sidecar) and no uploaded sheet. A track
that has one is never looked up, whatever the surface asking, so a sheet an
owner corrected by hand is never second-guessed.

When LRCLIB answers, its sheet is appended to the empty answer: one sheet,
synced when LRCLIB has timings and plain otherwise, in language `xxx` since
LRCLIB names none. An instrumental answer is no sheet.

**Matching.** The request carries the track's artist as displayed
(`track.artist_display`), its effective title (a title correction wins, as it
does everywhere else), its album when it has one, and its duration rounded to
the second. LRCLIB's `/api/get` matches on all four with a tolerance of two
seconds on the duration. A track with no artist, or a duration of zero, is not
looked up: there is nothing to match it on. An answer whose own duration is
more than two seconds from the track's is taken as a miss, so a looser match on
LRCLIB's side cannot attach another recording's words.

## Decision 3 — on demand, never at scan time, and waited for briefly

The lookup is made **when lyrics are asked for**: the native lyrics route and
its download, the properties panel and the playing screen that read it,
`getLyricsBySongId`, and `getLyrics` (decision 5). Never at scan time — a
first scan of a large library would spend days of LRCLIB's goodwill on tracks
nobody plays — and never on a timer.

**This is where it departs from RFC-011 decision 6**, which refuses to wait
for a provider on the answering path. That rule protects a page a client
revisits; lyrics are asked once per listen, by the client playing the track,
and an empty first answer is the whole answer for that listen — no client
asks again while the song plays. So the request that finds nothing schedules
the lookup and **waits for it for at most four seconds**. A lookup that takes
longer still completes in the background and is cached for the next listen;
the request answers with what it had, which is no sheet.

What RFC-011 decision 6 protects is kept:

- **The writer gate is never held across the network.** The row is written
  after the request is over, under the gate, in one statement.
- **One request per track in flight.** Concurrent asks for the same lookup key
  wait on the same request rather than sending their own.
- **Paced and bounded.** Requests are spaced to `WAVEFLOW_LYRICS_RATE_PER_SEC`,
  at most 64 lookups wait or run at once, and beyond that nothing is
  scheduled: the next ask tries again.

## Decision 4 — the cache is instance-wide, keyed by what was asked

Like RFC-011's, the cache holds public data that belongs to no account:
`lyrics_lookup`, keyed by provider and a BLAKE3 digest of the canonical
artist, title, album and duration that were sent. The same recording in two
libraries is asked about once. A track whose tags change is asked about
again, under its new key, because what is sent has changed.

Three outcomes, as in RFC-011 decision 5:

- **hit** — the lyrics, kept for `WAVEFLOW_LYRICS_TTL_DAYS`;
- **miss** — LRCLIB answered 404, or an instrumental, or a match outside the
  duration tolerance, kept for `WAVEFLOW_LYRICS_MISS_TTL_DAYS` (an
  instrumental is stored as its own outcome with the long TTL, since it is an
  answer and not an absence);
- **unreached** — a timeout, a refused connection, a status other than 200 or
  404, an answer over 1 MiB or one that does not parse: **no row is
  written**, and the next ask tries again.

A stale row is served while its refresh is scheduled, without waiting for it.

Reading the cache never bypasses tenancy: the track is resolved through
`library_member`, with the caller's id as the first bind, before its key is
computed. A track the caller cannot see is 404 before anything is read or
sent.

## Decision 5 — served through the surfaces that already exist, and credited

- **Native**, `GET /api/v2/tracks/{id}/lyrics`: the sheet carries
  `source: "lrclib"`, beside the sources RFC-012 defined. The download route
  serves it as `.lrc` or `.txt`, like any scanned sheet.
- **Web client**: the properties panel names the source, "From LRCLIB", as it
  names every other one; the playing screen shows the sheet, synced when it
  is.
- **`getLyricsBySongId`**: the sheet, synced when it is, **without a source
  attribute** — the frozen `structuredLyrics` element has none, and none is
  added. A Subsonic client has no field to credit it in.
- **`getLyrics`**: unchanged when a visible track matching the artist and
  title exactly carries lyrics of its own. When none does, the first visible
  track matching exactly is looked up as above.

The façade's lyrics stay what RFC-012 left them: scanned sheets only, uploads
excluded. A track whose only sheet is an upload is therefore not looked up on
any surface (decision 2) and still answers no sheet on Subsonic, as it did
before this RFC.

**No observable change unless an operator switches it on.** With
`WAVEFLOW_LYRICS_PROVIDERS` empty, every surface answers what it answered
before, byte for byte.

## Decision 6 — statistics, per account, from `play_event`

`GET /api/v2/stats` answers one account's listening over a period:

| Parameter | Values |
| --- | --- |
| `period` | `7d`, `30d`, `365d` or `all` (default `30d`) |
| `utc_offset_minutes` | the client's offset from UTC, −840 to 840 (default 0), so a day is the listener's day |
| `limit` | how many entries each top list holds, 1 to 50 (default 10) |

It returns:

- **totals**: listens, listening time, distinct tracks, albums and artists;
- **top tracks, albums, artists and genres**, by listens, ties broken by the
  most recent listen and then by name;
- **a timeline**: listens and listening time per day for `7d` and `30d`, per
  month for `365d` and `all`, with empty buckets filled in for the three
  bounded periods so a chart has every day or month — `7d` and `30d` start at
  the listener's midnight six and twenty-nine days back, `365d` at the first
  of the month eleven months back;
- **the first and the last listen** of the period, with their track.

**A listen** is a `play_event` row with `submission = 1`: what scrobbling and
play counts already count. A "now playing" is not a listen.

**Listening time is an estimate**, and the response says how it is made: the
sum of the listened tracks' durations. No client reports how much of a track
was heard, and inventing a fraction would be worse than naming the measure.

**Tenancy is in the query.** Every statement starts from `play_event` joined
to `track` and to `library_member`, binds the caller's id first, and requires
`pe.user_id = m.user_id`: a user sees only their own listens, and only on
tracks in libraries they are still a member of and that are still available.
Leaving a library takes its listens out of the statistics, as it takes the
tracks out of every other answer; the rows are not deleted, so rejoining
brings them back.

**Artists and genres are grouped by canonical name**, because their rows are
per library: a musician present in two libraries is one entry, with the id of
one of its rows the caller can see. A track counts for its first credited
artist only, so a duet is not two listens, and for each of its genres.

The web client gains a **Statistics** page, under Library in the rail, with
the period as a choice and the timeline drawn as bars.

## Decision 7 — mixes, generated from local data only

`GET /api/v2/mixes` lists the mixes available to the caller today;
`GET /api/v2/mixes/{mix_id}` answers one with its tracks. Both take the
optional `library_id` every catalogue read takes, and `limit` (1 to 100,
default 50) bounds a mix's size.

| Mix id | What it holds |
| --- | --- |
| `most-played` | the caller's most played tracks, all time, best first |
| `never-played` | available tracks the caller has never listened to |
| `forgotten-favourites` | starred tracks, starred more than 30 days ago, not listened to in the last 90 days |
| `recently-added` | tracks added in the last 30 days, newest first |
| `genre:<name>` | one of the caller's three most listened genres over 90 days — or, with no history, the three largest |
| `decade:<year>` | one of the caller's two most listened decades over 90 days — or, with no history, the two largest |

**Variety.** No artist holds more than a tenth of a mix, and never fewer than
two of its tracks — the cap is `max(2, size / 10)`, counted on the track's
first credited artist. The order is then interleaved so two tracks by one
artist are not adjacent wherever another artist can stand between them.

**Deterministic per day.** A mix is drawn with a seed derived from the
account, the mix id and the day (in the caller's `utc_offset_minutes`), so it
is the same mix all day, on every device, and a new one tomorrow. The two
ranked mixes, `most-played` and `recently-added`, are ordered by their rank
and differ only as the history and the catalogue do.

**Bounded work.** A mix is chosen from a pool of at most 2,000 candidate
tracks read in SQL — ranked ones by their rank, random ones from a seeded
starting point in the track id space, which is uniform since track ids are
random — then ordered by a seeded hash, capped and interleaved in Rust, and
finally read through the shared song projection. A ten-thousand-track library
costs the same as a two-thousand-track one.

**A mix with no tracks does not exist.** It is left out of the list, and
asking for it answers 404, like a mix id the server does not know, or a genre
the caller cannot see.

The web client shows the list as **shelves** at the top of the home page,
each a card that plays the mix, and a page per mix.

**Not on Subsonic, in this RFC.** Three ways in were weighed:

- **Virtual playlists** in `getPlaylists`. Playlist ids are UUIDs on the frozen
  wire, clients cache playlists for offline use and re-download one whose
  content changes, and a mix changes every day; every write to one would have
  to be refused. Symfonium and DSub would start syncing six playlists a day.
- **New `getAlbumList2` types.** The type list is closed in the specification
  and the façade's ten are frozen; a client cannot discover an eleventh.
- **`getRandomSongs`**, which is already a random draw and is what a Subsonic
  client offers for "something to listen to".

None of them carries a mix without a change to frozen behaviour that no client
asked for. The façade is therefore unchanged by this decision; the question is
left open below.

## Decision 8 — similar songs fall back to local neighbours

`getSimilarSongs`, `getSimilarSongs2` and the new native route
`GET /api/v2/similar-songs/{id}` (an artist, album or track id, `limit` 0 to
500, default 50) share one implementation, `DomainServices::similar_songs`,
whose draw RFC-011 decision 7 describes. What changes is where the neighbours
come from.

1. **A configured provider comes first.** When the metadata cache names
   neighbours the caller can see, they are the neighbours, exactly as before.
2. **Otherwise the catalogue is asked**: with no provider configured, while a
   provider has not answered yet, and when it knows none the caller can see.

A local neighbour is another artist in the caller's visible, available
catalogue, scored against the seed artist:

| Signal | Score |
| --- | --- |
| credited on one of the seed's tracks, on the track itself or through its album (a duet, a feature, a guest) | 3 per such track, at most 9 |
| sharing genres | 2 per shared canonical genre, at most 6 |
| close era, by average year | 2 within five years, 1 within ten |
| listened to by the caller within 30 minutes after the seed, in the seed's last 500 listens | 1 per such listen, at most 5 |

An artist with neither a shared credit, a shared genre nor a co-listen is not
a neighbour, whatever its era. The twenty best, ties broken by name, then
take part in the same balanced draw as a provider's neighbours, with the seed
artist among them. A seed with no neighbour draws from its own tracks alone.

Every query binds the caller's id first and joins through `library_member`,
and the co-listening signal reads only the caller's own `play_event` rows: one
account's history never shapes another's answer.

**An observable change on a frozen method, made on purpose.** With no
provider configured, both methods answered an empty container for any id;
they now answer tracks. Filling a container the specification defines is
meeting it, not deviating from it — the argument RFC-011 decision 10 made for
the same two methods — and a client that calls them wants songs. The
compatibility matrix in `docs/subsonic-compatibility.md` records the change
and names it among the things to re-check in the next campaign.

## What this RFC changes elsewhere

- **RFC-002** — the sentence saying both similar-songs methods answer the
  empty container with no provider configured stops being true; this RFC is
  where that is said.
- **`docs/privacy.md`** — a section for LRCLIB: what is sent, when, and the
  variable that turns it off.
- **`docs/operations.md`** — switching LRCLIB on.
- **`docs/subsonic-compatibility.md`** — the lyrics methods (unchanged by
  default) and the similar-songs methods (changed by default).
- **`docs/api-v2-guide.md`** — the statistics, mixes and similar-songs routes.

## What is still open

- **Mixes on Subsonic**, if a client in the matrix turns out to want them —
  most likely as read-only playlists under derived ids, once a campaign
  measures how Symfonium and DSub treat a playlist whose content changes
  daily.
- **Uploaded lyrics on Subsonic.** RFC-012 kept them off the façade; a track
  whose only sheet is an upload answers none there, and is not looked up on
  LRCLIB either, because its owner already chose its words.
- **Eviction of `lyrics_lookup`.** Like RFC-011's cache it grows with what is
  played and never shrinks; a sweep can wait until a row count justifies it.
- **A per-account opt-out of statistics and mixes.** Both read only the
  account's own history and show it only to that account, so none is offered
  now.
