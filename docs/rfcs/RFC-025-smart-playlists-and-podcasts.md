# RFC-025 — Smart playlists and podcasts

- **Status**: Proposed
- **Implemented by**: [#333](https://github.com/InstaZDLL/waveflow-server/pull/333)
  (this RFC and its implementation, together), and
  [#342](https://github.com/InstaZDLL/waveflow-server/pull/342) (the amendment
  on artwork for podcasts and radio stations). The *Status* field above never
  flips in this project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (tenancy in the
  queries, static SQL, the frozen Subsonic contract),
  [RFC-020](RFC-020-playlists-and-shares.md) (public playlists, read through
  the reader's libraries), [RFC-011](RFC-011-metadata-enrichment.md) (the
  outbound rules and the address policy)

## Problem

Two features every comparable server has and this one did not.

1. **Smart playlists.** Navidrome, Ampache and the desktop client keep
   playlists defined by rules — "rock from the nineties I have not played this
   month" — that stay current without anybody editing them. Here a playlist
   was a list of tracks and nothing else. The feature is also where Ampache
   8.4.0 shipped an SQL injection: the logical operator joining the rules was
   taken from the request and concatenated into the statement.
2. **Podcasts.** The Subsonic façade answered every podcast method "not
   implemented" and reported `podcastRole="false"`, which was true. Symfonium,
   DSub and Feishin all offer podcasts to a server that has them.

Both are optional to an operator who does not want them: a smart playlist is
something an account makes, and podcasts are outbound traffic, off until an
operator switches them on.

## Decision 1 — a smart playlist is its own row, not a kind of playlist

A new table, `smart_playlist`, holds an owner, a name, a comment, the `public`
flag and the rules as JSON. It holds no tracks: what it contains is computed
each time it is read.

- **Not a flag on `playlist`.** A smart playlist has none of a playlist's
  verbs — no append, no removal by position, no M3U export, no reordering —
  and a flag on the shared table would have every one of those writes ask
  which kind it holds. Kept apart, the two meet only where they are listed
  together (decision 6), and the code that edits ordinary playlists does not
  change.
- **Not in the synchronization journal**, like radio stations. The journal's
  `entity_type` is CHECK-constrained; admitting a new one is a contract change
  the desktop client has to be ready for, and a mirrored smart playlist would
  be one it cannot evaluate.
- **A hundred per account.** `getPlaylists` lists every one an account can
  see without evaluating any (decision 4), so the bound is on the rows a
  listing reads and on what one account may publish, the way radio stations
  are bounded; the refusal is 409.

## Decision 2 — the rules, and the allowlist they are held to

```json
{
  "match": "all",
  "conditions": [
    { "field": "genre", "operator": "is", "value": "Rock" },
    { "field": "year", "operator": "between", "value": [1990, 1999] },
    { "field": "last_played", "operator": "not_in_the_last", "value": 30 }
  ],
  "sort": "play_count",
  "order": "desc",
  "limit": 50
}
```

| Kind | Fields | Operators | Value |
| --- | --- | --- | --- |
| text | `title`, `album` | `is`, `is_not`, `contains`, `not_contains`, `starts_with`, `ends_with` | text, 1–200 characters |
| text, several per track | `artist`, `album_artist`, `genre` | the same; a negative one holds when *no* value matches | the same |
| number | `year` (0–9999), `rating` (0–5), `play_count`, `duration` (seconds), `bitrate` (kbit/s) | `is`, `is_not`, `greater_than`, `less_than`, `between` | a whole number in range; `between` takes `[low, high]`, low first |
| moment | `last_played`, `date_added` | `in_the_last`, `not_in_the_last` | days, 1–36500 |
| | | `before`, `after` | a date, `YYYY-MM-DD`, midnight UTC |
| flag | `starred` | `is` | `true` or `false` |
| extension | `format` | `is`, `is_not` | lower-case letters and digits, 1–10 |
| identifier | `library` | `is`, `is_not` | a library id |

`match` is `all` or `any`, `all` when absent. `sort` is one of `title`,
`artist`, `album`, `year`, `rating`, `play_count`, `last_played`, `date_added`,
`duration`, `bitrate`, `random`, or absent for title order; `order` is `asc`
or `desc`. `limit` is 1–1000. At most twenty conditions; none at all is every
track, which is how "the hundred most recently added" is written.

- **Every key is checked, not only the ones read.** An object carrying a key
  this server does not know is refused, so a client sending a field a later
  version adds learns it is not supported instead of receiving a playlist that
  silently ignored it.
- **A refusal is a 422 that says where.** `{"code": "invalid_rules",
  "message": "does not apply to this field", "at": "conditions[2].operator"}`.
  The message is one of a fixed set of sentences: nothing the client sent is
  echoed back.
- **Only the parse builds the type.** `SmartRules::parse` is the one
  constructor of the rule type the service methods take; there is no route to
  the database with rules that skipped it. What is stored is the type's own
  serialization, never the request as it arrived, and a stored row that no
  longer parses is skipped rather than evaluated.
- Rating, star and play count are the **owner's** (decision 5).

## Decision 3 — one static statement, with the rules as one bound value

The project's rule is that SQL is static: literals assembled by `concat!`, so
a composition cannot be an injection. The brief offered two ways to keep it,
and this uses a third.

- **A fixed statement of optional predicates** — `(?1 IS NULL OR genre = ?1)`,
  one slot per field — cannot say what rules say. "Artist is Björk *or* artist
  is Radiohead" is two conditions on one field, `any` changes every `AND` to an
  `OR`, and the slots would have to be multiplied by the number of conditions
  allowed and by both connectives.
- **A candidate set filtered in Rust** answers anything, but only after loading
  the candidates. Bounded, it answers wrongly: "the fifty most played" over a
  bounded subset is the fifty most played of the subset. Unbounded, every read
  of every smart playlist loads the catalogue.
- **What is used: the conditions go in as one JSON document, and SQLite takes
  them apart.** `src/services/smart_playlists.rs` holds a single literal,
  `EVALUATE`. Its parameters are the owner's id, the connective, the
  conditions as one JSON value, their count, a sort code and the limit — and
  nothing else. `json_each` turns the document into rows; for each track, a
  `CASE` compares each row's field name against the string literal of every
  field the statement knows, and its operator against the literal of every
  comparison:

  ```sql
  … (p.mode = 'all' AND NOT EXISTS (SELECT 1 FROM cond c WHERE NOT <predicate>))
  OR (p.mode = 'any' AND EXISTS (SELECT 1 FROM cond c WHERE <predicate>)) …
  <predicate> = CASE c.f WHEN 'title' THEN (CASE c.o WHEN 'eq' THEN … END)
                         WHEN 'genre' THEN … END
  ```

  The sort is chosen the same way: one `ORDER BY` term per key and direction,
  each `CASE WHEN p.sort_key = n THEN column END`, with `NULLS LAST` on every
  one, so a track missing the value sorts after the ones that have it whichever
  way the list runs.

**Why the Ampache defect cannot happen here.** There are three layers, and
each would be enough on its own:

1. The connective is an enum parsed from exactly `"all"` or `"any"`; anything
   else is a 422 before the database is reached.
2. What is bound is re-serialized by this code from the enum — the literal
   `all` or `any` — not the request's bytes.
3. It is bound as a **value** and compared, `p.mode = 'all'`. Even a value that
   escaped both checks would be a string that equals neither literal, and
   matches nothing. The same holds for field names, operators and every
   operand: none of them is ever SQL text.

The conditions that reach the statement are themselves derived: the parse
validates the request, and the bound form — field, an elementary comparison,
the operand and its canonical form — is written by this code from the
validated types (`bound_conditions`). A test asserts the bound form carries
only the keys this code writes, another that every field and comparison the
parse admits is spelled in the statement, and a third that the sort codes and
the `ORDER BY` terms agree.

Text comparisons fold ASCII case, as `NOCASE` does everywhere else in the
catalogue. `is` on an artist or a genre compares canonical names —
`waveflow_core::scanner::canonical_name`, the folding `getGenres` and the
`byGenre` filter already use — so "hip hop" finds "Hip-Hop". `contains` uses
`instr`, not `LIKE`, so `%` and `_` in a value are only characters.

## Decision 4 — evaluated when opened, under a budget, and never by a listing

A smart playlist has no stored track list; reading one evaluates it. It is
current by construction — a play, a star or a scan shows on the next read with
nothing rebuilt — at the cost of the evaluation itself, and that cost is what
this decision bounds.

- **A listing evaluates nothing.** An earlier draft evaluated every visible
  smart playlist to report its count, and a security review named what that
  is: cross-tenant amplification. Any account could publish a hundred
  expensive public rule sets, and every other account's `getPlaylists` —
  which Subsonic clients call constantly — would run a hundred full-library
  queries. So `GET /api/v2/smart-playlists` and `getPlaylists` read the rows
  and nothing else. The counts they carry are **what this reader was last
  shown** of that playlist, remembered in memory when they opened it, and only
  while its rules are unchanged since (`updated_at` is part of the memory). A
  playlist this reader has not opened since the server started carries no
  count: `song_count`, `duration_ms` and `evaluated_at` are `null` natively,
  and Subsonic answers `songCount="0"` and `duration="0"` with no
  `validUntil` — the attribute that, when present, says when the count was
  computed. The memory holds at most 10,000 answers and is emptied when full;
  an answer it lost is a playlist listed without counts until it is opened.
- **Opening one evaluates it under a budget.** `getPlaylist`, the native read
  and the preview run the statement on a connection of their own with a
  SQLite progress handler: every thousand virtual-machine instructions it is
  asked whether to go on, and it stops the statement after 200 million
  instructions or ten seconds, whichever comes first. That is the bound on the
  rows considered, which a `LIMIT` cannot be — the rows are considered before
  they are sorted. A stopped evaluation answers **503** with
  `code: "evaluation_budget"` natively and error 0 in Subsonic, and nothing
  partial. The evaluation runs in a spawned task, so it always finishes what
  it started and removes its handler: were it cancelled with its request, the
  connection would go back to the pool still carrying an exhausted budget.
- **At most half the pool evaluates at once.** Each evaluation holds a
  connection for as long as its budget lets it, so a burst of them from any
  account could otherwise hold every connection the server has. A semaphore of
  `WAVEFLOW_DB_MAX_CONNECTIONS / 2` permits (at least one) is waited for before
  the evaluation starts and held by its task until it ends, request dropped or
  not. A read holds no connection while it waits or evaluates: it used to hold
  one across the evaluation, which takes a second, and as many concurrent reads
  as the pool has connections deadlocked it until the acquire timeout. A test
  runs twenty-four concurrent reads and previews against a pool of four.
- **At most 1,000 tracks** in an answer, the default when no limit is named;
  at most 100 smart playlists per account, public or not.
- **The cost is the projection's.** The statement wraps `song_select!`, whose
  play-count and last-played columns are correlated subqueries; a rule on them
  is evaluated per available track of the owner's libraries. That is the same
  cost as the existing "sort songs by play count" listing, and the budget is
  what stops a rule set from making it larger.
- A test lists thirty expensive public smart playlists of another account and
  asserts, through the services' evaluation counter, that the listing
  evaluated none; another lowers the budget and asserts the stop, and that
  every pooled connection answers afterwards.

## Decision 5 — evaluated as the owner, read as the viewer

A smart playlist is its owner's: "my most played" means the owner's plays, and
`starred`, `rating`, `play_count` and `last_played` read the owner's
annotations. The statement's first bind is the owner's id, and its tenancy is
the owner's `library_member` rows (checked twice: by `song_select!` and again
by the statement itself).

