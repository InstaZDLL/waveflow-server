# Implementing RFC-011: what the code holds, and an order

> Survey of 2026-10-14, read-only against `main` at `26a7f2b`, and against
> `InstaZDLL/WaveFlow` both at the revision this server pins (`012be48`) and at
> its current `main`. Sources: the server's code and migrations, the desktop
> crate's sources fetched at each revision, and the OpenSubsonic documentation
> for the two `getSimilarSongs` methods. What was not verified is listed at the
> end and says so.
>
> No code was modified during this survey.

[RFC-011](rfcs/RFC-011-metadata-enrichment.md) was written from the shape of
`waveflow-core`'s metadata module and of this server's surfaces. Planning the
work meant checking each attachment point it names, and six of them do not
hold as written. None of them changes a decision; four of them change the
order of the work, and one puts the first pull request in the other repository.

## 1. What the RFC assumed and the code does not hold

**1. The core's clients cannot be used as they are.** `LastfmClient::new()` and
`TheAudioDbClient::new()` each build their own `reqwest::Client`: a five-second
timeout, the default redirect policy — which follows up to ten — and a
`.json()` read with no ceiling on the body. Decision 9 forbids the first and
bounds the second. The server already has the right client,
`scrobblers::outbound_client`, built with `Policy::none()`; the core's clients
offer no way to be handed it.

The same constructors fix the base URL as a constant. That is what decision 9
wants for production and what makes this server's suite unable to test any of
it: every test here is hermetic, and the scrobbling tests reach their fake
providers by pointing a destination at a local `axum` listener — which works
there only because a scrobbling destination is an operator URL. A metadata base
URL is not a setting, so without a seam in the core there is nothing to point.

**2. TheAudioDB takes no key.** `artist_info(name, lang)` builds
`{BASE_URL}/{FREE_API_KEY}/search.php` with `FREE_API_KEY = "123"`. Decision 3
refuses that key for a server; the client gives no way to pass another one.

**3. The pin is 946 commits behind the core's `main`**, and 52 files under
`crates/core/` changed in between. `album_match.rs`, which the RFC lists among
what is "already linked into this binary", **does not exist at the pinned
revision** — it was added since. `lastfm.rs`, `theaudiodb.rs` and
`name_match.rs` do exist there, with the signatures the RFC quotes. Whatever
core change item 1 requires, consuming it means moving the pin across those
946 commits, which is a piece of work with its own risk and its own pull
request. The core passes `lofty` types through its API, so this server and the
core must resolve to one `lofty` version: a move that leaves two in the graph
breaks the scanner, and has before.

**4. There are two name normalisations, and the RFC names the wrong one for the
key.** Decision 4 says the cache is keyed through
`metadata::name_match::normalize_name`. But every `artist` row already stores a
`canonical_name`, computed by `scanner::canonical_name` (`src/catalog.rs`), and
the two functions disagree: `canonical_name` lowercases and collapses
punctuation, `normalize_name` also folds diacritics and turns `&` into `and`.
"Björk" is `björk` in the column and `bjork` through `normalize_name`.

The key should be the column — the read path then joins
`metadata_artist.name_canonical = artist.canonical_name` with nothing
recomputed, which is also what the desktop's own similar-artists cache does.
`normalize_name` keeps the job it was written for: matching a provider's ranked
candidates through `select_by_name`.

**5. No route can serve an image to a bare URL.** Decision 10 has
`smallImageUrl`, `mediumImageUrl` and `largeImageUrl` point at "this server's
own artwork route". There are two, and neither works from a URL a client loads
as-is: `media::artwork` wants the native bearer, and `getCoverArt` wants
Subsonic credentials in the query string — which would put a member's token
into a URL that clients cache and log. The established way out exists:
`src/stream_ticket.rs` seals `(user, subject, expiry)` under a one-byte
`TicketKind`, with `AUDIO = 0x01` and `CANVAS = 0x02`. An `ARTWORK` kind is the
same move. Whether to make it is decision A in section 4.

**6. The web client does not read RFC-007's stream.** Its only event-stream
consumer is scan progress (`/api/v2/scans/{id}/events`). Decision 6's "the
client is told when it lands" holds for the desktop, which mirrors through the
library feed; the web client must re-ask while the state is `pending`, a few
times and then stop.

### And four things that turn out cheaper than the RFC implies

- **The artist event needs no migration.** `library_event.entity_type` is
  `CHECK`-constrained to `('track', 'album', 'artist')` since
  `20260825000000_library_event.sql`; the vocabulary was admitted on the first
  day and only the emission is missing.
