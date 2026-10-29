# RFC-014 — An automatic artist banner, from TheAudioDB's fanart

- **Status**: Proposed
- **Implemented by**: in progress, in the order of "The order of the work"
  below: [#312](https://github.com/InstaZDLL/waveflow-server/pull/312) (this
  RFC) and [#314](https://github.com/InstaZDLL/waveflow-server/pull/314)
  (steps 2 and 3 together: the `theaudiodb` provider and its key, the fill and
  its pace, the fanart columns, linking and unlinking, the effective banner
  and `enrichment.banner`, the sweep, and the credit in the web client). The
  *Status* field above never flips in this project; this line is the one that
  says what is running.
- **Date**: 2026-10-24
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-011](RFC-011-metadata-enrichment.md) (the providers, the
  cache, the narrow image fetcher),
  [RFC-013](RFC-013-chosen-artist-imagery.md) (the banner, chosen by hand)
- **Amends**: RFC-013 decision 3, which says "there is no automatic banner",
  and decision 5, which says removing a chosen banner leaves none. Both were
  true because no provider supplied one; this document adds the provider.

## Problem

RFC-013 gave an artist a banner, but only one chosen by hand. An artist nobody
chose for has none, and the web client draws its header over the portrait,
blurred. That is the honest fallback; it is also what nearly every artist in a
library will look like, since few owners will choose a banner for each one.

The desktop app (`InstaZDLL/WaveFlow`) fills the same header from
**TheAudioDB's fanart**: wide 16:9 photographs, around 1920×1080, that the
community uploads per artist. `waveflow-core` already reaches TheAudioDB
(`metadata::theaudiodb`), and its `artist_info` answer already carries the
fanart. RFC-013's last open question named exactly this, and named why it
needed a document of its own: a provider RFC-011 does not configure, its own
key, its own host allowlist, its own entry in `WAVEFLOW_METADATA_PROVIDERS`.

This is that document.

## Decision 1 — TheAudioDB is a third provider, off like the others

`WAVEFLOW_METADATA_PROVIDERS` accepts `theaudiodb` beside `lastfm` and
`deezer`. Nothing else switches it on: RFC-011 decision 3 holds unchanged, and
a server that is merely upgraded asks TheAudioDB nothing.

- **It needs the operator's key**, `WAVEFLOW_METADATA_THEAUDIODB_KEY`. Listed
  without one, the server refuses to start, as it does for `lastfm`. **No key
  ships in the binary.** `waveflow-core` uses TheAudioDB's public test key
  (`"123"`) for one desktop user; RFC-011 decision 3 already refused that for a
  server, whose quota would be shared with the whole internet, and the reason
  has not moved. A key placed with no provider naming it builds nothing: the
  list is the switch, and a key is not consent.
- **It is asked for fanart only.** TheAudioDB also ships biographies in about
  fifteen languages, but nothing in this server knows a listener's language
  and RFC-011's cache is keyed without one; that is RFC-011's open question,
  and taking TheAudioDB's English biography here would answer it by accident.
  Its biography is not stored. An operator who lists `theaudiodb` gets banners,
  and the documentation says so in the same line that names the provider.
- **`WAVEFLOW_METADATA_IMAGES=off` leaves it unbuilt**, as it does `deezer`:
  images are all it is asked for.

## Decision 2 — which picture, and only one

TheAudioDB's search answers up to four fanarts (`strArtistFanart` to
`strArtistFanart4`), a "wide thumb" and a "banner". The server takes **the
first real fanart**, in TheAudioDB's own order, and nothing else. "Real" means
non-empty and accepted by the fetcher below: the fanarts are tried in that
order, one refused for what it is — off the allowlist, not an image, too large
— gives way to the next, and the first one stored ends the search. Only when
none of the four is accepted is the answer a miss. A fanart whose host could
not be reached ends it differently: the lookup is unreached, as RFC-011
decision 5 requires, and nothing is written.

