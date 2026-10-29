# RFC-008 — Receiving a file

- **Status**: Proposed
- **Implemented by**: [#149](https://github.com/InstaZDLL/waveflow-server/pull/149)
  (the negotiation), [#150](https://github.com/InstaZDLL/waveflow-server/pull/150)
  (the apply made independent of the scan), [#151](https://github.com/InstaZDLL/waveflow-server/pull/151)
  (the transfer). The *Status* field above never flips in this project: this
  line is the one that says what is running.
- **Date**: 2026-08-26
- **Revised**: 2026-09-15, after an external review. The last open question —
  sharing one blob between libraries — is settled by decision 9 below, and in
  the negative. **This RFC has no open question left.**
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md)
- **Makes possible**: balancing the two sources on the desktop, and the
  per-track video asset

## Problem

Nothing in `/api/v2` accepts incoming audio. The server is a source; it is not a
deposit. The desktop wants to upload automatically the tracks the server is
missing, which assumes exactly the opposite.

It is the heaviest of the four requests it has made, and not because it asks for
much code. It asks for little. It changes what the server *is*: receiving a file
is **spending somebody else's disk, permanently**. Almost every decision below
follows from that sentence.

## Decision 1 — a library opts in, it is not subjected

Two locks, and they answer two different questions.

The role says **who**: `Owner` or `Manager`, the same pair as
[`may_scan`](../../src/storage/database/libraries.rs) and as metadata correction, because a member
who can already spend the owner's disk on a rescan is the same level of trust.

A `library.accepts_uploads` flag, false by default, says **where**. An existing
server does not become a deposit because it was updated; its operator decides
library by library. Without that flag, the role alone would turn a read-only
installation into one that grows — a change nobody would have asked for.

A distinct `Uploader` role — a member who uploads without being able to rescan —
is conceivable and is not invented here. The two existing locks are enough to
start; a third is decided when a use demands it, not before.

Both locks are re-evaluated **at every step**: the negotiation, every fragment,
the commit. A session is not an authorization one carries away.

The server already has this rule elsewhere, and for the same reason: a playback
ticket replays the membership check on every redemption, "so revoking access
takes effect immediately instead of lasting until the ticket expires". An upload
session lasts far longer than a ticket and costs far more: a member who is
excluded, or a library its operator has closed again, must stop writing on the
next request — not at the end of a transfer that began before. The fragment is
refused, so is the commit, the working area is cleaned and the reservation
returned; the refusal takes the shape of the corresponding verdict, or a 404
when it is membership itself that has gone.

## Decision 2 — the hash before the bytes

An upload is negotiated before anything is transferred. The client announces
what it has — `full_hash`, size, extension — and the server answers with a
**named verdict**.

| Verdict | Meaning | What follows for the client |
| --- | --- | --- |
| `present` | the target library already holds these bytes | nothing to transfer |
| `accepted` | a session is open: identifier, expected fragment, expiry | transfer |
| `unsupported_format` | the extension is outside `AUDIO_EXTENSIONS` | final |
| `too_large` | above the per-file ceiling | final |
| `quota_exceeded` | the library has no room left | retryable later |
| `library_closed` | `accepts_uploads` is false | final until the operator acts |
| `too_many_sessions` | the concurrent-session ceiling is reached | retryable |

A verdict, never a boolean. `known` / `unknown` said two things at once — "I do
not have it" and "go ahead" — and kept silent about the four refusals that
nevertheless have to arrive here: **this is the only moment where a refusal is
cheap**, before the first byte rather than after the last. Naming them is what
lets a client tell what it must never retry from what it may.

One precision the table cannot carry: `library_closed` concerns only a caller
who has the right to be there. A non-member, or a member whose role is
insufficient, gets a 404 like everywhere else — RFC-002 holds that `Forbidden`
is folded into it, and a "closed" answer would confirm the library's existence
to somebody with no right to know.

The formats accepted are the ones the scanner can index —
`waveflow_core::scanner::AUDIO_EXTENSIONS` and nothing else. Accepting a file
the catalogue will not be able to read would be accepting to occupy space for
nothing.

### `present` means: in this library

The hash lookup is bounded to the target library. Two reasons, and the second is
not a preference.

The index already exists for that case:
`track_library_hash_idx ON track(library_id, full_hash, is_available)`. Looking
up by library is the shape the schema serves.

And a global lookup **would confirm an existence**. A member of library B who
announces a hash and hears `present` when only A holds those bytes has just
learned something about a library they cannot see. RFC-002 holds that a 404
blurs the absent and the foreign; a global deduplication would be exactly the
oracle that rule forbids. It would also leave B believing it owns a track it
does not have.

Sharing one blob between libraries — attaching, hard-linking, an internal copy —
was left open here until 2026-09-15. **Decision 9 settles it, and in the
negative**: two libraries, two files, two quotas.

### The negotiation happens in batches

Five thousand candidates will not make five thousand round trips. The
negotiation takes an array of hashes and returns an array of verdicts.

With one consequence not to miss: the router's body limit is sixteen kilobytes
(decision 4), which is about two hundred hashes once the JSON is counted. The
batch is therefore **explicitly bounded**, and this route carries its own body
limit like the fragments' route. An oversized batch is refused for what it is,
never truncated in silence.

The desktop reports that the bulk of the deduplication will happen on its side,
offline: its catalogue mirror already copies every track's `full_hash`, which
the native projection exposes. The negotiation serves isolated cases and other
clients, not the volume — one more reason not to over-build here.

### A session is found again, it is not duplicated

A negotiation carrying a hash for which a session is already open, for the same
account and the same library, **returns that session** — not a second one.
Otherwise every client restart abandons a working area and, with the reservation
below, ties up quota that nothing frees.

The number of concurrent sessions per account is capped, as transcoding already
is by `WAVEFLOW_TRANSCODE_PER_USER_LIMIT`. A client with five thousand files
opens as many sessions as it is allowed to hold, not five thousand.

### The quota is reserved when the session opens

Checking the quota at negotiation without holding anything back would let two
four-gigabyte sessions pass the check with five gigabytes free, and the overrun
would appear only once the disk was written.

The announced size is therefore **reserved** when the session opens, returned on
expiry or on failure, and the total re-checked at the commit. Both, not one or
the other: the reservation prevents the race, the re-check catches up with what
the reservation had only assumed.

A successful commit does not return the reservation: it **converts** it, in the
same transaction as the track. Returning then recounting would leave an interval
during which the space is free on both sides — the file is on the disk and the
quota does not count it yet — and two simultaneous commits would go through that
hole. The reservation stops being a promise at the moment it becomes a spend,
without ever ceasing to be one or the other.

## Decision 3 — an announced hash never establishes anything

The server recomputes the `full_hash` at the commit and refuses if the two
differ, throwing away what was received.

The announced hash serves to **avoid a transfer**, never to **establish an
identity**. The distinction is not theoretical: an identity founded on what the
client asserts would let any authorized member pass one file off as another, and
decision 2's deduplication would become a means of substitution rather than an
economy.

The announced size is bounded the same way, but earlier: the server counts what
it receives and interrupts the session as soon as the count exceeds the
announced size, without waiting for the hash.

That is not a second identity check — a hash that matches already guarantees the
size, they are the same bytes. It is what stops a client that lies from writing
forty gigabytes before the hash gets the chance to catch it out.

## Decision 4 — fragmented, and the global limit does not move

The router sets `DefaultBodyLimit::max(16 * 1024)`. That is not an oversight: an
API whose every route accepts sixteen kilobytes cannot be drowned by a request
body, and that property is worth keeping.

The upload route therefore carries **its own bound, per fragment**, and does not
raise the router's limit. A global ceiling of several megabytes would give every
route on the server — including those that expect only an identifier — a surface
none of them asked for.

Fragmented also because the desktop put it well: several thousand tracks over a
home connection do not go in one piece.

### A resume must not depend on an acknowledgement received

A session carries its identifier, the library, the account, the announced hash
and size, the extension, the bytes received, the index of the expected fragment
and its expiry. That state **can be read**, so that a client which restarts asks
where it stands instead of inferring it.

Three cases when a fragment `N` arrives, and only one is an error:

- `N == expected` — written, the expectation advances.
- `N < expected` — already received. An idempotent answer, with the current
  expectation.
- `N > expected` — a conflict, with the current expectation.

The second case deserves writing down rather than being endured. A fragment that
was written and whose acknowledgement is lost is the ordinary case of a
connection dropping, not a client fault; treating it as a rejection would make
the protocol fragile exactly where it was meant to absorb drops. The third is
refused for the opposite reason: a skipped fragment leaves a hole only the final
hash would reveal, far too late.

## Decision 5 — the server names the file, and it names it by its hash

The client never proposes a path, not even a name.

A whole family of problems disappears with that sentence: directory traversal,
overwriting a file the operator had filed themselves, a collision between two
simultaneous uploads. None of those problems needs solving if it is never posed.

The file lands in a subdirectory the server owns, **inside the library root**, so
that the ordinary scan finds it without being taught a second place to look.
Never a symbolic link — the walk already refuses them, deliberately.

What remains is under what name, and the question has an answer: **by hash**,
not by tags.

Filing by artist / album / title gives a tree an operator recognises, and that is
the only argument in its favour. Against it: tags move. A metadata correction, a
pass of RFC-006, a retag by the operator changes the name the path would have
frozen — and the server finds itself facing a choice it had no reason to pose:
move the file, or keep a path that lies.

The project has already settled that question the other way, elsewhere. A
metadata correction never rewrites the file; it lives beside it. **The file does
not move.** Filing by tags would reintroduce exactly the coupling that decision
set aside, on the storage side this time — a logical catalogue commanding a
physical organisation.

By hash, but **with its extension**: `<full_hash>.<extension>`, taken from the
set validated at the negotiation. That is not cosmetic. The walk only recognises
a file by its extension — the same rule that makes a `.part` fragment invisible
in decision 6 — so a file named by its hash alone would be invisible too, and
decision 7's catch-up, which counts on the next scan to pick up an orphan, would
pick up nothing. The two halves of that rule go together: what is incomplete has
no extension, what is complete has one.

The readable tree is not lost: it goes back to being what it is, a view — an
export, if it is ever asked for — and not the internal representation.

## Decision 6 — nothing partial is ever visible

Fragments are written **in the destination directory itself**, under a name that
carries no audio extension: `<uuid>.part` until the final rename.

The first version of this RFC placed the working area outside the root, which
contradicted itself: a system temporary area can be on another mount, and
`rename()` then stops being atomic — it becomes a copy followed by a delete,
with a window during which a truncated file exists. Writing in the same place as
the destination gives the same filesystem for free, and there is no longer any
cross-mount move to hope for.

What remained was invisibility, and it comes from a rule that already exists.
The walk skips no directory — `WalkDir::new(root).follow_links(false)`, with no
`filter_entry`: a hidden directory would indeed be visited, and counting on the
leading dot would have been an illusion. What the walk filters is the extension.
A fragment named `.part` is therefore unindexable **by construction**, without
having to teach the scanner a path to avoid — an extra rule a rework of the walk
could break in silence.

Two invariants, both structural now:

- the working area is on the same filesystem as the destination;
- nothing incomplete carries an extension the scan recognises.

A scan that came across a half-written file would index it as a truncated track,
with a hash that would stop being true a second later. Those two invariants make
that encounter impossible rather than improbable.

## Decision 7 — the track exists at the commit

The commit does not merely place the file: it applies it through the ordinary
scan path, and the track exists immediately.

The server already knows how to do exactly that, since metadata correction —
read a file and apply it without going through a full scan. Reusing that path
rather than writing a second one gives three things at once: the track is
readable as of the response, the library event stream announces it like any
other change, and what the commit writes is by construction what a scan would
have written.

The alternative — place the file and wait for the next scan — would leave the
client with a successful upload and a catalogue that ignores it, with nothing to
say for how long.

### An extension is not proof

Any file renamed to `.flac` passes the negotiation's check, which sees only a
string. The commit, on the other hand, really reads the file: if the extraction
fails, the upload is refused and the working area cleaned.

That check is not a piece to write — it is the decision above. Applying the file
through the scan path means having it read by the same extractor; what it cannot
read does not get in.

### The order, because there is no shared transaction

SQLite and the filesystem share no transaction. The order is therefore a
decision, not an implementation detail:

1. the size received matches the size announced;
2. the recomputed hash matches the announced hash;
3. an atomic rename to its final name;
4. the file really opens and extracts;
5. one single SQLite transaction: the track and the library event.

A crash between 3 and 5 leaves an orphan file on the disk, which the next scan
picks up — that is exactly what a scan knows how to do, and the file already
carries its final name. The reverse order would leave a catalogue row pointing
at nothing: a state nothing repairs on its own and that every read meets.

The extraction comes after the rename and not before, contrary to what the first
version of this RFC held. The extractor chooses its DSD branch on the file's
extension: reading a fragment named `.part` would take the wrong branch and
refuse a perfectly valid DSF. And the window that order opens is empty — a file
the extractor cannot read is not indexable by a scan either, since it is the
same extractor. What does not open is removed, with nothing having been able to
latch onto it in the meantime.

The library event belongs to the same transaction as the track, which
`record_library_event` already imposes by its signature — it takes the
transaction, not the connection.

### A replay does not duplicate the track

A commit replayed after the rename succeeded — the transaction failed, or a scan
passed in the meantime — finds the file under its final name. A track's identity
resolves first by its path, and the path is now that of a track that exists: the
apply updates it instead of creating a second one. That is the ordinary case of
a scan, not a special case of uploading.

What a replay can produce twice is the event. It has no idempotence key, and it
gets none here: an `upsert` states a state, not a delta, so a client receiving it
twice re-reads the same track twice and draws nothing false from it. Importing
RFC-003's `operation_id` would cost the column, its constraint and the merging
of the two vocabularies RFC-007 deliberately keeps apart, to remove a re-read.
The day an event kind states a delta rather than a state, the question will come
back — and it will be an RFC-007 question.

### A received file has no scan, and says so

The apply path required the identifier of the scan writing the track. An upload
has none: no walk took place.

Three ways out, and two are bad. Recording a fake scan job would put a fact that
never happened into the history, and that lie would come out elsewhere —
metrics, purge, the administration screen. Widening `scan_job.trigger`'s `CHECK`
constraint would force the table to be rebuilt, in order to declare that an
upload is a kind of scan when it is not one.

The column is already `NULL`able. It was the signature that was too narrow: it
now takes an optional identifier, and `NULL` means **this row does not come from
a scan** — never "unknown scan". Applying to the catalogue is not intrinsically
a scanner operation, and the signature is where that stops being implied.

That meaning exists only if the end-of-scan sweeper respects it, and it did not:
it treated `NULL` as "never seen", hence as gone. A file received while a scan
was running would have been marked unavailable by that scan — whose walk had
begun before the file existed — and a deletion would have been announced to
every client seconds after its arrival. A row with no scan is therefore swept
only if it predates the current scan, the case where the walk should indeed have
found it. The first scan to pass over a received file stamps it, and it joins the
ordinary case for good.

### The response returns the identifier and the hash

The commit response carries the created track's identifier and its `full_hash`.

That is what the desktop asked for, and the argument is right: at that moment,
and only at that moment, both sides know that their file and this track are the
same bytes. The reconciliation link is written for free. Without those two
fields, the client waits for the next catalogue walk to discover its own track,
then re-reads the whole file to link it — for information the server already
held as it answered.

### What success does not depend on

The response depends only on what the server read in the file itself. Any later
operation on that track, wherever it comes from, is a library change like any
other and travels through RFC-007's stream — it never holds the response back.

The word "enrichment" goes around in the architecture diagrams; it corresponds
to nothing in the code, and RFC-007 says why it does not enter the vocabulary
until somebody has decided on it. This RFC does not introduce it either: it lays
down the invariant — local indexing is synchronous, everything else is an
event — which stays true if such a step ever exists.

## Decision 8 — what receiving does not do

**`uploadRole` stays false.** The Subsonic facade does not accept uploads, and
announcing it because a native route exists would make clients try and fail. The
field describes the facade, not the server.

**No transcoding on the way in.** The server stores what it is given. A quality
ceiling at upload time is the client's decision; it alone knows what its
connection costs.

**No replacement.** An upload whose hash is already known stops at decision 2.
Substituting one file for another is a different operation, with questions of
its own, and it is not asked for.

**Deletion is out of scope.** The first version said "no deletion", which froze
more than this RFC has the right to freeze. A client able to fill a disk and
unable to remove what it has just put there by mistake will be reported as a
defect, not as an accepted limit — and cleaning up an expired or refused session
is already a deletion the server has to write anyway. What this RFC decides is
that it does not open a deletion route. Not that there will never be one.

## Decision 9 — two libraries, two files, two quotas

**Added on 2026-09-15, after an external review.** It was this RFC's last open
question, and decision 2 deferred it by saying that sharing a blob "is not
excluded for ever, but it requires deciding what those boundaries mean". Here
they are, and the answer is no.

**The same content in A and in B is two files on the disk and two quota
spends.** The negotiation stays strictly bounded to the target library: even if
the server happens to know that those bytes exist elsewhere, that changes
neither the verdict, nor the transfer, nor the commit, nor what is counted.

### Why the canvas could and why audio cannot

[RFC-009](RFC-009-track-canvas.md) shares its blobs, and its decision 11 says
exactly what one would like to reuse: the bytes are shared, the existence of the
hash is not, and each library pays for its own. What makes that possible is not
a boldness missing here: it is that the canvas has **a global store**,
`canvas_dir`, under the data directory, and that nothing reaches it without
going through `canvas_for_user`, which checks membership.

Audio has no store. Decision 5 holds that a received file is filed **in its
library's tree** —
`<root>/.waveflow-managed/<hash>.<extension>` — precisely so that the ordinary
scan discovers it, with no second storage model. The file's location *is* its
membership.

Wanting to share anyway leaves four ways, and each one closes:

- **A hard link** requires a single filesystem, which two roots chosen by an
  operator do not guarantee. And it makes both paths the same bytes: a library
  root is a directory its operator opens and modifies, so retagging A's file
  would retag B's. That is the worst kind of boundary — invisible, and
  discovered through its effects.
- **A symbolic link** was already ruled out by decision 5 — "never a symbolic
  link, the walk already refuses them" — and that is not a recommendation: the
  scan runs with `follow_links(false)` and skips anything `is_symlink()`
  (`src/scanner.rs`), so a linked blob would never be indexed. The portable,
  cheap option has therefore been closed since this RFC's first day, and not by
  the present decision.
- **A global content-addressed store** with references would mean undoing
  decision 5 and teaching the scanner a second storage model. That is another
  RFC, not an optimisation.
- **A reflink** would be the soundest and depends on the filesystem. It cannot
  become an invariant of this server.

### And deduplication before the transfer stays forbidden

What would remain is not transferring the same bytes twice. That is precisely
the oracle decision 2 closes: a hash answering `present` because *another*
library holds it teaches its announcer that a file it cannot see exists, and
makes it believe it owns a track it does not have. The bandwidth saved does not
pay for the leak.

### The door left open, and its lock

A **transparent physical deduplication** stays possible later — a reflink where
the filesystem allows it, falling back to a copy — **on three conditions, all
three**: that it creates no oracle between libraries, that it does not make one
library's file depend on another's, and that it changes nothing in the API
contract. That is: a storage optimisation, applied after the commit, that no
member can observe. Anything noticeable from an API is a change of semantics and
is a matter for a decision, not for an optimisation.

## What this RFC changes elsewhere

**`library_event` must carry the originating device.**

The table has neither `operation_id` nor an originating device, and its
migration comment says why: "nothing here is client-originated yet — a scan
writes every row". That was already untrue when metadata writing merged, and
uploading takes the server further from it.

Without that column, a client receives **its own upload** through the stream as
a track it is discovering, and treats it as a discovery. The user journal has
carried `origin_device_id` since its first day, for exactly this. The missing
column is an RFC-007 debt rather than a question for this one; it is noted there.

## A cost on the client, to be accepted rather than discovered

`full_hash` covers the whole file. The digest the desktop keeps locally is a
head-and-tail digest — the same compromise as `quick_hash` here, and for the
same reason: not re-reading the whole library on every scan.

"Upload everything that is missing" therefore requires reading the local library
in full, once. That is acceptable, and it is already what reconciliation
requires, but it is a decision to take with eyes open rather than a surprise at
implementation time.

## What is still open

**Nothing, since 2026-09-15.** Two of the three questions were settled while
implementing, the third by decision 9. This section's title is kept as it was
rather than rewritten: what it now recounts is how each one closed, and by what.
For the code it is the *Implemented by* line in the header that must be read,
not this section: it names pull requests, and a pull request can be checked.

- ~~**The values**: per-file ceiling, per-library quota, fragment size,
  negotiation batch size, concurrent sessions per account, session expiry.~~
  Chosen while implementing, delivered by
  [#149](https://github.com/InstaZDLL/waveflow-server/pull/149) and carried by
  `UploadLimits` in `src/config.rs`, each behind its environment variable:
  **1 GiB** per file, a **50 GiB** quota per library, **4 MiB** fragments,
  batches of **200** offers, **4** concurrent sessions per account, a **24 h**
  expiry. `validate_uploads` cross-checks them at start-up rather than letting
  one setting contradict another — a fragment cannot exceed a file's ceiling,
  nor that ceiling the quota.
- ~~**Cleaning up abandoned working areas**: on expiry, at start-up, or both.~~
  Both, and a third moment the question had not seen: `spawn_upload_sweeper`
  (`src/services/uploads.rs`, delivered by
  [#151](https://github.com/InstaZDLL/waveflow-server/pull/151)) passes at
  start-up and then once per session lifetime, and **every negotiation sweeps
  before counting a quota** — that is where the cleaning really counts, since it
  is the negotiation that decides. An abandoned file therefore lives at most
  twice the lifetime promised to its session. Every session is swept under its
  own lock, and the file goes before its row: the reverse order leaves the
  orphan this sweep exists to pick up.
- ~~**Sharing one blob between libraries**, which first requires deciding what
  those boundaries mean.~~ Settled on 2026-09-15 by **decision 9**, and in the
  negative: two libraries, two files, two quotas. The question had stayed open
  because [RFC-009](RFC-009-track-canvas.md) seemed to show the way — its
  decision 11 shares a canvas's bytes without sharing the existence of the hash.
  The review showed why audio cannot follow it: the canvas has a global store,
  audio files its file in its library's tree (decision 5), and the file's
  location *is* its membership. What is reused from decision 11 is its
  confidentiality half alone, which decision 2 already laid down here.