- **A rescan never erases a portrait.** The artist upsert (`src/catalog.rs`)
  updates `name`, `canonical_name` and `updated_at` on conflict and leaves
  `artwork_hash` alone, and `forget_unreferenced_artwork` already counts
  `artist.artwork_hash` as a reference.
- **Native clients get portraits for free.** `ArtistItem` already carries
  `artwork_hash`, and `media::artwork` already serves it behind the bearer with
  a per-request tenancy check.
- **Last.fm cannot supply images at all.** Its artist pictures are a placeholder
  star, which `is_lastfm_star_placeholder` exists to recognise. Portraits need a
  second provider — TheAudioDB or Deezer — which is decision B.

## 2. The order

Each server pull request below touches `src/` or `tests/`, so CodeQL fires and
none of them ends `BLOCKED`. The documentation each one makes true travels with
it rather than in a separate documentation-only branch: the RFC's erratum with
C, RFC-002's amendment with D, RFC-007's with F.

| | issue | repo | title | content | size | needs |
|---|---|---|---|---|---|---|
| A | **#263** | WaveFlow | The core's metadata clients accept a client, a base URL and a key | `with_http(client, base_url)` on the Last.fm, TheAudioDB and Deezer clients; TheAudioDB's key as a parameter; a bounded body read | `m` | — |
| B | **#264** | server | Move the `waveflow-core` pin | the bump alone, across 946 commits, no feature | `m` | A |
| C | **#265** | server | The cache and the engine that fills it | settings, two migrations, providers over the core, rate limit and single-flight, the three outcomes; the RFC's erratum; `operations.md` | `l` | B |
| D | **#266** | server | Biographies and similar artists | `getArtistInfo`/`2`, resolution through `library_member`, the native `enrichment` object and its four states; RFC-002 amended | `l` | C |
| E | **#267** | server | Similar songs | `getSimilarSongs`/`2`, the three kinds of id, random draw from the artist and their visible neighbours | `m` | D |
| F | **#268** | server | Portraits | the narrow fetcher, the artwork row and `artist.artwork_hash` in one transaction, the artist event; RFC-007 amended | `l` | C |
| G | **#269** | server | The artist page shows it | biography, similar artists, portrait, provenance, the `pending` re-ask | `m` | D (F for the portrait) |
| H | on #269 | — | The compatibility matrix | a campaign with real clients, then the matrix rows | — | D, E, F |

D and F both depend only on C and can run in either order. H is not a pull
request: it is a session in front of DSub, Symfonium, Feishin and Juliet, and
it gates the next tag, exactly as RFC-002's accepted deviations already do.

## 3. Each step

### A — in the core, first

Three constructors, each taking a `reqwest::Client` and a base URL, beside the
existing `new()` so the desktop changes nothing. TheAudioDB's key becomes a
field rather than a constant; `new()` keeps `"123"` for the desktop, which is
its business. The body is read through a ceiling before it is parsed — a
`bytes()` with a length check is enough, since the providers answer a few
kilobytes.

Optional and worth asking for at the same time, since the pin will move only
once: an `artist.getInfo` by **MusicBrainz id**. `ArtistItem` already carries
`musicbrainz_id` from the tags, and an MBID has no homonyms, where a name does.

### B — the pin, alone

Nothing else in the branch, so that a scanner regression has one suspect. The
full suite, and especially `scanner`, `identity` and `migrations`, which are the
three that would show a changed canonical form, a changed derived id or a
changed tag read. `cargo tree -d` to check that no crate the core shares with
this server — `lofty` first — appears twice.

### C — the cache and the engine

- **Settings** in `src/config.rs`, as decision 3 lists them, parsed the way
  `destinations` already are.
- **Two migrations**: `metadata_artist` and `metadata_artist_similar`, keyed
  `(provider, name_canonical)` where `name_canonical` is the column's form
  (section 1, item 4). `metadata_album` waits for the RFC's open question on
  album notes.
- **The outbound client moves** out of `scrobblers/` into a module both use;
  it is no longer scrobbling's.
- **Per provider: a token bucket and a single-flight map**, keyed on the
  canonical name. A fill is a background task; the writer gate is taken for the
  row write only, never across the request.
- **The erratum**: RFC-011 gains a dated *Revised* line and corrections to
  decisions 4, 6 and 10, each saying what the first version asserted — the
  convention RFC-010 set.

Tests, against a fake provider on a local listener, each with the inversion
that must make it fail:

| test | inversion |
|---|---|
| an empty provider list makes no request at all | call the provider anyway — the fake's counter moves |
| a provider that answers nothing is cached as a miss with the miss TTL | use the hit TTL |
| a provider that cannot be reached writes no row | write a miss on a transport error |
| ten concurrent first visits make one request | remove the single-flight map |
| a `302` from the provider is not followed | build the client with the default policy |
| a body over the ceiling is refused | lift the ceiling |
| the key never reaches the log | log the request URL |

### D — biographies and similar artists

`artist_info` in `src/subsonic/browse.rs` reads the cache after its tenancy
lookup, schedules a fill when there is nothing fresh, and answers. Similar
artists resolve through `library_member` with the caller as the first bind.
`ArtistDetail` gains the `enrichment` object with its state and per-field
source.

| test | inversion |
|---|---|
| a similar artist living only in another member's library is not returned | resolve without the `library_member` join |
| a foreign artist id answers as a missing one | answer the enrichment before the lookup |
| a Last.fm miss does not hide the second provider's biography | stop at the first provider's row |
| `pending`, `ready`, `none`, `disabled` each appear when they should | collapse `none` into absence |

### E — similar songs

One implementation, two entry points. The id is resolved first and for its
refusal; an album resolves to its album artist, a track to its track artist.
The draw is random over the artist's own tracks and their visible neighbours',
`count` defaulting to 50.

| test | inversion |
|---|---|
| an album id and a song id reach the same artist as the artist id | accept artist ids only |
| the artist's own tracks are in the draw | draw from the neighbours alone |
| no track from a library the caller is not in | drop the tenancy bind |

### F — portraits

The fetcher of decision 8: allowlisted hosts per provider, https, no redirect,
private and loopback ranges refused, the resolved address pinned for the
connection (`ClientBuilder::resolve` per fetch), a type and size ceiling. The
artwork row and every `artist.artwork_hash` that names the same canonical name
are written in **one transaction** under the writer gate, so the sweeper never
sees the row unreferenced. The cache row keeps the hash, so an artist
re-identified by a PID spec change is relinked without a second download — after
checking the artwork row still exists, since the sweeper may have taken it.

The artist event is emitted only when `artwork_hash` actually moved, which is
RFC-007 decision 6's own rule for albums.

One test seam needs deciding in the branch rather than here: the fake image
host lives on loopback, which the fetcher must refuse. The address policy has to
be injectable, with one test asserting that the **default** policy refuses
loopback — otherwise the suite proves the fetcher works only with its main
protection turned off.

### G — the web client

`ArtistPage` in `webapp/src/pages/artist-page.tsx` and the `ArtistDetail` type in
`webapp/src/api/catalog.ts`. The query re-asks a bounded number of times while the state is
`pending`. Provenance is shown, because decision 11 says it must be. A
production build before any Playwright run, since a stale `dist` has already
made one of those prove nothing.

## 4. The decisions, and the step each one blocked

**All four were taken on 2026-10-14**, each as recommended below. What follows
keeps the reasoning, so a later reader can tell a choice from an accident.

**A. Do `*ImageUrl` exist?** Blocks F's Subsonic half only. Either an
`ARTWORK` ticket kind and a route serving it, or the three fields stay absent
and clients find the portrait through `coverArt`, which every artist element
already carries. **Taken: absent in F**, and campaign H says whether a client in the matrix
actually reads `largeImageUrl`. A ticket can be added then; a URL scheme
cannot be withdrawn once clients cache it.

**B. Which provider supplies images?** Blocks F. TheAudioDB wants a key, and
whether a personal one is free was not verified. Deezer's public API asks for
none. **Taken: Deezer**, subject to its terms, which were not verified either.

**C. Who opens A?** It is a pull request on `InstaZDLL/WaveFlow`. Everything
after it waits for it. **Taken: this project's agent, in the same session.**

**D. Issues.** The #206-#223 plan was tracked as one issue per step. **Taken:
#263 to #269**, A to G, with H recorded on #269.

## 5. Not in this plan

Album notes (`metadata_album`), the language of biographies, cache eviction —
the RFC's open questions, which stay open. Lookup by MusicBrainz id beyond what
A can offer. Wasmtime, which decision 2 settled.

## What was not verified

- Whether TheAudioDB issues personal API keys without a paid subscription, and
  what Deezer's API terms say about a server caching and re-serving its images.
- Which Subsonic clients read `largeImageUrl` rather than `coverArt` for an
  artist — the reason decision A is recommended as a measurement.
- What the 946 commits between the pin and the core's `main` change in scanning
  behaviour. B exists to find out under a full suite, not to assume it.
- Last.fm's rate limit: "roughly five requests a second" is the core's own doc
  comment, not a measurement.
