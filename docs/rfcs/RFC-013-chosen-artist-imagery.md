# RFC-013 — Chosen artist imagery: a portrait and a banner, set by hand

- **Status**: Proposed
- **Implemented by**: in progress, in the order of "The order of the work"
  below: [#308](https://github.com/InstaZDLL/waveflow-server/pull/308) (this
  RFC), [#309](https://github.com/InstaZDLL/waveflow-server/pull/309)
  (serving the choice: `artist_image_choice`, the effective portrait,
  `banner_hash`, the upload and removal routes) and the pull request drawing
  it in the web client (the banner behind the artist header, and "Change
  portrait" / "Change banner" for owners and managers). The
  *Status* field above never flips in this project; this line is the one that
  says what is running.
- **Date**: 2026-10-23
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md),
  [RFC-011](RFC-011-metadata-enrichment.md)
- **Amends**: RFC-011 decision 11, which refuses any route that writes artist
  imagery into the server. This document opens exactly one such route, for a
  library's owners and managers, and states why it is not the ingestion surface
  decision 11 refused.

## Problem

An artist page on the web client shows a portrait, or a grey record when there
is none, and a plain header. The portrait comes from the files' artwork or,
since RFC-011 decision 8, from Deezer. Both are automatic, and both are
sometimes wrong: a provider matches the wrong "Ashreveal", a band's best
picture is not the one Deezer ranks first, a collaboration credited as
"Agust D, Ryuichi Sakamoto, WOOSUNG" has no picture anywhere.

The desktop app already answers this, on the local machine only:

- a **portrait** picker, from a Deezer search or a local file, with a way back
  to the automatic one;
- a **banner** — the wide image behind the artist page's header — picked from
  fanart suggestions or a local file, also with a way back.

Neither reaches the server, and the server has neither concept: no route sets
an artist's image, and nothing is called a banner. The owner of a library asks
for the same two controls on the server's web client. This document decides
what they write, where, and what every reader then shows.

## Decision 1 — a chosen image belongs to the library, not to one listener

A portrait or banner set by hand is **seen by every member of the library** and
set by those who may correct its metadata: `LibraryRole::may_write_metadata`,
owner and manager. A listener sees it and cannot change it, and is refused with
the same `404` a foreign library would give (CLAUDE.md, "404 blurs
everything").

That is the same rule as a tag correction and as filing (RFC-012 decision 3),
for the same reason: it changes how the library is presented to everyone in it,
and nothing in a file. A per-listener picture would be a private skin over a
shared catalogue, which nobody asked for.

It is **not** the shared enrichment cache. RFC-011 decision 4 keeps
`metadata_artist` instance-wide and free of account data, and says anything
account-specific "belongs in a different table". A choice is library-specific
rather than account-specific, but the conclusion is the same: its own table,
keyed by the library's artist, never a row in the cache.

## Decision 2 — its own table, never `artist.artwork_hash`

```sql
CREATE TABLE artist_image_choice (
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('portrait', 'banner')),
    artwork_hash TEXT NOT NULL REFERENCES artwork(hash),
    source TEXT NOT NULL CHECK (source IN ('upload', 'deezer')),
    set_by TEXT NOT NULL,
    set_at INTEGER NOT NULL,
    PRIMARY KEY (artist_id, kind)
) STRICT;
```

**Not the artist row's own `artwork_hash`.** RFC-011 decision 8 writes that
column, and `link_portrait_in` rewrites it for every library on every fill and
on every repair at read time — whenever it differs. A chosen portrait stored
there would be replaced by the provider's the next time anyone opened the
page. Two columns that two writers fight over is the bug; two places with one
writer each is the design.

**Keyed by the artist row**, which is per library: the same performer in two
libraries can carry two different choices, one per owner. Artist ids are
derived (CLAUDE.md, "Identifiers and time"), so a choice survives every
ordinary rescan. It does **not** survive a change of `WAVEFLOW_PID_ARTIST`,
which re-identifies every artist: the choice goes with the old id, as album
favourites already do. That is stated here rather than discovered.

**The artwork store keeps what a choice names.** The sweep in
`src/services/artwork.rs` deletes an `artwork` row no artist, album or track
references. It learns a fourth reference — `artist_image_choice` — or it would
delete a chosen image on its next pass, and a fifth, the candidates of a live
Deezer search (decision 4). This is the line most likely to be forgotten, so it is in
this document.

## Decision 3 — what every reader shows

**The portrait** a reader sees is the chosen one if there is one, otherwise
what the server already shows:

```text
effective portrait = chosen portrait ?? artist.artwork_hash
```

One SQL expression, `COALESCE(choice.artwork_hash, a.artwork_hash)`, used
everywhere an artist's artwork is read today:

- the native API's artist items and the artist page (`artwork_hash`, and the
  provenance the page prints under a provider's portrait, which must not claim
  a chosen image came from Deezer);
- cover-art resolution in `src/services/catalog.rs`, so
  `GET /api/v2/artwork/{id}` serves it under the same membership check;
- the Subsonic façade's `coverArt` on an artist (`src/subsonic/nodes.rs`).

**The Subsonic contract is unchanged in shape.** `coverArt` already exists on
every artist and already names whatever image the server holds; a chosen
portrait changes which image it names, not what the element looks like. That
is data, like a tag correction changing a title, and the frozen v2.0-beta
contract (RFC-002) is a contract about shape. No Subsonic element is added for
the banner: the protocol has none, and inventing one is exactly what the
freeze forbids.

**The banner** exists only on the native API, as `banner_hash` on the artist
detail — absent when nothing is chosen. There is no automatic banner in this
document: RFC-011 configures no fanart provider, and inventing one here would
be a second enrichment RFC hidden inside this one. A client with no banner
draws what it draws today; the web client may blur the portrait behind the
header, as the desktop does, and that is a client's choice, not a field.

## Decision 4 — how an image arrives

Two sources, both ending in the same content-addressed `artwork` store
(`data/artwork/<hash>.<format>`, RFC-011 decision 8 and RFC-009) that portraits
already use.

**A file the owner sends.** The body is the image's bytes, as a lyrics upload
is (RFC-012 decision 6): no multipart, no base64.

- JPEG, PNG or WebP, recognised **from the bytes**, never from a name or a
  `Content-Type`;
- at most **10 MiB**, the cap `src/portrait.rs` already applies to a fetched
  portrait, enforced by the route's own body limit so a larger body answers
  `413` before it is read;
- stored once per content: the hash names the file, so the same image chosen
  for two artists is one file.

Nothing is resized or re-encoded. A client that wants a thumbnail asks for the
artwork as it does for any other.

**A Deezer artist the owner picks** (portrait only). This is the desktop's
"Search Deezer" tab, and it must not become what RFC-011 decision 8 refused: a
browser that loads an image from a third party tells that party who is looking.
So the server does the looking:

- `GET /artists/{id}/image/candidates?q=` asks Deezer's artist search, through
  the client RFC-011 already uses, and answers up to 20 candidates — a Deezer
  id, a name, a fan count — **without picture URLs**;
- each candidate's picture is fetched by the server through `src/portrait.rs`'s
  narrow fetcher (https, host allowlist, public addresses only, pinned
  resolution, 10 MiB) into the artwork store, at Deezer's small picture size,
  and the candidate carries its `artwork_hash`. The client shows previews from
  the server, never from Deezer;
- choosing one writes the choice with `source = 'deezer'` from a candidate the
  server fetched. The client sends a Deezer id; it never sends a URL, so no
  user input ever names a target the server dereferences — #217's line holds.

### Who may search, and how much

A search costs the server a request to Deezer and up to twenty image fetches
and writes, so it is not open to whoever may read the page:

- **Owners and managers only**, the same `may_write_metadata` gate as the
  choice it serves; anyone else gets the `404` of decision 1. A listener has no
  picker to open, so this refuses nothing a client offers.
- **Deezer's pace is RFC-011's.** Every request to Deezer — the search and each
  preview — keeps the per-provider pace RFC-011 decision 3 sets
  (`WAVEFLOW_METADATA_RATE_PER_SEC`) and `src/enrichment.rs` enforces, shared
  with enrichment. A picker cannot make the server ask Deezer faster than
  the operator allowed, however often it is opened.
- **One search at a time per account.** A second search while the first is
  still fetching answers `429`; the client waits for the first.
- **A repeated search is answered from what the server holds.** The same
  query for the same artist within a search's life (below) returns the search
  already stored and fetches nothing.

### What the server remembers between the offer and the pick

The pick names a Deezer id and nothing else, so the server has to remember
what it offered, or it could neither refuse an id it never offered nor find
the preview to store. It remembers a **search** and its **candidates**, in two
tables, so that a repeated search can be answered whole and two searches may
offer the same Deezer artist:

```sql
CREATE TABLE artist_image_search (
    id TEXT PRIMARY KEY,
    artist_id TEXT NOT NULL REFERENCES artist(id) ON DELETE CASCADE,
    -- The query as typed, trimmed and case-folded: "Björk " and "björk" are
    -- one search.
    query_key TEXT NOT NULL,
    searched_by TEXT NOT NULL,
    searched_at INTEGER NOT NULL,
    -- An hour from the moment the search completed; NULL while it fetches.
    expires_at INTEGER,
    -- Set when every candidate's preview is stored. 0 means a task is
    -- fetching for it now: a failed search is deleted, a search past its
    -- fetch deadline is swept, and boot deletes any left at 0. It is what
    -- makes a second search by the account answer 429.
    complete INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1))
) STRICT;

CREATE TABLE artist_image_candidate (
    search_id TEXT NOT NULL REFERENCES artist_image_search(id) ON DELETE CASCADE,
    -- Deezer's order, so the answer reads as the search did.
    position INTEGER NOT NULL,
    deezer_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    fans INTEGER,
    artwork_hash TEXT NOT NULL REFERENCES artwork(hash),
    PRIMARY KEY (search_id, position),
    UNIQUE (search_id, deezer_id)
) STRICT;
```

- **A search is the artist's, so it is the library's**: the artist row is per
  library, and any owner or manager of that library may pick from it, not only
  the person who searched.
- **It lives one hour from the moment it completes.** Long enough to look
  through twenty faces and choose; short enough that the tables hold what is
  on someone's screen and not a history of searches. The hour starts when the
  last preview is stored, so however long the fetching took, it never eats
  into the time to choose.
- **A repeated search is answered from it.** The same `query_key` for the same
  artist, while a complete search for it is alive, returns that search's
  candidates in their order — id, name, fans, preview — and asks Deezer
  nothing. A different query makes a search of its own, even where the two
  return the same artist.
- **The pick checks it.** `PUT …/image/portrait/deezer` accepts a Deezer id
  only if a live search of that artist offered it, and stores that candidate's
  `artwork_hash` as the choice — the image the owner saw, fetched once, not
  fetched again. When several live searches offered it, the most recent one's
  candidate is used; their previews are the same picture at the same size, so
  this decides nothing visible. An id no live search offered answers `422`,
  and so does one offered for another artist: a search belongs to the artist,
  so a search made in one library is never found from another.
- **A search that does not finish is not left half-done.** `complete = 0`
  means one thing only: a task is fetching for it right now. So:
  - **the fetching is not the request's.** The route records the search and
    hands the fetching to a task of its own, then waits for it. A client that
    closes the tab or loses the network drops the request — and axum drops
    the handler with it, mid-await, running none of its remaining code — but
    not the task, which goes on to complete the search or delete it. The next
    picker that asks the same query finds it complete;
  - a search whose Deezer query or any preview fetch fails is **deleted** by
    that task. Its candidates go by cascade, and the previews already stored
    become unreferenced artwork for the sweep;
  - **a deadline the task itself obeys.** Every search gets a fetch deadline,
    `searched_at` plus five minutes, and its task runs under that deadline as
    a whole — waiting for the provider's pace, which it shares with enrichment
    and cannot predict, the requests, and the writes alike
    (`tokio::time::timeout_at`). A task that reaches the deadline stops and
    deletes its search as a failed one. Its last write, completing the search,
    happens under the writer gate and checks the deadline inside that
    transaction: past it, nothing is completed, and the search is deleted
    instead. So no task ever runs, or writes, beyond its deadline;
  - **past the deadline, the search is fair game.** An incomplete search past
    its deadline no longer counts for the `429`, and the sweep — which takes
    the same writer gate — deletes it. Since its task has stopped by then, this
    removes only what a panic or a bug left behind, never a search still
    fetching;
  - at **boot**, every search with `complete = 0` is deleted: nothing can be
    fetching for it across a restart;
  - the `429` of one search at a time therefore only ever reflects a search
    that is really in flight, and the next search after a failure or an
    abandoned page starts at once.

### The pick and the sweep never interleave

The sweep deletes expired searches and then any `artwork` row nothing names;
the pick reads a live candidate and then writes a choice that names its
preview. Run side by side, the sweep could delete a preview between the pick's
read and its write. They cannot, for the reason every write here is safe: both
take the process-wide **writer gate** (CLAUDE.md, "SQLite discipline") and run
in **one transaction** each.

- The pick, under the gate and inside its transaction, reads the candidate with
  `expires_at` compared to the transaction's own clock reading, and inserts the
  choice. The sweep cannot run in between, and it deletes only searches whose
  `expires_at` has already passed — so a search live when the pick read it was
  not swept while the pick ran.
- The sweep, under the gate, deletes expired searches (their candidates go by
  cascade) and then unreferenced artwork, in that order and in one transaction,
  so a preview a choice already names is never taken by the second step.
- **The sweep never touches a search that is still fetching.** An incomplete
  search has no `expires_at` to pass, so the sweep cannot delete it from under
  the task writing its candidates. Its ways out are the ones named above: its
  task completes it or deletes it, the sweep deletes it once past its fetch
  deadline — which no running task can be — or boot deletes it. The previews
  an incomplete search has stored so far are named by its candidates, so the
  sweep keeps them too until one of those happens.
- If anything ever slipped through, the foreign key from
  `artist_image_choice.artwork_hash` to `artwork` fails the insert rather than
  storing a dangling hash; the route answers `422`, as for an expired search,
  and the owner searches again.

Candidates are offered only where `deezer` is among
`WAVEFLOW_METADATA_PROVIDERS`. An instance that never opted into Deezer makes
no request to it because someone opened a picker: the operator's switch is the
only switch.

**The switch is read at every call, not at the search.** An operator who
removes `deezer` and restarts closes both routes at once: the candidates route
and the pick answer `404`, even for a search that is still live. A search
made under the old setting does not outlive the operator's decision by up to
an hour; its rows simply expire and are swept like any other. A portrait
already chosen from Deezer stays: it is the library's image now, stored on the
server, and no request to Deezer is needed to show it.

## Decision 5 — a way back

Removing a choice returns the artist to the automatic image: for the portrait,
whatever `artist.artwork_hash` holds; for the banner, none. The image itself
is not deleted by the removal; the sweep collects it when nothing names it any
more.

## Decision 6 — clients learn of it as they learn of a portrait

Setting or removing a choice records the **artist** library event
(RFC-007) for that artist, with no payload — exactly the event RFC-011
decision 8 records when a fetched portrait arrives. A client that re-reads the
artist on that event sees the change; nothing new is needed on the wire.

## Why this is not the ingestion route RFC-011 refused

RFC-011 decision 11 says there is "no ingestion route by which a desktop
client or a companion process could push enrichment into the server": an
authenticated write path for machine-produced metadata, with provenance and
tenancy questions and no client to use it.

This is narrower on every count. It writes **one image per artist per kind**,
chosen by a **person** in the library's metadata role, recorded with who chose
it and when, scoped to **one library**, and never into the shared cache. Bios,
similar artists and everything else decision 11 covered stay unwritable.

## The surface

All under `/api/v2`, tenancy-checked in the queries; `404` for an artist the
caller may not see, and for a write by someone who may not make it.

| Route | What |
| --- | --- |
| `GET /artists/{id}` | Gains `banner_hash` (absent when none), and `image_choice` per kind (`source`, `set_at`) so a client can say "chosen by hand" instead of "from Deezer". `artwork_hash` becomes the effective portrait. |
| `PUT /artists/{id}/image/{kind}` | `kind` is `portrait` or `banner`. The body is the image's bytes. `200` with the artist; `413` past 10 MiB; `422` for bytes that are not JPEG, PNG or WebP. |
| `PUT /artists/{id}/image/portrait/deezer` | `{ "deezer_id": … }`, offered by a live search of this artist; that candidate's preview becomes the portrait, never fetched again. `422` for an id no live search offered; `404` where Deezer is not configured now, a live search notwithstanding. |
| `GET /artists/{id}/image/candidates?q=` | Owners and managers only. Up to 20 Deezer artists, each with a server-held preview `artwork_hash`, stored as a search for an hour; the same query answered from it while it lives. `429` while the same account's previous search is still fetching; `404` where Deezer is not configured. |
| `DELETE /artists/{id}/image/{kind}` | Back to automatic. `204`; `404` when nothing was chosen. |

Every write goes through `DomainServices` (CLAUDE.md, "One set of domain
services"), so the desktop can reach the same rules when it chooses to.

## What this RFC changes elsewhere

- **RFC-011**: decision 11 gains a sentence naming this document as the one
  write path it does not cover.
- **The artwork sweep** keeps hashes named by `artist_image_choice`.
- **The Subsonic façade**: `coverArt` on an artist names the effective portrait.
  No element is added or removed.
- **The desktop app** (`InstaZDLL/WaveFlow`): nothing in this repository. Its
  local pickers keep writing its local profile. Pushing them to the server, or
  showing a server artist's chosen banner, is the desktop's own change, made
  against the routes above.
- **Migrations**: one new dated file with `artist_image_choice`, and one with
  `artist_image_search` and `artist_image_candidate` in step 4. None edits an
  applied one.

## The order of the work

Each step is its own pull request, shippable without the next:

1. **This RFC.**
2. **The choice, served**: the table and its migration, the sweep's fourth
   reference, the effective portrait in every reader, `banner_hash` and
   `image_choice` on the artist, the upload and removal routes, and the artist
   event.
3. **The web client**: a banner behind the artist page's header (the chosen
   one, or the portrait blurred), and, for owners and managers, "Change
   portrait" and "Change banner" with a file and "Use the automatic one".
4. **Deezer candidates**: the search and candidate tables and their place in
   the sweep, the candidates route with server-held previews and its limits,
   the pick route checked against a live search, and the "Search Deezer" tab
   of the portrait picker.

## What is still open

- **An automatic banner.** TheAudioDB's fanart, which the desktop suggests,
  would need a provider RFC-011 does not configure — its own key, its own host
  allowlist, its own entry in `WAVEFLOW_METADATA_PROVIDERS`.
- **Cropping.** Neither picker crops; the client frames the banner at a fixed
  position, as the desktop does. A focal point stored beside the choice is a
  later, additive change.
- **Choices for an artist present in several libraries.** Each library's owner
  chooses for their own; nothing copies a choice across. Whether an instance
  operator should be able to set one everywhere is not decided here.