The answer is then read **through the viewer's own libraries**, by the same
lenient projection RFC-020 uses for a public playlist: a track in a library the
viewer is not a member of is not shown, not counted and not named. The stars,
ratings and play counts the viewer is shown are the viewer's own.

- **`public` works as it does for ordinary playlists** (RFC-020 decision 2): a
  public smart playlist is readable by every account, a disabled owner
  publishes nothing, and another account's private one is a 404 — the same 404
  a missing one answers.
- **Read-only to anyone but its owner.** Every write names the owner in SQL;
  anyone else's `PATCH` or `DELETE` is a 404.
- What a public smart playlist publishes is its rules' answer under the
  owner's history. That is the owner's choice to make, exactly as publishing an
  ordinary playlist of their favourites is.

## Decision 6 — the surfaces

- **Native.** `GET /api/v2/smart-playlists` (`?include_public=true` adds other
  accounts' public ones after the caller's own), `POST`, `GET`, `PATCH` and
  `DELETE` on `/api/v2/smart-playlists/{id}`, and `POST
  /api/v2/smart-playlists/preview`, which evaluates rules for the caller
  without storing them. A `PATCH` carrying `rules` replaces the whole rule set.
  Tagged `smart-playlists`, outside the operation-id protocol, as radio
  stations are.