- **Not `strArtistBanner` nor `strArtistWideThumb`.** Both are 1000×185 strips
  by TheAudioDB's own specification. The desktop fed its header from them and
  measured the result (WaveFlow#693): a header wider than 1000 pixels and far
  taller than 185 relative to its width showed them upscaled and cropped to a
  sliver. Fanart is the picture the header is shaped for.
- **One, not four.** The desktop lets its user flip between the alternates.
  Offering them here is a picker, which RFC-013 decision 4 shows is a search,
  a candidate table and a pick route of its own; that is a later document if
  anyone wants it (see "What is still open"). An automatic banner is the one
  the server can choose without asking anybody.

The picture is fetched through RFC-011 decision 8's narrow fetcher, as a
Deezer portrait is: https only, no redirect, public addresses only, the
resolved address pinned, a recognised image type, **10 MiB** at most — a
1920×1080 JPEG is a fraction of that — and **a host allowlist of its own**:
the hosts TheAudioDB serves its images from (`r2.theaudiodb.com` and
`www.theaudiodb.com` when this was written; the list is a constant beside the
base URL and is checked against live answers when it is implemented). A fanart
URL naming any other host is dropped without being contacted.

## Decision 3 — cached like a portrait, linked like a portrait

RFC-011's cache holds one row per provider and canonical name. A TheAudioDB row
records its fanart in a new nullable column, **`metadata_artist.fanart_hash`**,
referencing `artwork(hash)` with `ON DELETE SET NULL`, as `image_hash` does for
a portrait. The outcomes are RFC-011 decision 5's:

- a **hit** stores the fanart and stands for the long TTL;
- a **miss** — TheAudioDB knows no such artist, or knows it without fanart, or
  names only pictures the fetcher refuses for what they are — stands for the
  short one;
- **unreached** writes nothing, and the next visit asks again.

**The artist row carries it**, in a new nullable column
**`artist.fanart_hash`**, written the way RFC-011 decision 8 writes
`artist.artwork_hash`: every artist row with the canonical name, in every
library, linked in the transaction that writes the cache row, only where it
changes, and each moved row recording the **artist** library event (RFC-007),
so a client that re-reads the artist on that event sees the banner arrive. A
fill that fetched the same picture again changes nothing and says nothing.

**A miss unlinks.** When a refresh comes back a miss — TheAudioDB has dropped
the artist or its fanart — the rows still naming the fanart the previous cache
row held have `artist.fanart_hash` set back to `NULL`, in the same transaction
and with the same artist event per row. Otherwise the header would go on
showing a picture the provider no longer offers, and the sweep would keep it
for as long as the stale link lasted. **Unreached unlinks nothing**: a timeout
or a `429` says nothing about the artist, writes no cache row, and leaves every
link where it was.

A column on the artist, rather than a join from the artist to the cache at
read time, for the same reasons as the portrait: the sweep keeps what an artist
names, cover-art resolution finds it through the artist's own membership, and
a scan that re-identifies the artist leaves the next fill to link it again
without a second download.

**It is never a choice.** `artist.fanart_hash` belongs to the provider and is
rewritten whenever TheAudioDB's answer changes; a hand-chosen banner stays in
`artist_image_choice` (RFC-013 decision 2), which nothing automatic writes.

## Decision 4 — what every reader shows

RFC-013 decision 3 becomes, for the banner:

```text
effective banner = chosen banner ?? artist.fanart_hash
```

- **The native API's artist detail** answers it as `banner_hash`, the field
  RFC-013 added: absent only when neither exists. `image_choice.banner` still
  says whether it was chosen by hand.
- **Its provenance** is published as RFC-011 decision 11 asks:
  `enrichment.banner` carries the hash and its `source` (provider and fetch
  date) when the banner shown is TheAudioDB's. TheAudioDB's terms ask for
  attribution, like Last.fm's; a banner that appears without saying where it
  came from is the licensing problem decision 11 exists to avoid.
- **`GET /api/v2/artwork/{id}`** serves it to the members of a library whose
  artist names it, under the same membership check as every other artwork.
- **The Subsonic façade gains nothing.** The protocol has no banner element,
  and RFC-013 decision 3 already declined to invent one under the freeze.

**Decision 5 of RFC-013 changes accordingly**: removing a chosen banner returns
the artist to the automatic one, which may be TheAudioDB's fanart or nothing.

## Decision 5 — TheAudioDB's own pace

RFC-011 spaces each provider's requests to `WAVEFLOW_METADATA_RATE_PER_SEC`,
four a second by default. TheAudioDB documents a far lower ceiling for its
keys — thirty requests a minute on the tier `waveflow-core`'s comment
describes. So TheAudioDB's pace is the **lower** of the operator's rate and a
ceiling held as a constant beside its base URL, one request every two seconds.
An operator cannot raise it by raising the shared setting, and two hundred
artist pages opened in a row fill their banners over a few minutes rather than
earning the key a refusal. RFC-011's ceiling on queued fills (`MAX_IN_FLIGHT`)
already bounds what a burst of visits can schedule.

A `429` from TheAudioDB is **unreached**, not a miss: it says nothing about the
artist and writes nothing, as RFC-011 decision 5 requires of any failure to
ask.

## Decision 6 — the sweep

The artwork sweep keeps a hash named by `artist.fanart_hash`, beside the four
it already keeps (`artist.artwork_hash`, `album.artwork_hash`,
`track.artwork_hash`, `artist_image_choice`) and RFC-013's candidates. A fanart
no artist names any more is collected like any other artwork; its cache row
loses its hash by `ON DELETE SET NULL`, and the next fill for that name, if
anyone asks, downloads it again.

## What this RFC changes elsewhere

- **RFC-011**: decision 3's settings gain `WAVEFLOW_METADATA_THEAUDIODB_KEY` and
  the provider `theaudiodb`; decision 8's allowlist gains TheAudioDB's hosts,
  which that decision already anticipated by name.
- **RFC-013**: decision 3's "there is no automatic banner" and decision 5's
  "for the banner, none" name this document as the one that changed them.
- **`docs/guides/operations.md`**: the provider table gains `theaudiodb`, what it is
  asked for (banners only), the key it needs, and its fixed pace.
- **The web client**: nothing changes in how the header is drawn — it already
  draws `banner_hash`. It gains the provenance line under an automatic banner,
  as it has one under a provider's portrait.
- **Migrations**: one new dated file adding `metadata_artist.fanart_hash` and
  `artist.fanart_hash`. None edits an applied one.

## The order of the work

Each step is its own pull request, shippable without the next:

1. **This RFC.**
2. **The provider and the banner, served**: `theaudiodb` in the provider list
   and its key, the fill and its pace, the two columns and their migration,
   the linking and its event, the effective banner and `enrichment.banner` on
   the artist detail, cover-art resolution, the sweep, and
   `docs/guides/operations.md`.
3. **The web client**: the provenance line under an automatic banner.

## What is still open

- **Choosing among the fanarts.** TheAudioDB offers up to four, and the
  desktop lets its user flip between them. Doing that here is a picker with a
  search and a pick checked against it, as RFC-013 decision 4 built for Deezer
  portraits; it is not taken until someone asks for it.
- **Hiding an automatic banner.** Removing a chosen banner now falls back to
  TheAudioDB's, not to none. An owner who wants no banner at all over a fanart
  they dislike has no way to say so; a choice of "none" in
  `artist_image_choice` is the obvious shape, and is left until it is missed.
- **TheAudioDB's biographies.** Still RFC-011's open question on language.
- **The key tiers.** The ceiling of decision 5 is the documented free-tier
  figure. If a paid key allows more, a setting can raise TheAudioDB's ceiling
  for an operator who holds one; it is not added before someone does.
