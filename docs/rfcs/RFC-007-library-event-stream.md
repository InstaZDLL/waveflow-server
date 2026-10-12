# RFC-007 — The library event stream

- **Status**: Proposed
- **Implemented by**: [#145](https://github.com/InstaZDLL/waveflow-server/pull/145)
  (the stream and its watermark), [#152](https://github.com/InstaZDLL/waveflow-server/pull/152)
  (the originating device), [#159](https://github.com/InstaZDLL/waveflow-server/pull/159)
  (album events, decision 6),
  [#166](https://github.com/InstaZDLL/waveflow-server/pull/166) (retention,
  decision 7),
  [#167](https://github.com/InstaZDLL/waveflow-server/pull/167) (the
  acknowledgement, decision 8). The *Status* field above never flips in this
  project: this line is the one that says what is running, and it can be
  checked — a pull request can be read, a status word cannot be audited.
- **Date**: 2026-08-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md)
- **Prerequisite of**: writing a track's metadata, receiving a file (RFC-008),
  and the per-track video asset

## Problem

WaveFlow Server holds two kinds of state, and only one of them can tell its own
story.

**User state** — favourites, ratings, plays, queue, playlists, shares,
bookmarks — converges through the synchronization journal described in RFC-003.
Seven entity types, `CHECK`-constrained on `sync_event` because the vocabulary
is a contract and not free text. A client follows its cursor and knows what it
missed.

**Library state** — tracks, albums, artists, artwork — has no stream at all.
The observation is checkable rather than argued: `scanner.rs` and `catalog.rs`
contain **no call to the journal**. The one neighbouring stream,
`/api/v2/scans/{id}/events`, carries the *progress* of a scan job — a snapshot,
some ticks, then it dies with the job. It says a scan is advancing; it never
says a track has changed.

The consequence for a client: the only way to learn that a catalogue has moved
is to re-probe it. A client comparing counters catches a track added or removed
and misses everything else — a retagged title, a corrected year, a replaced
cover. That is a workaround, and it is blind by construction.

Three pending requests all run into this gap: writing a track's metadata,
receiving a file, and attaching a video asset. None of them has anywhere to
announce what it did.

## Decision 1 — two streams, never one

Library state gets its own table and its own sequence. It does not join
`sync_event`.

The reason is mechanical rather than aesthetic: `sync_event.cursor` is an
`INTEGER PRIMARY KEY AUTOINCREMENT`, that is **one unique, global sequence**,
filtered per user on read. A library stream sharing it would, on every rescan,
advance the very sequence every favourites cursor measures itself against. A
scan of fifty thousand tracks would move the synchronization cursor of every
account on the server.

The consequence on the client is accepted: **two cursors**, one per account for
user state, one per library for library state. That is the point. They do not
advance at the same rate or for the same reasons, and merging them would make it
impossible to advance one without the other.

### The asymmetry is the design

The two streams are not two symmetric halves:

| | User state | Library state |
| --- | --- | --- |
| Volume | small | large |
| Scope | one account | one library, shared |
| Origin | the client writes | the server writes, almost always |
| Machinery | operation ids, replay, conflict | none of it |

Almost always: a metadata `PATCH` and an upload are library changes **coming
from the client**, and their offline queue will replay them. So the stream needs
idempotence for a minority of its traffic and none for the majority. That is
exactly what `MutationContext::server_generated()` already models on the user
side today: a mutation context with no client origin. The mechanism transposes
without being reinvented.

## Decision 2 — membership is evaluated on read

Reading the stream joins `library_member` **at request time**, like every
projection in this server.

That settles revocation without building anything. A member who loses access
stops receiving that library's events on the next request, and retroactively:
there is no revocation list to keep, because there was never a subscriber list.
Current membership *is* the filter.

What a client has already downloaded stays on its disk, and nothing can take it
back. But it can no longer resolve anything from it: a 404 blurs a missing
resource and a foreign one, which RFC-002 lays down as a rule. The leak is
bounded by what the client already had, which is a property of any cached
catalogue.

## Decision 3 — one stream per library, with typed events

One stream per library, whose events carry a kind. Not two thematic streams.

The temptation is to separate what comes from the scan from what would come from
enrichment, on the grounds that one is wanted immediately and the other at
leisure. But two streams means two cursors *per library* — exactly the coupling
decision 1 set out to avoid, reintroduced one level down. A typed kind is
enough: a cursor is a position, not a queue, so ignoring a kind costs nothing
and holds nothing back.

The vocabulary is `CHECK`-constrained, like the user journal's, and for the same
reason: it is part of the contract.

- `entity_type`: `track`, `album`, `artist`
- `action`: `upsert`, `delete`

### The originating device, settled

An event carries the device that asked for it, when a client asked.

The table was first created without it, on the grounds written into its
migration that no row came from a client. Metadata writing made that reason
false and receiving a file ([RFC-008](RFC-008-receiving-a-file.md)) moved it
further away: without that column, a client reads its own upload back off the
stream as a track it has just discovered, and treats it as one. The user journal
has carried `origin_device_id` since its first day, for exactly this.

`NULL` keeps its plain meaning: nobody asked for it. A scan writes `NULL`, and
so does a client that names no device — the header is optional, and whoever
prefers to receive their own changes simply leaves it out.

The device's membership is verified before it is written. Without that check an
account could attribute its writes to somebody else's device — and any client
filtering its own changes out of the stream would then discard someone else's.

**A track `upsert`'s payload carries its `full_hash`.** That is the one thing
nothing else supplies, and it deserves explaining.

### An external edit moves the fingerprint, silently

The metadata writing described elsewhere rewrites no file: the correction lives
beside the track, so `full_hash` does not move and a reconciliation link a client
established stays valid. That is not where the problem is.

It is where the operator retags with an external tool. The scanner's skip
condition requires `existing.full_hash == input.full_hash`; different bytes make
it fail, the apply runs, and **the track keeps its identifier while its
fingerprint changes**. No client is told today.

Carrying the `full_hash` in the payload is enough: the client holds the old one,
compares, and knows its link needs rechecking. A dedicated event kind would be
more vocabulary for the same information.

## Decision 4 — retention is a first-day question

The user journal has no compaction today, and admits it: the comment in
`src/sync.rs` notes that it "will trim the head of the journal for everyone"
when it arrives. That deferral is tenable because user mutations are rare.

The library stream does not have that luxury. A first scan of fifty thousand
tracks produces fifty thousand events before any client has read anything.
Retention is therefore not a question for later here.

The precedent to follow already exists on the user side: when a cursor precedes
the oldest event kept, the server does not improvise — it answers that the
cursor has expired and that the client must start from a snapshot. For library
state that snapshot is not a route to write: it is the catalogue itself, which
`/api/v2/libraries/{id}/tracks` and the browse routes already serve.

The exact values are set by decision 7.

## Decision 5 — what the stream does not carry

**No enrichment.** The server fetches no outside metadata, by decision: it
stores and serves what it is given. The word appears in the architecture
diagrams going around; it corresponds to nothing in the code, and leaving it in
the vocabulary would promise a capability nobody has decided on. If it is ever
decided, it will arrive as one more kind, which decision 3 makes possible
without breaking anything.

**No event for a content-addressed resource.** A cover and a canvas are
immutable by hash: the bytes behind a fingerprint never change, so there is
nothing to notify. Only a track's *link* to a resource is a change, and that
travels in the track's `upsert`.

**Never user state.** A favourite, a rating, a play stay in RFC-003's journal.
The library stream does not duplicate them, and a client receiving them twice
would count them twice.

## What this RFC changes elsewhere

It **removes** the riskiest line from metadata writing. Without a stream of its
own, announcing a changed track would have required admitting a `track` type
into `sync_event` — hence rebuilding that table around its `CHECK` constraint,
SQLite being unable to alter one. That is the move with the implicit
`DELETE FROM` that has already cost this project a fix. A separate table does
not touch it.

It **sets aside** a decision taken too quickly: broadcasting one event per
library member into the user journal. The arithmetic condemns it — a library
with forty members, retagged over two thousand tracks, would write eighty
thousand rows for two thousand facts, in a table with no compaction — and so
does the semantics: the event is not about the user.

## Decision 6 — an album has its event, an artist does not need one

Opened by the first version of this RFC, settled on 2026-08-30 on a case the
desktop named.

**An album emits one.** The desktop's incremental mirror skips an album whose
`song_count` has not moved — that is the whole economy of its walk. An album
that is *merely retagged* is therefore invisible to it: a release date learned, a
cover found, an album artist corrected change no count. Without an event, the
only way to learn it is to ask for every album again, which cancels exactly what
the walk saves.

It is emitted **only if the row was actually written**. The album `upsert` runs
once per applied track, so twelve times for a twelve-track album; a `WHERE`
clause on the `DO UPDATE` compares the columns that carry meaning —
`updated_at` excluded, which nothing reads — and `RETURNING` yields a row only
if something moved. One event per album, never one per track, and nothing at all
for a rescan that finds no change.

The payload is empty. The track's payload carries `full_hash` because no other
surface exposes it; everything that describes an album is on the album, which
the client re-reads on receiving the event.

**An artist emits none.** The counters are derived, and a truth stored twice
goes stale. The desktop reserved one case — the artist photo, which derives from
no `GROUP BY` — and that case does not exist: `artist.artwork_hash` is read all
over this server and **written nowhere**. An artist photo cannot change because
there never is one. The question reopens the day something writes it, and on
that day this is the line to re-read.

## Decision 7 — two bounds, because one alone is always wrong

> **Built.** See the *Implemented by* line in the header.

**An age and a floor, held together.** Neither is enough on its own, and the two
fail in opposite directions.

Age alone lets a chatty library swell the table without limit — a rescan of
fifty thousand tracks writes fifty thousand rows, and thirty days of daily
rescans make that a million and a half. A count alone cuts the head off a quiet
library: ten thousand events can cover two years at an operator who adds
nothing, and trimming them by date bothers nobody, while trimming them by a
fixed quantity erases a history nothing required erasing.

So: **nothing is cut that is less than thirty days old, and never below ten
thousand events per library**, the second rule winning over the first. More than
thirty days, strictly — the bound is exclusive, and the paragraph that fixes it
below says why.

Thirty days because the bound to beat is the longest plausible absence of a
client — a holiday, a laptop put away for a month — beyond which re-reading the
catalogue is no longer a punishment. Ten thousand because a full rescan of a
large library exceeds it on its own: after a rescan an absent client *must*
start from the snapshot, and that is correct since the rescan may have moved
everything.

The contract of the two settings, written here because half of it otherwise gets
decided by accident in the code:

| Variable | Unit | Default | Absent | Invalid |
| --- | --- | --- | --- | --- |
| `WAVEFLOW_LIBRARY_EVENT_RETENTION_DAYS` | whole days | `30` | the default | start-up fails |
| `WAVEFLOW_LIBRARY_EVENT_RETENTION_MIN` | events | `10000` | the default | start-up fails |

Both go through `parse_positive_env`, so **zero and negative values are refused
at start-up**, not folded back to a default. That is the rule `validate_uploads`
and `validate_canvas` already follow, and for the reason written there: any
fallback on a bound is wrong — too small betrays the configured value, too large
is no longer a bound — so the operator is refused where they can see why. There
is no ceiling: a huge value means "purge nothing", which is safe and legible.

And the bound is **exclusive**. An event exactly thirty days old survives; only
what is strictly older is cut. That is the convention `stream_ticket::verify`
already follows — its test says `expiry is exclusive` — and it is also the
prudent direction: keeping one event too many breaks no client, cutting one too
many sends somebody back to the snapshot.

**And the watermark advances in the transaction that cuts.** `library_event` and
`library.events_purged_through` say the same thing from two places; writing them
separately leaves a window where the watermark announces less than what has
disappeared, and a client reading inside it receives a catch-up that looks
complete while skipping the hole. That is exactly the failure decision 4
refuses.

## Decision 8 — the acknowledgement, and what it does not decide

> **Built.** See the *Implemented by* line in the header.

A `library_event_ack` table, primary key `(library_id, device_id)`, carrying the
cursor and its date. It is `sync_ack` minus its account column: there the key is
`(user_id, device_id)` because the journal is *per account*; here the stream is
*per library*, and the account is read back off the device —
`device.user_id` — then checked against `library_member` like every other read
in this server.

Not `(account, device, library)`: the account would be a third column derivable
from the other two, hence a second truth to keep in agreement with
`device.user_id`. Both foreign keys cascade, so a revoked device or a deleted
library leaves no row behind.

**What the acknowledgement does not decide, and this is the point**: it does not
hold the purge back. A device that never comes back would pin the stream for
ever, and a shared library would have no retention at all the moment one phone
was thrown away. The acknowledgement *informs* — it tells the operator what a
purge will cost whom — and decision 7 *decides*. Mixing them would make a
datum the server does not control the bound of a table it must control.

## What is still open

Nothing in this RFC is open any more. The three questions it carried were
settled on 2026-08-30:

- ~~The retention values.~~ Decision 7.
- ~~Whether `album` and `artist` events are needed or merely derivable from
  `track` events by the client.~~ Decision 6, and differently for the two.
- ~~The exact shape of the acknowledgement.~~ Decision 8.

Decisions 7 and 8 are built, and this RFC has nothing pending. The *Implemented
by* line in the header names only what is running, and that is the line to
read — not this section.
