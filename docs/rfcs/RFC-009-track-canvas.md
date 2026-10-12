# RFC-009 — A track's canvas

- **Status**: Proposed
- **Implemented by**: [#155](https://github.com/InstaZDLL/waveflow-server/pull/155)
  (the ticket discriminator), [#156](https://github.com/InstaZDLL/waveflow-server/pull/156)
  (the store and the model), [#157](https://github.com/InstaZDLL/waveflow-server/pull/157)
  (the six routes), [#158](https://github.com/InstaZDLL/waveflow-server/pull/158)
  (its own flag), [#163](https://github.com/InstaZDLL/waveflow-server/pull/163)
  (sweeping the store). The *Status* field above never flips in this project:
  this line is the one that says what is running.
- **Date**: 2026-08-28
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md),
  [RFC-008](RFC-008-receiving-a-file.md)
- **Closes**: the last of the desktop's four requests, arbitrated on 2026-08-24

## Problem

The desktop wants to show, during playback, a short video loop attached to the
track — what Spotify calls a *canvas*. The server has nothing of the kind: it
serves audio bytes and covers extracted from files, and a video loop is neither.

The request looks like receiving a file ([RFC-008](RFC-008-receiving-a-file.md))
and it is not one. That machinery is attached to a **library** and produces a
**track**; a canvas is neither. It lands on a track that already exists, it
creates no catalogue row, and the operator did not file it into their
collection — the server files it.

The sentence that governs RFC-008 governs this one too, at a smaller scale:
receiving a canvas is **spending somebody else's disk, permanently**. What
changes is the scale — a few hundred kilobytes rather than a gigabyte — and the
scale decides half the questions below.

## Decision 1 — the canvas belongs to the server, not to the library

The bytes go into a content-addressed store, `canvas_dir`, beside `artwork_dir`
under `data/`. Named `<hash>.<format>`, the hash being the BLAKE3 of the bytes
received, as everywhere else in this project.

**Not the library root.** This is not audio, and above all the operator did not
put it there. The root is their collection: what they back up, what they move,
what they count. Writing a server-produced object into it changes what "delete
the library" means, for a file the scanner ignores anyway — `discover_audio`
filters on the extension.

**Not `artwork_dir` either.** The `artwork` table constrains the format to a set
of images and `read_artwork` holds their MIME map; filing a video in the same
directory would force those two lists to agree for ever. A separate store does
not have that problem, and it can live on another disk.

Content addressing gives deduplication without writing anything for it: the
twelve tracks of an album sharing one loop share the bytes, and twelve rows name
them. That is the right shape — a canvas is per track, as the request puts it,
but it is almost always per album in practice.

**A `canvas` table describes the blob**, `hash` as the primary key, plus
`format`, `byte_size` and `created_at`: the same quartet as the `artwork` table,
for the same reason. The name on disk is `<hash>.<format>`, and a route that
receives only the hash cannot reconstruct it — `read_artwork` already takes
both, because the database gives it both. Guessing by trying the whitelist's
extensions one after another would be a filesystem sweep on every request, for
an answer the database holds. So the table holds it here too, and it is what
makes decision 5's counting and decision 6's sweep possible: without it, how
many references a blob has is written nowhere.

## Decision 2 — it lands beside the track, never on it

A `track_canvas` table, primary key `track_id`, `ON DELETE CASCADE`, carrying
the blob's hash and the `library_id` tenancy requires. Not a column on `track`.

That `library_id` is **derived from the track**, read in the same transaction as
the write, never accepted from the request — which is what `track_override`
already does, whose route re-reads `t.library_id` before inserting. A composite
foreign key onto `track(id, library_id)` would say the same thing to the schema,
but it would require a unique index on `track(id, library_id)` that nothing else
justifies, and the real constraint — that nobody chooses the library a track is
attached to — is already held by the read.

A canvas is **supplied by a human**: nothing in the file produces it, so no scan
can find it again. That is exactly the property that gave `track_override` its
table, and it is the right model. The cover is the opposite model and must not
be copied: the scanner extracts it, and a rescan that finds an embedded image
moves `track.artwork_hash`. A canvas link placed on `track` would live in the
`ON CONFLICT DO UPDATE SET` list of `apply_catalog_track_in_transaction`, where
one would have to remember not to touch it — until the day somebody adds a
column to that list and silently erases every canvas in the library.

**Not a column on `track_override` either.** That table carries *tag*
corrections, all of them columns of `track`, written by
`PATCH /api/v2/tracks/{id}`. A canvas carries a filesystem object with a life
cycle of its own. Housing it there would make the metadata route the silent
owner of a byte store, and would make "no canvas" indistinguishable from "no
correction": two states that look alike as long as one is looking at a null
value. A dedicated table makes the row's existence the fact itself.

## Decision 3 — the ticket says what it opens

`<video src>` cannot carry an `Authorization` header. That is the problem
[`src/stream_ticket.rs`](../../src/stream_ticket.rs) already solved for audio,
and it has to be solved a second time without writing a second module.

The ticket's payload is fixed today at 40 bytes,
`user (16) || track (16) || expiry (8)`, and `verify` refuses any other length.
Two ways: a ticket shape of the canvas's own, or a discriminator.

**A discriminator.** The payload becomes
`kind (1) || user (16) || track (16) || expiry (8)`, and `verify` returns the
kind with the pair; every route asserts the one it expects.

Both values are fixed here: **`0x01` for audio, `0x02` for the canvas**. `0x00`
is not one of them, and that is deliberate — a zero byte is what a truncated or
badly filled payload produces most readily, and the kind must not be what the
error chooses. `mint` accepts only those two values and `verify` refuses any
other, with the same indistinct refusal as a wrong length or a broken seal.
Without those constants written down, two implementations read the same ticket
differently and one of them opens the wrong resource, which is precisely what
the discriminator exists to prevent. The test vectors cover both directions: a
round trip per kind, and an unknown kind **sealed under the right key**, which
must be refused — a forged ticket would prove nothing about this contract, since
it already fails decryption.

This is not code economy. Without the discriminator, **a ticket issued for a
canvas opens the audio**, and the other way round: two resources, one of which
costs a thousand times the other in bandwidth, behind the same authorization.
The client never asked for that privilege and the operator never granted it. A
second module would have the same property by construction, but at the price of
two implementations of the AEAD seal and of the "every failure looks alike"
discipline — and two implementations diverge.

The audio's rules apply as they are: membership is re-checked on **every**
redemption, so a member who is removed loses the canvas immediately, and the TTL
bounds how long a leaked URL stays useful, not how long access lasts.

Four routes, and the reason for each:

| Route | For whom |
| --- | --- |
| `GET /api/v2/canvas/{hash}` | authenticated; immutable, `ETag` = the hash |
| `GET /api/v2/tracks/{id}/canvas` | the alias, revalidatable — it resolves the current link |
| `POST /api/v2/tracks/{id}/canvas-ticket` | mints the ticket |
| `GET /api/v2/canvas-stream/{ticket}` | what `<video src>` puts in its attribute |

The caching distinction is the one the cover already settled in PR #143:
addressed by hash, the URL can never answer differently; addressed by track, it
resolves the link of the moment and must stay revalidatable. `private` on both,
for the same reason as then — two accounts whose libraries hold the same canvas
share its hash.

**A hash is not an authorization.** That is the corollary of the previous
sentence and it is worth writing down: since two accounts foreign to each other
can hold the same one, knowing a hash establishes nothing.
`GET /api/v2/canvas/{hash}` therefore resolves it the way `artwork_for_user`
resolves a cover — by requiring that a library the requester is a member of
references it — and answers 404 otherwise. Neither `private` nor the `ETag`
replaces that check: the first is about intermediate caches, the second about
freshness. A hash nobody reachable references and one that does not exist answer
the same thing, as everywhere else in this API.

**The alias is not exempt.** `GET /api/v2/tracks/{id}/canvas` is addressed by a
track identifier, which is just as hard to guess and just as easy to share as a
hash: knowing one establishes no more than knowing the other. The route
therefore requires membership of the track's library, re-read on **every
request** and not only on the first, and answers 404 in the three cases that
must look alike — the track does not exist, it belongs to somebody else, it has
no canvas. That is this project's general rule, tenancy held in the queries and
`Forbidden` projected onto 404; it is repeated here because a route that
*resolves* a link before serving bytes is exactly the place where one authorizes
the link and forgets the track.

`canvas-stream` and not `canvas/{ticket}`: a literal segment under the same
prefix as a parameter is a trap for the next reader, even when the router
resolves it correctly.

## Decision 4 — it arrives in one piece, and the server alone names it

`PUT /api/v2/tracks/{id}/canvas`, a single body, and `DELETE` on the same route
to remove it.

No negotiation, no fragments, no session. All that apparatus exists in RFC-008
because a music file can be a gigabyte and a transfer has to be resumable. A
canvas fits in one request. Imposing a resume machine on it would cost three
times the surface for a case that does not arise.

The route carries **its own body ceiling**, derived from
`WAVEFLOW_CANVAS_MAX_BYTES`, as the upload routes carry theirs. The global
16 KiB limit does not move: an API whose every other route accepts that size
cannot be drowned by a request body.

**The server computes the hash; the client announces none.** RFC-008's decision
3 holds here in a simpler form: there, an announced hash served to *avoid a
transfer*, never to establish an identity; here there is no transfer to avoid,
so there is no reason to accept an announcement.

The format is verified by **probing the bytes received**, never by believing a
`Content-Type` or an extension. `ffprobe` is already a dependency and already on
the `PATH` of every deployment. Required: a container from the whitelist, a
video stream present, and a duration under
`WAVEFLOW_CANVAS_MAX_DURATION_SECS`. That last bound is not caution: without
it, "a short loop" becomes video hosting, which is another product with other
costs.

## Decision 5 — it is the operator's disk, and they said so once

The role required is `may_upload` — `Owner` or `Manager` — and the library must
carry `accepts_canvas`.

A canvas follows the rules of a **deposit**, not those of a correction. The line
is there: a tag correction spends bytes in a row, a canvas spends bytes on a
disk. `may_write_metadata` and `may_upload` name the same pair of roles today;
the question is not which to choose but which to follow the day they diverge,
and it is the second.

**Its own flag**, `accepts_canvas`, off by default and distinct from
`accepts_uploads`.

This RFC had first settled the opposite: one flag, answering "may a member of
this library spend the operator's disk", and both answers go together *as long
as nobody has shown a case where they come apart*. The condition was explicit,
and the case came — from the desktop, on 2026-08-30. A read-only server is the
most common installation there is, and an operator may perfectly well want a few
hundred kilobytes of loop on a machine they absolutely refuse to let grow in
audio. Tying the two made request (d) inert exactly where it is most expected —
a cost the previous version named in its open questions while believing it
theoretical.

The role does not move: `may_upload`, the `Owner | Manager` pair. It is the
*what* that separates, not the *who*.

Off by default, and above all **not inherited from `accepts_uploads`**: a
library that already takes files has said nothing about loops, and inheritance
would sign the operator up to something they did not name.

**Its own quota**, `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES`, distinct from the
uploads'. Mixing them would let loops starve the space the quota exists to
protect — the music's — and the two stores can live on different disks.

It counts **the distinct blobs a library references**, not its links: the canvas
shared by the twelve tracks of an album is billed once, which is exactly what
decision 1's deduplication saves. Billing the links would make the price of a
canvas depend on how many tracks it is attached to, and a member would pay
twelve times for bytes written once.

A blob **two libraries** reference is counted by each, although it exists only
once on the disk. That is deliberate: one library's ceiling must not depend on
what another happens to hold, or the first to place a canvas would pay for it
and the second would get it for nothing, until the first removed it and watched
its neighbour inherit the bill. The operator loses a figure that overestimates
their disk; they do not lose the ability to predict what a library can cost.

**Nothing to reserve.** RFC-008's reservation exists because a negotiation is an
open promise that nothing holds until the commit, and because two simultaneous
promises pass a check that is still counting nothing. Here there is no promise:
the body is already bounded by the route's ceiling, so the quota is checked and
spent in the same transaction, under the writer gate. No promise, no race.

## Decision 6 — the write order, and the two failures are not equal

**The file first, the row after. Then, on deletion: the row first, the file
after.** Both orders leave the same failure, and it is the recoverable one.

The store is content-addressed, hence enumerable: a sweep can list `canvas_dir`
and ask the database which hashes are still referenced. Orphaned bytes can be
found again. A row naming an absent file cannot — that is a dead link the client
sees.

**And the file leaves only with the last reference.** Taking a track's canvas
away removes a `track_canvas` row, not a blob: eleven other tracks may name it.
Deleting the row, counting the remaining references and deciding to erase the
bytes therefore hold in **one transaction**, under the writer gate — that is
what stops two simultaneous removals each concluding that a reference remains
and leaving a blob nothing names any more. The quota is returned at the same
moment, and only for the library that lost its last reference.

**The bytes leave only after the commit.** The transaction decides, the erasure
follows once the database is committed. The reverse order — erase then commit —
leaves a row naming an absent file the moment the transaction fails, which is
exactly the dead link this whole order exists to avoid. A failure between the
two leaves bytes that nothing names any more, and that is recoverable: it is the
restart, and it is what the store is enumerable for.

**But one lock covers the whole cycle.** Committing then erasing opens a window,
and it is not theoretical: between the two, a `PUT` of the same content finds no
row any more, writes its bytes, inserts its own — and the erasure that follows
takes away the file of a live link. That is the dead link again, reached from
the other end. And holding the writer gate for longer would not be enough:
placing writes the file *before* its row, so a write slipped in before the gate
and a row inserted after it would meet in the same window from the side.

**A lock per hash**, and not the writer gate. Placing takes it before writing
the bytes and releases it after inserting the row; removal takes it before its
transaction and releases it after the erasure. Placing a canvas and removing one
therefore never overlap — and two different hashes do not get in each other's
way, which the global gate would impose on them for nothing: the race is per
blob.

This is not only finer, it is the only shape available. The writer gate is
**process-wide** and must never cover file I/O — that is the rule `upload_locks`
already follows, written in `src/services/mod.rs`: "file I/O has no business
happening while the process-wide gate is held". An earlier version of this
decision asked for the opposite; it had been written without that rule in view.
The gate therefore stays what it is everywhere else, the envelope of a SQL
transaction, and the per-hash lock carries the rest.

Decision 4's `ffprobe` probe stays outside both: it touches nothing, and waiting
on a subprocess under a lock is what one does not do.

RFC-008 had the same race and could not close it: `commit_upload` observes that
another session on the same hash may be committing between its rename and its
transaction, and a gigabyte file does not serialise behind a lock. So it *kept*
it, with sessions, a working name and a guarded sweep. Here the scale allows it
to be closed.

**And the schema closes the same door a second time.**
`track_canvas.canvas_hash` references `canvas(hash)` with no `ON DELETE` clause,
so SQLite refuses to delete the row of a blob a link still names. The
transaction's count and that constraint say the same thing; the constraint is
the one that speaks last, and it is good that it exists — a count is code, a
constraint is a property.

**And the `canvas` row leaves with the last reference.** It describes a blob;
when nothing names the blob any more, it lies. It is therefore deleted in the
same transaction as the last `track_canvas` row — hence before the bytes, like
the link row and for the same reason — and never marked: a "describes a file
that is gone" state would spread through every read while being useful to none.

**The cascade bypasses all of this**, and that is worth writing rather than
discovering. `track_canvas` cascades from `track`, which cascades from
`library`: deleting a track or a library would erase the links without counting
the references, without returning the quota and without touching the bytes.
Nothing takes that path today — no route or command deletes a track or a
library, a missing track is *marked unavailable* — so it is a trap set for later
rather than a current defect. The day either deletion exists, it goes through
decision 6's transaction or it leaves behind a store only the sweep catches up
with. `artwork` already has exactly this property and nobody had written it
down.

This is the reverse of RFC-008's order, and without contradiction: there the
session row **was** the only memory that a working file existed, under a name
nothing enumerates. Here the content remembers itself.

## Decision 7 — nothing to announce, except the link

[RFC-007](RFC-007-library-event-stream.md) has already settled it, and this RFC
merely conforms: no event for a content-addressed resource. The bytes behind a
hash never change. Only a track's **link** to a canvas is a change, and it
travels in the track's `upsert`.

Placing or removing a canvas therefore emits a `track` / `upsert` event through
the same `LibraryChange` path as a metadata correction, with the originating
device PR #152 has just added — without which the client that just placed the
canvas would receive it as a discovery. Nothing new is needed, and that is the
measure that RFC-007 was built in the right place.

**The event's payload does not change either.** It carries
`{ "full_hash": … }` on an `upsert` today, and nothing else: the digest is there
because **nothing else carries it** — a track keeps its identifier while its
bytes move, and the event is the only witness. A canvas link is not in that
case; it is read off the track, which the client re-reads anyway on receiving
the event. Adding a field, and a `canvas: null` to say it has gone, would make
the payload a partial projection of the track — a second model to keep in
agreement with the first, for information already available one round trip away.

## Decision 8 — what the canvas does not do

- **It is not transcoded.** The server stores and serves what it is given. The
  FFmpeg chain is the audio's; attaching the canvas to it would make it a
  compute load per playback, for a loop of a few seconds.
- **It is not extracted, nor generated.** No file contains it and the server
  fetches nothing from outside — RFC-007's refusal of enrichment applies as it
  is.
- **It does not appear in the Subsonic facade.** The contract is frozen for
  `v2.0-beta`, and no validated client asks for it.
- **It does not travel in a share.** `/share/{token}` serves audio to a visitor
  with no account; adding a surface there is decided separately, not in passing.

## Decision 9 — the sound goes at ingestion

Opened by the first version of this RFC, which set out three options without
choosing: refuse an audio stream, ignore it on playback, or let the client
decide. Settled on 2026-08-30, and it had to be settled because the code had
already settled it **by omission** — the probe required a video stream and said
nothing about audio, so a canvas with sound was accepted without anybody having
decided it.

**The third option does not exist.** The desktop renders its `<video>` with
`muted` hard-coded, plus `aria-hidden` and `pointer-events-none`: that is not a
setting, it is what a canvas *is* — a loop running over a track that is already
playing. No client can decide to hear that sound without talking over the music.
"Let the client decide" therefore came down to storing and serving bytes nobody
will ever hear.

And those bytes cost. They are taken from the 4 MiB ceiling, at the expense of
the image the same bytes could have paid for.

**The real choice is therefore between refusing at the probe and accepting while
stripping the track.** It is the second. Refusing would reject an otherwise
valid mp4 over a stream whoever sent it did not necessarily choose to include —
many encoders write an empty track by default — and for a reason the operator
cannot diagnose by looking at their video.

**It is a remux, not a re-encode**, and decision 8 is no obstacle. Its argument
was that transcoding would make the canvas a compute load *per playback*; here
it is a stream copy, once, at ingestion, and the image comes out bit for bit
what it was.

**The hash is computed afterwards.** The store is content-addressed, so two
members offering the same file must produce the same output, or decision 1's
deduplication silently stops working and each pays for their own blob. The remux
is therefore made reproducible — metadata dropped, output *bit-exact*. That
holds inside one deployment; two versions of FFmpeg can still mux the same
packets differently, so a server upgrade may give a second blob to the same
source file. That costs a duplicate, never a wrong answer.

## Decision 10 — who picks up the bytes a crash left behind

Decision 6 says when a blob stops being referenced and by which transaction; it
left open *who* picks up what a crash left behind. A sweeper, in the shape of
RFC-008's: one pass at start-up, then one a day. Orphans are made by crashes and
not by use, so this is a repair and not maintenance — on a server that has not
crashed, the pass finds nothing and costs one directory read.

**Three kinds of file, and only one is deleted without reservation.**

A blob no `canvas` row names any more goes — but under its hash's lock, and the
count is re-read inside. Without that lock, this sweeper would be the cause of
the damage it exists to repair: a placement writes its file, and between that
write and its row the sweeper sees bytes nothing names and takes them, leaving
the placement to commit a row whose file has gone.

A working file — `<uuid>.part`, `<uuid>.silent` — goes if it is older than an
hour. Those names belong only to the call that wrote them: no row mentions them
and there is no lock to take for them, so age stands in for both. An hour is far
beyond anything a live placement could still be doing.

The stem is checked as strictly as a blob's: it is a UUID this module invented,
and requiring it costs nothing. Without that requirement, the operator's
`progress.part` would be the only thing in the store to carry an extension
enough to make it disappear — an asymmetry that would contradict the next
paragraph's rule at the very moment it is stated.

**And everything else is counted, never deleted.** The store lives under the
operator's `data/`. A sweeper that erases what it cannot name is a sweeper
nobody should run: a file is a candidate only if it is `<64 hex>.<format>` for a
whitelisted format, or one of the two working names this module writes.

**A row with no file is reported and never repaired.** That is the failure
decision 6 calls unrecoverable — a dead link the client sees — and the "repair"
would consist of deleting somebody's canvas. Answering a dead link by making it
absent is a worse answer than telling the operator the truth.

## Decision 11 — the bytes are shared, the knowledge is not

Decision 5 sets the price of a blob two libraries hold — each counts it —
without saying what the boundary means. Here it is, and the main point is that
it describes what the code already does rather than what it ought to do.

**The file is shared, the existence of the hash is not.** One copy on the disk,
and `canvas_for_user` only gives access to it through a library the requester is
a member of. A hash nobody reachable references and a hash that does not exist
answer the same thing. RFC-008 had already settled the analogous question and
for the reason that holds here: a *visible* deduplication between libraries
would be the oracle the 404 rule forbids — it would tell B that a file it does
not own exists somewhere.

That is why each pays. It is not accounting clumsiness: billing the first
depositor would make one library's ceiling depend on what another happens to
hold, and the day the first removed its canvas, its neighbour would inherit a
bill it never saw coming. The operator loses a figure that overestimates their
disk; they do not lose the ability to predict what a library can cost.

**What stays outside this RFC** is what RFC-008 had already set aside:
attaching, hard-linking or copying a blob from one library to another. Those are
operations that *move a boundary* rather than respect it, and none is asked for.
They are not excluded for ever; they require deciding what one library may learn
about another, and that is not a question the canvas should settle for everybody.

## What this RFC changes elsewhere

**The stream ticket's payload grows by one byte**, and tickets minted before the
upgrade stop validating — `verify` checks the exact length. The damage is
bounded by the TTL: a browser in the middle of a seek gets a 404 and mints
again. That deserves saying rather than discovering, but does not deserve a
version field: the format is sealed under the instance key and is never
persisted.

**`Config` gains a directory and three bounds.** `canvas_dir`,
`WAVEFLOW_CANVAS_MAX_BYTES`, `WAVEFLOW_CANVAS_MAX_DURATION_SECS`,
`WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES` — and the values that only make sense
against one another are validated at start-up, as `validate_uploads` already
does.

**The values chosen**, decided while implementing and delivered by PR #156:

| Setting | Default | Why that one |
| --- | --- | --- |
| `WAVEFLOW_CANVAS_MAX_BYTES` | 4 MiB | A real loop weighs a few hundred kilobytes. Comfortably above anything that is still a loop, low enough that the derived body ceiling stays a ceiling. |
| `WAVEFLOW_CANVAS_MAX_DURATION_SECS` | 15 | These loops run three to eight seconds. Room for an unusual one, not for an episode. |
| `WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES` | 1 GiB | Thousands of real canvases, and a number an operator can hold against a disk. |
| The container whitelist | `mp4`, `webm` | The two a browser and a desktop read with no decoder to ship. Short on purpose: every entry is a format the server promises to serve for ever, since the bytes behind a hash never change. |

`canvas_dir` is derived from `WAVEFLOW_DATA_DIR` rather than configured on its
own, exactly like `artwork_dir`.

## What is still open

Nothing. No question of shape was open — decision 11 closes the last one — and
the only work still pending has been delivered.

- ~~**Sweeping `artwork_dir`.**~~ Delivered by
  [#169](https://github.com/InstaZDLL/waveflow-server/pull/169), in
  `src/services/artwork.rs`. It was not `sweep_canvas_store` with another
  directory, contrary to what this entry assumed: the cover is written by
  `waveflow_core::scanner::extract_cover`, from a blocking task in a crate that
  has never heard of `DomainServices`. There is no lock to take, and the writer
  gate does not stand in for one — `DomainServices`'s comments forbid file I/O
  under that global gate. Age therefore stands in for a lock, as it already does
  for the canvas's working files. The store also carries `<hash>_1x.jpg` and
  `<hash>_2x.jpg` thumbnails that nothing in the database names.

The *Implemented by* line in the header is the one to read, not this section: it
names pull requests, and a pull request can be checked.