- **Web client.** The playlists page lists smart playlists beside the ordinary
  ones, marked as such, with an editor whose controls are the allowlist:
  fields, then the comparisons that field admits, then a value of the right
  kind. It previews before saving. The `library` field is in the server's
  allowlist and not yet in the editor.
- **Subsonic.** `getPlaylists` lists them after the ordinary playlists,
  `getPlaylist` answers one with its entries. Each carries the OpenSubsonic
  `readonly="true"`, and `validUntil`, the moment its count was evaluated for
  this caller, so a client that caches it knows its copy is already due. In
  `getPlaylists` the count and duration are the caller's last `getPlaylist` of
  it, or zero with no `validUntil` before that (decision 4). `updatePlaylist`, `deletePlaylist` and `createPlaylist` with a
  `playlistId` reach ordinary playlists only, so naming a smart one answers
  error 70. **Ordinary playlists are unchanged on the wire**: they carry no
  `readonly` attribute, as before.
- **Navidrome `.nsp` import is not implemented.** The format's operators map
  onto most of this allowlist, but not all of them, and an import that dropped
  the conditions it could not read would be a playlist answering more than its
  author wrote. Left open, below.

## Decision 7 — podcasts are off by default, behind one switch

`WAVEFLOW_PODCASTS`, off by default, and an upgrade leaves it off. Off, no feed
is fetched, no episode is downloaded, and `podcastRole` is false for every
account; what was stored while it was on stays readable, and a downloaded
episode stays playable from disk.

