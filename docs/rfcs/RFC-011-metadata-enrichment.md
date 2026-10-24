# RFC-011 — Server-side metadata enrichment

- **Status**: Proposed
- **Implemented by**: in progress, in the order of
  [the implementation plan](../metadata-enrichment-plan.md):
  InstaZDLL/WaveFlow#855 (the core's clients accept a client, an address and a
  key), #270 (the pin moved onto them), #271 (the cache and the engine that
  fills it), #272 (`getArtistInfo`, `getArtistInfo2` and the native artist
  page read it), #273 and #274 (`getSimilarSongs` and `getSimilarSongs2` draw
  from it), #275 (portraits from Deezer), and the pull request closing #269
  (the web client's artist page, and `enrichment.image` on the native API,
  which #275 had left out). The compatibility campaign is still to come. The
  *Status* field
  above never flips in this project; this line is the one that says what is
  running.
- **Date**: 2026-10-14
- **Revised**: 2026-10-15, while implementing the cache (#265). Decisions 4, 6
  and 10 rested on attachment points the code does not hold; each correction
  below is marked and says what the first version asserted, rather than
  erasing it.
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-010](RFC-010-external-scrobbling.md)
- **Amends**: RFC-002, which states that "WaveFlow queries no remote source".
  That sentence was a decision, not an observation, and this document is where
  it is revisited rather than quietly contradicted.
- **Closes**: [#218](https://github.com/InstaZDLL/waveflow-server/issues/218),
  the last open issue of the #206-#223 plan, and with it the last Subsonic
  containers that answer empty for want of a source.

## Problem

Four Subsonic methods answer an empty container because the server has nothing
to put in them: `getArtistInfo`, `getArtistInfo2`, `getSimilarSongs`,
`getSimilarSongs2`. `getAlbumInfo` and `getAlbumInfo2` carry one real value, the
release identifier, and nothing else. The reason is written in
`src/subsonic/browse.rs` and in RFC-002: the server asks nobody.

Issue #218 frames the way out as a binary choice — host the desktop's plugin
engine, or write a Last.fm client. Both halves of that framing needed
correcting before anything could be decided.

**The desktop does not have the capability the issue credits it with.** Its
plugin SDK declares `artist-info: func(name: string) -> result<artist-details,
string>` in all three versions of the `metadata` world, and
`record artist-details { bio, image-url, similar }` with it. The host never
calls it. `src-tauri/crates/core/src/plugin/runtime.rs` holds five `metadata_*`
helpers — `album-info` in v1, v2 and v3, `lyrics` in v2 and v3 — and no
`artist_info` among them. A WIT contract is a declaration, not an
implementation, exactly as an RFC's *Status* field is not evidence about the
code.

**The capability that does exist is native, and it is already linked into this
binary.** `waveflow-core` exposes `pub mod metadata`, outside any feature gate,
and this repository already depends on that crate (`Cargo.toml`, rev
`012be48`). Inside it:

| module | what it provides |
|---|---|
| `lastfm.rs` | `artist_get_info` (bio), `artist_get_similar` (ranked neighbours) |
| `theaudiodb.rs` | `artist_info` — bio in several languages, plus fanart |
| `deezer.rs` | artist photos and related artists |
| `lrclib.rs` | lyrics |
| `name_match.rs` | fuzzy artist-name matching: case, punctuation, diacritics, canonical "& The Wailers" suffixes |
| `album_match.rs` | the same problem for albums |

Its module header says what it was written for: *"designed to be reusable from
both the desktop app and the future `waveflow-server` without any glue"*.

So the question is not how to acquire the capability. It is what the server is
allowed to do with one it already carries.

## Decision 1 — the server enriches, because only the server serves every client

The alternative deserves to be stated, because it is cheap and it was real: let
each client enrich for itself. The desktop already contacts TheAudioDB and
Deezer directly, so a WaveFlow user running the desktop app already has their
biographies today, with the server untouched.

What that leaves out is everybody else. The embedded web client has no
enrichment path of its own and no business growing one. Third-party Subsonic
clients — DSub, Symfonium, Feishin — call `getArtistInfo` and render whatever
the server returns; they cannot be asked to go and look elsewhere. And every
client that did enrich for itself would ask the same question of the same
provider, once per client and once per device, against rate limits sized for
one caller.

Enriched once, cached once, visible everywhere. That is the whole argument, and
it is sufficient.

## Decision 2 — no plugin engine: the providers are native

Issue #218 proposes embedding Wasmtime so the server can run the desktop's
metadata plugins. This RFC declines, and the decision is not a new one — it is
already written in the dependency both repositories share.
`waveflow-core`'s `Cargo.toml` gates its plugin host behind a `plugins`
feature, with this comment:

> The desktop app (`crates/app`) enables this; `waveflow-server` does not (the
> HTTP/WS surface doesn't run guest WASM in v1, so the ~5 MiB Cranelift codegen
> tree would be pure bloat).

Three things follow, and together they settle it.

**The cost is real and the benefit is hypothetical.** Cranelift and
`wasmtime-wasi` are several megabytes of code generation in a binary whose
selling point, stated in `CLAUDE.md`, is that it is one axum binary. Against
that: the registry today holds `apple-artwork` (`waveflow:metadata/v1`, animated
covers) and `release-radar` (`waveflow:ui/v1`). Neither answers the question
this RFC exists for. There is no artist-biography plugin to run.

**A sandbox is a threat model, not a feature.** Guest WASM in the server means
fuel or epoch interruption, a memory ceiling, a host allowlist for `http`, and
a story for what a runaway guest does to the writer gate and to scan
parallelism. That is a body of work with its own RFC, and spending it to run
zero existing plugins is the wrong order.

**Deferring costs nothing.** This is the part that makes the decision easy: the
engine is not an integration to build later, it is a feature flag on a
dependency already in the graph. If genuinely useful metadata plugins appear,
turning `plugins` on and wiring the `metadata` world is a bounded piece of work
that this decision does not make harder. The `ui` and `canvas` worlds are a
different matter and stay out of scope entirely — `canvas` overlaps
[RFC-009](RFC-009-track-canvas.md), which already shipped.

## Decision 3 — off until an operator switches it on, and separate from scrobbling

A self-hosted server that silently starts talking to Last.fm has broken a
promise its operator never agreed to revisit. Enrichment is therefore **off by
default**, and it stays off through an upgrade: a server that is merely updated
makes no outbound metadata request at all, exactly as RFC-010 says of
scrobbling.

**It is a separate switch from scrobbling, and that is deliberate.** This server
already holds `scrobbling.lastfm: Option<LastFmApplication>` — an operator who
runs Last.fm scrobbling has already placed an API key that `artist.getInfo`
would accept. Reusing it automatically would mean that enabling outbound
*submissions on behalf of one account* silently enables outbound *lookups about
the whole catalogue*. Those are two consents. The operator may supply the same
key value to both settings; they switch on independently.

Settings, under `WAVEFLOW_METADATA_*`, in `src/config.rs` with the rest:

- `WAVEFLOW_METADATA_PROVIDERS` — an ordered, comma-separated list, empty by
  default. Empty is the off switch; there is no second boolean to disagree
  with it.
- `WAVEFLOW_METADATA_IMAGES` — on once providers are configured, `off` to keep
  the lookups and refuse decision 8's image fetch. It is a narrowing, never a
  widening: an empty provider list means no outbound request of any kind, and
  this setting cannot contradict it.
- `WAVEFLOW_METADATA_LASTFM_KEY` — the operator's own application key.
- `WAVEFLOW_METADATA_THEAUDIODB_KEY` — added by
  [RFC-014](RFC-014-automatic-artist-banner.md) for the `theaudiodb`
  provider, which is asked for artist banners only, under the same rule: no
  key ships in the binary.
- `WAVEFLOW_METADATA_TTL_DAYS`, `WAVEFLOW_METADATA_MISS_TTL_DAYS`,
  `WAVEFLOW_METADATA_RATE_PER_SEC`, `WAVEFLOW_METADATA_REQUEST_TIMEOUT_SECS`.

**No embedded key.** `waveflow-core` reaches TheAudioDB with
`FREE_API_KEY = "123"`, their public test key. That is a defensible choice for
one desktop user and not one for a server: the quota is shared with the entire
internet, and RFC-010 decision 4 already settled the general form — this
repository is published under AGPL, and a secret in a public binary is not a
secret. A provider is available when the operator has placed a key for it.

## Decision 4 — the cache is keyed by provider and canonical name, and it is instance-wide

A biography is public. It is the same text whoever asks, it belongs to no
library and to no account, and the question "which tenant does this artist's
bio belong to?" has no answer worth computing.

This matters because `artist` rows in this schema are **per library**:
`artist_select!` joins through `library_member` on `ar.library_id`, so the same
musician is a different row in each library that holds them. Keying a cache on
`artist.id` would fetch the same bio once per library and store it that many
times, for no gain.

The cache is therefore keyed on the **canonical name**, as the desktop's own
`app.lastfm_similar` cache already is, and sits in new dated migrations under
`migrations-v2/`:

- `metadata_artist` — `(provider, name_canonical)` as primary key, plus `bio`,
  `image_url`, `outcome`, `fetched_at`, `expires_at`.
- `metadata_artist_similar` — `(provider, name_canonical)`,
  `similar_canonical`, `rank`, `score`; the ranked neighbour list as that
  provider gave it, resolved to local artists only at read time (decision 7).
- `metadata_album` — the same shape for album notes, keyed on the provider and
  the canonical `(artist, title)` pair that `album_match.rs` already computes.

**Revised on 2026-10-15.** The migration that shipped
(`20261015000000_metadata_artist.sql`) differs from this list in three places.
The biography is two columns, `bio_summary` and `bio_full`, because that is how
the providers return it. `image_url` is not there: portraits add their columns
with the fetch that fills them. And `metadata_album` was not created — the
open question on album notes below decides whether it exists, and
`album_match.rs` was not even in the pinned core when this was written.

### One row per provider, merged on read

**The provider belongs in the key, not in a column**, and the first draft of
this decision had it wrong. Decision 3 lets an operator order several
providers, and decision 8 assumes a division of labour that is real — Last.fm
has the better biographies and the ranked neighbours, TheAudioDB has the
images. A single row per name cannot hold that: it would have to pick a winner
at write time, before knowing what the other providers would have said.

Worse, it would make a negative answer contagious. Last.fm not knowing an
artist would stamp the one row with a miss, and TheAudioDB — which may well
have a biography and a portrait for them — would never be asked again until the
miss expired. Decision 5's whole point is to cache a negative answer; caching it
against the *wrong* subject turns a useful optimisation into a silent gap.

So each provider keeps its own row, with its own outcome and its own clock, and
the merge happens at read time:

- **Field by field, in the operator's order.** The first provider with a
  biography supplies the biography; the first with an image supplies the image.
  A provider that is absent, missing or stale on one field does not disqualify
  what it holds for another.
- **Provenance is per field**, because the merge is. An answer whose bio comes
  from Last.fm and whose portrait comes from TheAudioDB says exactly that —
  which is decision 11's job, and what makes it honest rather than decorative.
- **Reordering the list changes the answer and nothing else.** No cache is
  invalidated when an operator changes their preference: the rows are facts
  about what each provider said, and the order is a question asked of them.

Normalisation is `waveflow_core::metadata::name_match::normalize_name`, and
matching a provider's ranked candidate list back to the name asked for is
`select_by_name`. Neither is re-implemented here; both have a desktop issue
behind them (#342) recording what naive equality dropped.

**Revised on 2026-10-15: the key is not `normalize_name`.** The paragraph above
asserted it was. Every `artist` row already stores a `canonical_name`, computed
by `waveflow_core::scanner::canonical_name`, and the two functions disagree:
`normalize_name` also folds diacritics and turns `&` into `and`, so "Björk" is
`björk` in the column and `bjork` through it. The cache is keyed on the
column's form, which lets the read path join
`metadata_artist.name_canonical = artist.canonical_name` with nothing
recomputed. `normalize_name` and `select_by_name` keep the job they were written
for — matching a provider's candidates — where a provider returns candidates.

**Nothing in these tables is account data**, which is what makes an
instance-wide cache correct rather than merely convenient. Anything that ever
becomes account-specific — a per-person override, a hidden artist — belongs in
a different table with a user id in it, and does not get added to these.

## Decision 5 — a miss is an answer; a failure to ask is not

The desktop already learned this one and the lesson is worth importing verbatim.
In `commands/deezer.rs`, a lookup that **reached** the provider and found
nothing stamps the cache; a transport error leaves the timestamp NULL so the
next visit retries. Without that distinction a thirty-second network blip
becomes "this artist has no biography" for the whole TTL.

So three outcomes, written on **that provider's** row — decision 4 keys them
per provider precisely so that one provider's silence is not read as the
world's:

- **hit** — content, cached until `expires_at` (default 30 days).
- **miss** — the provider answered and has nothing. Also cached, for a shorter
  window (default 7 days), because re-asking a rate-limited API on every page
  view about an artist it has never heard of is the failure mode this column
  exists to prevent.
- **unreached** — no row is written at all. The next request tries again.

Stale is served rather than withheld: past `expires_at` the cached value is
still returned and a refresh is scheduled. A slightly old biography beats none,
and it keeps a provider outage invisible.

## Decision 6 — the lookup never happens on the answering path

`getArtistInfo` is a synchronous call on a frozen contract, and the temptation
is to block it on the outbound request so the first visit shows a biography.
This RFC refuses.

**The answer comes from the cache.** On a miss the handler returns the container
it would have returned anyway and schedules the fetch; the next visit has the
content. The cost is named rather than hidden: the first time anyone opens an
artist page, there is no bio.

Three reasons, in order of weight:

1. `CLAUDE.md` is explicit that the writer gate is never held across network
   I/O. A fetch-then-write inside a request handler is the shape that invites
   exactly that mistake.
2. A provider that is slow makes the façade slow for every client, on a method
   clients call eagerly the moment an artist page opens.
3. Rate limits are per server, not per request. Last.fm allows roughly five
   calls a second; a page that opens twenty artists would spend four seconds of
   the instance's whole budget.

**One outbound call per name in flight.** Concurrent requests for the same
artist collapse onto one fetch — single-flight, keyed on the canonical name.
Ten clients opening the same page is one call.

**No prefill at scan time.** A first scan of a ten-thousand-artist library would
be two thousand seconds of saturated budget for data nobody has asked for.
Enrichment is demand-driven, and a library nobody browses costs nothing.

**And the client is told when it lands**, rather than left to poll or to wait
for someone to reload the page. The mechanism already exists and decision 8
explains why it may now carry an artist: RFC-007's library event stream, whose
decision 6 declined an artist event for a reason this RFC removes.

**Revised on 2026-10-15: only one client is told.** The paragraph above implied
every client is. The desktop is, because it mirrors the library through that
feed. The web client is not: the only event stream it reads is a scan's
progress. It re-asks instead, a bounded number of times while the answer says
the lookup is pending, and then stops — which is the web client's step of the
plan, not a property this mechanism gives it.

## Decision 7 — similar artists resolve to artists the caller can see, and to nothing else

`artist.getSimilar` returns names. Subsonic's `similarArtist` entries carry an
**id**, and the id is where a leak would happen: resolving a name against the
whole catalogue would hand a caller the identifier of an artist living in a
library they are not a member of, and `getArtist` on it would then answer 404 —
the server having already confirmed, by returning the id, that the row exists.

So the resolution runs through `library_member` with the caller's id as the
first bind, like every other query in `src/services/`. A neighbour that resolves
to an artist the caller can see is returned with its id. **A neighbour that does
not resolve is dropped**, rather than returned with a null or sentinel id: an
entry a client cannot follow is an entry that makes the client look broken.

### What the two song methods actually ask for

The same resolution feeds `getSimilarSongs` and `getSimilarSongs2`, but their
parameters are not the same and the first draft of this decision treated them
as if they were.

- **`getSimilarSongs2` takes an artist id.** One kind, and nothing else.
- **`getSimilarSongs` takes "the artist, album or song ID".** Three kinds
  through one parameter. An album id resolves to its album artist, a song id to
  its track artist, and both then take the artist path. OpenSubsonic's own
  clarification says the two methods return no different result when given an
  artist id, so there is one implementation and two entry points, not two
  implementations.
- **The selection is random, and it includes the artist themselves.** The
  specification reads "a random collection of songs from the given artist *and*
  similar artists" — not the neighbours alone, which is what the first draft
  said. `count` bounds it, and its default is **50**. *(Revised with #273's
  follow-up: the draw takes turns across artists rather than drawing from one
  pool of tracks, so an artist with two hundred tracks beside neighbours with
  ten each does not fill the answer with their own songs.)*

Both are in the compatibility matrix with real callers behind them: Feishin
calls `getSimilarSongs`, and without the `.view` suffix, which the router
already accepts; Juliet calls `getSimilarSongs2`. Neither has ever seen these
methods answer anything.

The id is resolved before anything else and for its refusal, as `album_info`
already does: an id naming something the caller cannot reach answers exactly as
one naming nothing at all.

This is also what keeps the feature from becoming a discovery service for music
nobody owns. The server answers about the collection it holds, enriched; it does
not advertise a catalogue.

## Decision 8 — the server fetches artist images, and #217 does not forbid it

Issue #217 settled that the server never goes and fetches an internet radio
stream: it stores the address and hands it to the client, which keeps it from
being a request forger aimed at its own private network. Artist images look like
the same question and are not quite, and the difference is worth stating because
this is the only place this RFC reverses a stance.

**In #217 the target is chosen by the user.** Someone types a URL and the server
dereferences it — that is the SSRF primitive itself, and refusing was right.
Here the target comes out of a response from a host the **operator** opted into,
reached at a base URL that is a compile-time constant in `waveflow-core`. The
user chooses nothing.

**And the privacy argument runs the other way.** Passing a third-party image URL
through to clients means every browser and every phone that opens an artist page
announces itself to TheAudioDB — the viewer's address and what they are
listening to, leaked by the self-hosted server that existed to avoid precisely
that.

The server therefore fetches the image once and serves it from its own store,
through the content-addressed artwork store that `artwork_dir` and
[RFC-009](RFC-009-track-canvas.md) already provide — hash, format, byte size,
created at. The fetcher is narrow: **https only**, **no redirect followed**
(`Policy::none()`, as RFC-010 decision 10 already requires), **private and
loopback address ranges refused**, **content type restricted to the image set
`read_artwork` already knows**, **size and timeout bounded**.

**A provider's response is still untrusted input**, and the paragraph above is
not enough on its own. Two more conditions, both cheap:

- **The host is on an allowlist**, declared per provider beside its base URL —
  TheAudioDB and Deezer each serve their images from a small, stable set of
  domains. An image URL pointing anywhere else is dropped, not fetched. This is
  the control that actually matters: it means a compromised or merely creative
  provider cannot nominate a target at all.
- **The address checked is the address connected to.** Refusing private ranges
  after a DNS lookup, then letting the client resolve the name a second time,
  checks one answer and dereferences another. The resolved address is pinned
  for the connection it was validated for.

Last.fm's placeholder star is not an image and is discarded —
`waveflow_core::metadata::lastfm::is_lastfm_star_placeholder` exists for this.

### The route already exists, and so does the event this reopens

`artist.artwork_hash` is read in four places — the artwork sweep, cover-art
resolution in `src/services/catalog.rs`, and the `coverArt` attribute that
`src/subsonic/nodes.rs` already emits on every artist — and **written nowhere**.
An artist portrait therefore needs no new route and no new column: filling that
hash makes it serve itself through the cover-art path every client already
calls.

It also settles a question RFC-007 left open rather than closed.
Its decision 6 declines to emit a library event for an artist, and gives the
reason: "the desktop reserved one case — the artist photo, which derives from
no `GROUP BY` — and that case does not exist: `artist.artwork_hash` is read all
over this server and written nowhere. An artist photo cannot change because
there never is one. **The question reopens the day something writes it, and on
that day this is the line to re-read.**"

This is that day. Decision 8 writes it, so an artist event now has the reason
RFC-007 found it lacked, and it is also the answer to how a client learns that
an enrichment scheduled by decision 6 has arrived — rather than polling, or
leaving a page stale until someone reloads it. The event carries no payload,
like the album's: what describes an artist is on the artist, which the client
re-reads.

**The escape hatch is the operator's**, as in RFC-010: an instance that would
rather not spend the disk sets `WAVEFLOW_METADATA_IMAGES=off` and the three
`*ImageUrl` fields stay absent. Absent, not passed through — a half-measure that
leaks the viewer's address is worse than the empty element clients already cope
with.

## Decision 9 — the outbound surface

RFC-010 decision 10 is the governing text and most of it applies unchanged. What
differs is stated here rather than reasoned out again.

- **The base URLs are constants**, not settings. `waveflow-core` holds them;
  there is no operator field and therefore no URL validation problem for the
  lookups themselves. Decision 8 covers the one dereference that follows data.
- **No redirect followed**, for the lookups and for the image fetch alike.
- **Timeouts and response size bounded**, like every other I/O here.
- **Nothing from a response body reaches a client.** A provider failure is a log
  line and an absent element, never a 500 and never an echo.
- **Never on the Subsonic error path.** An enrichment failure must not turn a
  working `getArtistInfo` into anything other than the container it answers
  today.
- **The log rule holds without exception.** `CLAUDE.md` forbids logging headers,
  query strings, tokens and passwords; a Last.fm lookup carries the API key as a
  query parameter, so what is logged is the method and a verdict, never the URL.

## Decision 10 — what the façade answers, and what must be re-validated

The Subsonic contract is frozen for v2.0-beta. Filling these containers is not a
deviation from the specification — it is finally meeting it — but it **is** a
change in observable wire behaviour against clients that were validated seeing
them empty.

- `getArtistInfo` / `getArtistInfo2` — `biography`, `similarArtist` entries
  (decision 7), and `smallImageUrl` / `mediumImageUrl` / `largeImageUrl`
  pointing at this server's own artwork route (decision 8).

  **Revised on 2026-10-15: the three image URLs stay absent.** The bullet above
  asserted a route they could point at. Neither artwork route serves a URL a
  client loads as-is: `media::artwork` wants the native bearer, and
  `getCoverArt` wants Subsonic credentials in the query string, which would put
  a member's token into a URL that clients cache and log. A client finds the
  portrait through `coverArt`, which every artist element already carries. The
  compatibility campaign measures whether a client in the matrix reads
  `largeImageUrl` at all; a sealed artwork ticket can be added then, while a URL
  scheme cannot be withdrawn once clients have cached it.
- `getSimilarSongs` / `getSimilarSongs2` — tracks by the neighbours the caller
  can see.
- `getAlbumInfo` / `getAlbumInfo2` — `notes` beside the `musicBrainzId` they
  already carry. The element-not-attribute shape stays as RFC-002 records it.
- `getTopSongs` does not move. #215 settled that it answers this account's own
  listening, from `play_event`, and a remote popularity ranking is not the same
  question.

**The compatibility matrix must be re-run before the next tag**, as with the
four accepted deviations already listed in RFC-002.
`docs/subsonic-compatibility.md` names the clients that call these methods; a
container that was always empty is the kind of thing a client parses loosely
until the day it stops being empty.

## Decision 11 — provenance travels with the data

Every cached row names the provider it came from, and the native API publishes
it. A biography that appears in a client without saying where it came from is
both a licensing problem — Last.fm's terms ask for attribution, and TheAudioDB's
ask for it too — and an honesty problem the moment two providers disagree.

On `/api/v2`, artist detail grows an `enrichment` object: `bio`, `image`,
`similar`, and a `source` **per field** — decision 4 merges across providers, so
a single source for the whole object would be a lie the moment the biography and
the portrait come from different places. Each names its provider and its fetch
date.

**And it carries an explicit state**, because absence cannot be made to mean one
thing. The first draft claimed a missing object let a client tell "not enabled"
from "nothing found"; it does not — absence would also cover "enabled, asked,
still waiting" and "enabled, asked, the provider has nothing", which are four
situations and one signal. So the object is always present when the caller can
see the artist, and says which it is:

| state | meaning |
|---|---|
| `disabled` | no provider is configured; nothing has been asked and nothing will be |
| `pending` | a lookup is scheduled or in flight — decision 6's first visit |
| `ready` | content, with its per-field provenance |
| `none` | every configured provider answered and none of them knows this artist |

`unreached` is deliberately not a fifth state. Decision 5 writes no row for it,
so from the outside it is `pending` — which is what it is: the next request will
try again. A client that renders a spinner for `pending` and a blank for `none`
is doing the right thing in both cases.

The Subsonic façade publishes none of this. Its containers have no field for it
and the contract is frozen; a client there sees an empty element on the first
visit and a filled one later, which is all the protocol can express.

Nothing here is writable through the API. There is **no ingestion route** by
which a desktop client or a companion process could push enrichment into the
server. That surface was considered and is refused for now: it is an
authenticated write path, with validation, provenance and tenancy questions of
its own, built for a client that does not exist. It belongs in its own RFC, if
anything ever needs it.

## What this RFC changes elsewhere

- **RFC-002** — the sentence "WaveFlow queries no remote source" is amended to
  name this document and the condition under which it is no longer true. It
  appears in that file and again in `src/subsonic/browse.rs`; both move together
  or the code and the design disagree.
- **`src/subsonic/browse.rs`** — the doc comment on `album_info`, and
  `artist_info`, whose whole body is currently a tenancy check followed by an
  empty node.
- **`src/subsonic/mod.rs`** — `getSimilarSongs` and `getSimilarSongs2` stop
  being literal empty nodes.
- **RFC-007** — its decision 6 refuses an artist event on a condition this RFC
  removes, and says so in the line that names the day it reopens. Decision 8
  above is that day; RFC-007 gains an artist event rather than being
  contradicted. Nothing else in it moves: the two streams, the membership
  check on read and the bounds are untouched.
- **`docs/subsonic-compatibility.md`** and **`docs/subsonic-api-guide.md`** —
  the rows for the six methods, and a note saying when they were last measured.
- **`docs/operations.md`** — a section on switching enrichment on: which key,
  which provider, what the server starts contacting, and what it stores.
- **`CLAUDE.md`** — the "one axum binary" claim is **unaffected**, which is
  decision 2's doing and worth recording as such.

## What is still open

- **Which provider ships first.** Last.fm gives bios and ranked neighbours from
  one key and is the obvious opening move; TheAudioDB gives better images and
  several languages. The ordered list in decision 3 lets an operator express a
  preference, but the shipped default order is not settled here.
- **Language.** `theaudiodb.rs` selects a biography by language and falls back
  to English. Nothing in this server currently knows a user's language, and
  decision 4's cache is keyed without one. A per-language cache key is the
  obvious extension and is deliberately not taken now.
- **Album notes may not be worth the second provider.** Last.fm's album wiki is
  thinner than its artist bio. If the first measurement says it is mostly empty,
  `getAlbumInfo` keeps carrying its release identifier alone and loses nothing.
- **Eviction.** The cache grows with the catalogue and never shrinks. A sweep
  like RFC-009 decision 6's is the shape, and it can wait until a row count
  justifies it.