| Variable | Default | |
| --- | --- | --- |
| `WAVEFLOW_PODCASTS` | off | the switch |
| `WAVEFLOW_PODCAST_REFRESH_INTERVAL_SECS` | 21600 | how often every channel is fetched again; `0` for never on its own; never more often than every 300 |
| `WAVEFLOW_PODCAST_ALLOW_HTTP` | on | whether a feed or an enclosure may be plain http |
| `WAVEFLOW_PODCAST_MAX_FEED_BYTES` | 16 MiB | the largest feed read |
| `WAVEFLOW_PODCAST_MAX_EPISODE_BYTES` | 1 GiB | the largest episode stored |
| `WAVEFLOW_PODCAST_QUOTA_BYTES` | 20 GiB | what the stored episodes may occupy together |
| `WAVEFLOW_PODCAST_FEED_TIMEOUT_SECS` | 20 | one feed fetch, redirects included |
| `WAVEFLOW_PODCAST_DOWNLOAD_TIMEOUT_SECS` | 1800 | one episode download |

Plain http is on by default because a good share of podcast enclosures are
still served that way, and a feed is public content; the address rules of
decision 9 apply to http and https alike. An operator who wants https only says
so.

## Decision 8 — one list per instance, managed by `podcastRole`

As in the reference server: an account that may manage podcasts subscribes,
and every account reads the channels and plays what is stored. A channel is
fetched once however many accounts listen to it, which keeps the outbound
traffic proportional to the channels rather than to the accounts.

- **`podcastRole` is one predicate**: the switch is on, and the account is an
  enabled administrator. It is what `getUser` and `getUsers` report, and what
  every write checks. There is no per-account role column to grant it with;
  adding one is a change to accounts rather than to podcasts, left open.
- **Refusals blur.** A write by an account without the role answers error 50
  in Subsonic and 404 natively, as every `Forbidden` does. `GET
  /api/v2/podcasts` says in advance whether the account may manage
  (`can_manage`) and whether the feature is on (`enabled`).
- **A feed address is shown only to a manager.** Private feeds — Patreon,
  Supercast, Substack and most paid feeds — carry the subscriber's token in
  the path as often as in the query (`/rss/<token>.xml`,
  `/feed/<user-token>/podcast.xml`), so an earlier draft that stripped the
  query alone, flagged by a security review, would have handed the token of
  whoever subscribed to every other account. Natively, `url` is `null` for an
  account that may not manage podcasts. Subsonic's schema requires `url` on a
  channel, so a reader is answered the feed's **scheme and host** —
  `https://feeds.example.org/` — and nothing after; the feed's own link is in
  `site_url`. The managers are the administrators (channels are per instance),
  and they are shown the address in full.
- **An enclosure address is shown to nobody.** It may carry the same token,
  and no surface needs it: an episode is played from the server (decision 11).
- **No feed or enclosure address reaches a log line**, on any path; a failure
  is logged as a word from decision 9's list. A test subscribes to a private
  feed whose path, query and enclosures carry tokens, and asserts that no read
  a second account can make — native or Subsonic, channels, episodes, newest,
  bookmarks — contains either token, while the manager's does.
- At most 500 channels on an instance: a refresh fetches every one.

## Decision 9 — every fetch, under the outbound rules

`src/podcast.rs` makes every podcast request through the shared client of
`src/outbound.rs` — no proxy from the environment, no redirect followed by the
client, a bounded connection — and adds what an address typed by a person and
addresses read out of responses need:

- **http or https only**, http only where allowed, **no user information** in
  the address, and a host.
- **Every address the name resolves to is public**, by the portrait fetcher's
  `AddressPolicy`: loopback, private, link-local, carrier-grade NAT,
  documentation and reserved ranges are refused, IPv4 inside IPv6 included,
  and IPv6 must be global unicast. **Checked before connecting, and pinned**
  (`ClientBuilder::resolve`), so the address checked is the address connected
  to.
- **Redirects are followed by hand, at most five**, and each hop is put through
  every rule above *before* it is contacted. A `302` to
  `http://169.254.169.254/` is refused as an address, never fetched.
  Enclosures routinely pass through a measurement prefix or two on their way
  to a CDN, which is why redirects are followed at all.
- **Bodies are bounded while they are read** — the declared length first, then
  every chunk, so a host that lies about its length is stopped where it passes
  the ceiling — and each request is bounded by its deadline.
- **Nothing about a request is logged** but a verdict from a fixed list, and a
  refresh logs counts. A feed address may carry a private token, and an answer
  is never stored as text: `error_code` is one of `destination_refused`,
  `address_refused`, `too_many_redirects`, `too_large`, `unreached`,
  `http_<status>`, `invalid_feed`, `not_audio`, `quota_exceeded`,
  `unavailable`.

**When.** When a manager subscribes (the feed is fetched and read before
anything is stored, so a typo leaves nothing behind), when a manager asks for a
refresh, every refresh interval — the first one interval after boot, so a boot
sends nothing — and when a manager asks for an episode. Nothing else, and no
request is ever made on a listener's behalf while they listen.

## Decision 10 — feeds are parsed as hostile documents

RSS 2.0, RSS 1.0 and Atom, read with `roxmltree`, already in the tree for TTML
lyrics:

- **A DTD is refused outright** (`allow_dtd: false`, written down rather than
  left to the default because it is the protection), so no entity can be
  declared, let alone expanded: the billion laughs and external entities are
  both a parse error. Feeds with a document type declaration are rare and are
  refused with the rest.
- **The text is bounded before it is parsed** (decision 9's ceiling), and the
  parse by a node ceiling of a million. Traversal is iterative, so nesting
  depth cannot reach the stack.
- **Text is kept as plain text**: markup inside a description is stripped,
  whitespace collapsed, titles cut at 500 characters and descriptions at 4,000.
  What a client is handed is a sentence, never markup a feed chose.
- **An item is an episode only if its enclosure is http(s)**, resolved against
  the feed; `file:`, `javascript:` and the rest are skipped. At most the 500
  newest are kept per refresh.
- **The cover is not fetched.** Its address is kept and handed to clients as
  `originalImageUrl`; channels carry no `coverArt`. Fetching artwork is a second
  outbound surface this RFC does not open. *Superseded by the
  [amendment](#amendment--artwork-for-podcasts-and-radio-stations) below: the
  cover is fetched, and its address is shown to managers only.*

## Decision 11 — episodes are downloaded, never relayed

The server downloads an episode into `<data dir>/podcasts` when a manager asks,
and plays it from disk like a track. It never relays a stream: proxying would
make every listen an outbound request at a listener's moment, with ranges to
forward, and the fetch rules above would have to hold under a seeking player.

- **The file name is derived**: the episode's own id and a suffix chosen from
  the first bytes — `mp3`, `m4a`, `ogg`, `flac`, `wav` or `aac`. Nothing a feed
  says names a path. A body that is not one of those, a page of HTML above all,
  is refused as `not_audio` and nothing is kept.
- **Written beside its final name and renamed**, hashed with BLAKE3 as it is
  written. The hash is the key the transcode cache files a conversion under, so
  `format=mp3` and `maxBitRate` work on an episode as on a track.
- **One download at a time, against a quota** of everything stored, and the
  per-episode ceiling. An episode that would pass the quota is refused as
  `quota_exceeded`.
- **Streaming reuses the media service.** Every stream route — the native
  ticket and `/api/v2/tracks/{id}/stream`, Subsonic `stream` and `download` —
  looks the id up as a track the caller can read, and otherwise as a stored
  episode, through one service method. The path guard is the same one tracks
  pass, rooted at the podcast directory.
- **Deleting an episode** removes its file and marks it `deleted`, so the next
  refresh does not bring it back. Deleting a channel removes every file.
- **At boot**, a download the previous process left running goes back to
  `new`, and its staging file is removed.

## Decision 12 — the Subsonic methods

`getPodcasts` (`includeEpisodes`, `id`), `getNewestPodcasts` (`count`, 20 by
default), `getPodcastEpisode` (OpenSubsonic), `refreshPodcasts`,
`createPodcastChannel`, `deletePodcastChannel`, `deletePodcastEpisode` and
`downloadPodcastEpisode`, in the schema's shapes:

- **`channel`**: `id`, `url` (the feed address to a manager, its scheme and
  host to anyone else — decision 8), `title`, `description`,
  `originalImageUrl`, `status` (`completed` or `error`) and `errorMessage` (the
  code).
- **`episode`**: a `Child` with `type="podcast"`, `channelId`, `parent` the
  channel, `album` and `artist` the channel's title, `publishDate`, `status`
  (`new`, `downloading`, `completed`, `error`), and `streamId` once it is
  stored — the episode's own id, which `stream` and `download` take.
- The five writes answer the bare envelope. A feed that cannot be read answers
  `createPodcastChannel` with error 0 and a generic sentence; the native route
  carries the reason.
- `podcasts.channel`, `channel.episode` and `newestPodcasts.episode` are JSON
  arrays at any length, from their first answer.
- `podcastRole` is reported by decision 8. **This is a wire change**: it was
  always `false`, and is now `true` for an administrator of a server with the
  switch on. Recorded in `docs/subsonic-compatibility.md`.
- No OpenSubsonic extension is advertised for it; none is defined.

## Decision 13 — progress is a bookmark

An account's position in an episode is kept by the bookmark surfaces it would
use for a track: native `PUT` and `DELETE /api/v2/bookmarks/{id}`, Subsonic
`createBookmark` and `deleteBookmark`, all accept an episode id. `getBookmarks`
lists episode bookmarks after the track ones, their entry the episode. Natively
an episode carries the reader's `position_ms`.

They are stored beside the track bookmarks, in `podcast_bookmark`, rather than
in them: `bookmark.track_id` is a foreign key to `track`, and a bookmark is a
synchronization entity whose journal entry the desktop client resolves against
tracks it knows. An episode bookmark is in neither the journal nor the
snapshot, and `GET /api/v2/bookmarks` still lists tracks only.

## Decision 14 — the web client's podcasts page

A page under *Listen*: the channels, a subscribe form and a refresh for a
manager, a channel's episodes with their state, and a player of the page's
own. An episode is not a track — no album, not scrobbled, not in the queue the
server keeps — so, like a radio station, it plays from its own element, and the
music player is paused when it starts. The position is written to the bookmark
every fifteen seconds and on pause, cleared when the episode ends, and read
back to resume. Leaving the page stops the episode, with its place kept.

## What does not change

- Ordinary playlists, their routes, their wire shapes and their journal
  entries.
- The bookmark contract for tracks, and the sync journal's vocabulary.
- Every Subsonic answer that existed, except `podcastRole` (decision 12) and
  the podcast methods that answered "not implemented".

## Logging

Nothing new is logged per request. A refresh logs how many channels it fetched
and how many failed; a failed download logs its code. No feed or enclosure
address is ever written, nor any part of a response.

## Tests

`tests/smart_playlists.rs` and `tests/podcasts.rs`, the second against a feed
host of the suite's own on loopback, never the internet: a feed larger than the
ceiling streamed with no declared length, one declaring entities, a redirect to
a private address for a feed and for an enclosure, a redirect loop, an
enclosure that is HTML, an episode past its ceiling and one past the quota.
`src/podcast.rs` and `src/services/smart_playlists.rs` carry the parse tests,
and the production address policy's refusal of loopback is the portrait
fetcher's own test.

## Amendment — artwork for podcasts and radio stations

*Added 2026-10-26.* The play bar showed a station or an episode with a
generic symbol, and the podcasts page showed no cover at all. The covers
existed — a feed names one, and a station may have a logo — but the web client
cannot load them: RFC-019's Content-Security-Policy keeps `img-src` to this
origin, and an image loaded from the feed's host would hand that host the
address of every listener, which decision 11 exists to prevent for the audio.
So the server fetches the image once and serves it from the artwork store,
like any cover.

**A1 — one fetcher, the podcast rules, the portrait checks.** A cover's address
comes out of a document, a station image's from a person, so neither can be
held to an allowlist as a provider's portrait is (RFC-011 decision 8).
`portrait::fetch_followed` reaches the host through decision 9's rules —
`podcast::open`: http or https, no user information, every address public,
checked and pinned, redirects followed by hand, at most five, each hop checked
before it is contacted — and reads the answer through the portrait fetcher's
own checks, now shared by both doors: 10 MiB, read with
`outbound::read_bounded_body`, and the bytes sniffed as JPEG, PNG or WebP. A
`Content-Type` naming another type is refused unread; one naming none, or
`application/octet-stream` — what object storage labels every cover with — is
read and judged by its bytes. A page of HTML is refused whatever it is
labelled. Nothing is logged but a verdict.

**A2 — a channel's cover.** Fetched when the channel is subscribed — in the
background, once the channel is committed, so that the subscription answers
with its cover pending rather than holding the request for a feed fetch and a
cover fetch in a row (Subsonic's `createPodcastChannel` sits under no
`TimeoutLayer`, so nothing else would bound that sum) — and on a refresh,
before the writer gate, recorded in the refresh's transaction:
`podcast_channel.artwork_hash`, with the address it was fetched from
(`artwork_url`) and when (`artwork_fetched_at`). A refresh fetches it again
only when the feed names an address other than the one fetched from, when no
cover is held (the last fetch failed), when the file is gone from the store,
or when the copy is older than a week; otherwise the feed is read and the cover
is not. A failed fetch stores nothing and keeps whatever was held — the old
cover, when the feed moved to a new one that cannot be reached — and the next
refresh tries again, because the addresses still differ. A feed that stops
naming a cover has it forgotten. Plain http is allowed for a cover exactly
when `WAVEFLOW_PODCAST_ALLOW_HTTP` allows it for the feed.

Two fetches of one channel's cover can be in flight at once — the
subscription's, in the background, and a refresh's — so the write that records
one names what its fetch assumed: the channel still names the address it came
from (or, to forget a cover, names none), and `artwork_fetched_at` is still
what it was when the fetch began. Otherwise the cover is dropped and the newer
state stands; its `artwork` row names nothing, and the sweep collects it.

**A3 — episodes show their channel's cover.** An episode's own
`itunes:image` is not fetched: a refresh keeps up to five hundred episodes, and
fetching one image each would make the cover traffic proportional to the
episodes rather than to the channels, which is what decision 8 keeps it to.

**A4 — what is handed out.** The hash, natively as `artwork_hash` on a channel
and on each episode, and through Subsonic as `coverArt` on `getPodcasts`,
`getNewestPodcasts`, `getPodcastEpisode` and an episode bookmark's entry,
served by `/api/v2/artwork/{hash}` and `getCoverArt` to every account, as every
account reads the channels. **The address is shown to managers only** —
natively `image_url`, through Subsonic `originalImageUrl` — as the feed's is
(decision 8): a private feed's cover can carry its subscriber's token. This
changes an answer: `originalImageUrl` was given to every account.

**A5 — a station's image.** A station may carry an `image_url`, validated as
`homepage_url` is: absolute, http or https, a blank value meaning none. When it
is set or changed through the native API, the server fetches it once under A1
— plain http allowed, as a station's stream so often is — and stores
`radio_station.artwork_hash`; an address already stored, with its image in the
store, is not fetched again. **A failed fetch keeps the address and shows no
image**, and the retry is the next save that carries the address; there is no
background pass, so a station image is never fetched without an account
asking. The image is served to the station's account alone, as the station is
(404 to everyone else). **A replacement does not clear the image by omission**:
`PUT` without `image_url` keeps it, and `null` clears it. The Subsonic writes
have no image parameter — `createInternetRadioStation` creates a station
without one and `updateInternetRadioStation` leaves it as it is — so renaming a
station in a Subsonic client cannot take its image away, and a native client
written before the field existed cannot either. `internetRadioStation` is
unchanged and carries no `coverArt`: the frozen contract defines none.

**A5b — nothing is fetched for a write that is refused, and a station image
has a budget.** Every check that can refuse a write runs, and the row is
committed, before any image is fetched: the address rules, the per-account
station ceiling, the station being the account's own (404 otherwise), and for
a channel the instance ceiling and the duplicate check — the ceiling is now
also asked before the *feed* is fetched. The image is then fetched, written to
the store, and attached by a second write that names the row (and, for a
station, its account and the address it was fetched for), so an image that
arrives after the row was deleted or its address changed attaches to nothing
and is swept. An account at its ceiling, or naming a station that is not its
own, makes the server contact nobody and write nothing. Station images are the
one fetch any account can ask for, so they are also bounded per account: at
most ten an hour (in memory, refilled by a restart), past which a station is
saved without an image and a later save retries. With an unchanged address
never fetched again, that caps the worst case at 100 MiB an hour per account,
which the sweep collects.

**A6 — one switch, on by default.** `WAVEFLOW_REMOTE_ARTWORK` (on) stops both
when it is off: the addresses are still stored, nothing is fetched, and images
already stored stay shown. On by default because nothing here happens on its
own: a cover only while `WAVEFLOW_PODCASTS` — off by default — is on, and a
station image only when an account sets one. That makes a station image the
one request a server at its defaults can be asked to make; `docs/privacy.md`
says so.

**A7 — the sweep.** `sweep_artwork_store` counts `podcast_channel.artwork_hash`
and `radio_station.artwork_hash` as references, so a cover named by either is
never collected, however old its file. Deleting the channel or the station —
or the feed or the account naming another image — releases it, and the next
sweep collects the row and the file. The window the sweep's module describes
(a row committed an instant after the sweep read the table) can carry off a
fetched file; the next write that names it asks whether the file is still
there before it trusts an unchanged address, and fetches it again.

**Tests.** `tests/podcasts.rs`, against its loopback host: a cover fetched,
served natively and through `getCoverArt`, and `coverArt` on channels and
episodes; an unchanged address not fetched again, a moved one fetched once
through a redirect and the old cover released; a cover over the ceiling,
declared and streamed; HTML labelled as an image and as HTML; redirects to a
private and to a mapped private address; an unlabelled JPEG accepted; a failed
fetch retried on the next refresh; a station image fetched, served to its
account only, kept by a native write without the field and by a Subsonic edit,
retried on the next save, cleared by `null`, and a foreign station's
replacement answered 404 with no request made; an account at its station
ceiling and an instance at its channel ceiling refused with no request to the
feed's or the image's host and nothing stored; and the sweep keeping both
images until the channel and the station are deleted.
`src/services/remote_artwork.rs` tests the hourly budget. A Subsonic
`createPodcastChannel` whose cover host never answers is answered within five
seconds, the channel created and its cover pending. The private-feed test's cover
now carries the subscriber's token, so it proves the address reaches no
reader. `src/services/podcasts.rs` tests when a cover is due.

## What is still open

- **Navidrome `.nsp` import** (decision 6).
- **`readonly` on ordinary playlists.** OpenSubsonic defines it for every
  playlist, and under the presence rule it should be emitted everywhere —
  `true` for another account's public playlist. It is emitted on smart ones
  only, so that no existing answer changes in this pull request.
- **A per-account `podcastRole`.** Only administrators manage podcasts; a role
  column on accounts would let an operator grant it.
- **An episode's own artwork** (`itunes:image` on an item): see A3.
- **Automatic downloads** of new episodes, per channel, and a per-channel
  refresh.
- **OPML import and export** of subscriptions.
- **The `library` field in the web editor.**
