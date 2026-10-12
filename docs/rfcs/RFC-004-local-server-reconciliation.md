# RFC-004 — Local ↔ server reconciliation

- **Status**: Proposed
- **Server side**: nothing to implement. Decision 1 says so itself — the first
  M5 increment changes neither the server catalogue nor `/api/v2`. This RFC
  lives here as the reference for the contract the desktop consumes.
- **Date**: 2026-08-17
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-003](RFC-003-waveflow-sync-v2.md) and Desktop RFC-005
  `remote-source-and-sync-v2`

## Problem

WaveFlow Desktop presents two separate catalogues today: its local files and
WaveFlow Server's authoritative catalogue. That separation is correct for M4,
but it makes it impossible to recognise that a local file is the same byte-for-byte
copy as a remote track. In particular it rules out explicitly converting a
playlist from one source to the other, or preferring the local file when it is
available.

Reconciliation must never manufacture an identity out of metadata. A title, an
artist, a duration — or even a set of those values — can name several
recordings, editions or encodings.

## Decision 1 — the link is Desktop data

The first M5 increment changes neither the server catalogue nor `/api/v2`.
WaveFlow Server already publishes, for every track, its stable UUID, its size
and its `full_hash`, defined as unkeyed BLAKE3 in hexadecimal over the whole
file. Desktop alone holds the identifier of its local track and access to its
file, so it persists the link in the database of the profile concerned.

The link is strictly one-to-one and holds at least:

- the stable local identifier;
- the remote identifier, scoped by profile and server;
- the method (`exact_full_hash` or `confirmed_mbid`);
- the proof used and the confirmation/verification dates;
- a state of `confirmed`, `stale` or `rejected`.

The unique constraints are on both identifiers. An extra local or remote copy is
an ambiguity to resolve, never a second automatic link. Synchronising these
links across devices would need a separate server contract; it is outside the
first M5 increment.

## Decision 2 — only an exact and unique full hash links automatically

Desktop's current `track.file_hash` is not comparable to `full_hash`: it folds
in the size and samples only the beginning and the end of large files. Desktop
must use the whole-file BLAKE3 computation it already has.

To avoid re-reading the entire library:

1. index the reachable remote tracks by `(size, full_hash)`;
2. compute the full hash only for local files whose size has at least one remote
   candidate;
3. link automatically only when exactly one reachable local file and exactly one
   reachable remote track share the full hash;
4. present any multiplicity as a candidate to confirm.

The hash fingerprints the file, not the decoded audio. Two encodings, or two
files carrying different tags, therefore do not match even when they are the
same recording.

## Decision 3 — an MBID stays a candidate to confirm

A MusicBrainz identifier never causes an automatic link: several pressings,
encodings or copies can share one recording identifier. An MBID match can only
raise a candidate that the user confirms after seeing both sources.

The server currently stores no MBID. The MBID branch cannot begin until the
canonical identifiers the scanner, the catalogue and the public contract need
have been added explicitly. Without them, M5 works entirely off the full hash;
it does not infer an MBID from a title or an artist.

## Decision 4 — no fuzzy matching

Title, artist, album, duration, track number, size alone and any combination of
those fields are forbidden as grounds for confirming a link. They may only help
the user read a proposal already founded on an ambiguous exact hash or on an
MBID.

A rejected candidate does not come back until its proof — hash or MBID — has
changed. That memory is what keeps the same suggestion from pestering the user.

## Decision 5 — surviving moves and changes

The link references the stable identifiers of both catalogues, never their paths
or their metadata. It therefore survives a move and a tag change, as long as the
scanners keep the identifiers.

If the file's bytes change and its full hash no longer matches the recorded
proof, the link becomes `stale`. It is neither deleted nor silently pointed at
another track. A fresh verification or confirmation is required.

An unavailable track keeps its link. Availability decides only which playback
source is usable at that moment.

## Decision 6 — the proposed playback policy

For a linked track, Desktop prefers the local file when it is available and
readable, then falls back to the server stream. The interface says which source
is active and allows forcing it for the current session. An unlinked track keeps
the behaviour of its own source.

A link does not turn the local track into a server track and does not merge the
catalogues. Remote UUIDs go on naming the mutations and the user data the server
owns.

## Decision 7 — no implicit merge of user data

Creating or confirming a link changes no favourite, rating, history, counter,
playlist or queue.

- **Favourites and ratings**: they stay proper to their source. An explicit copy
  may be offered in a direction the user chooses; there is no automatic rule
  about which one wins.
- **History and counters**: historical events stay with their original
  authority. A display may compute a combined total without persisting it. A new
  play may feed the local history once and, if it has a remote link, emit an
  idempotent scrobble to the server; those are two projections of one event, not
  two plays.
- **Playlists**: an explicit action publishes a local playlist to the server or
  materialises a server playlist locally. A preview names the unlinked or
  ambiguous tracks before it is confirmed. There is no silent merge of ordered
  lists.
- **Queue**: the active playback source owns its queue. No link merges the two
  queues automatically.

## Security and isolation

Desktop considers only the remote tracks the linked account can see. Withdrawing
access makes the link unusable without revealing anything new. Local paths never
leave the device. The first increment sends the server no links, no local
inventory and no new hash.

## The minimal user journey

1. start a match analysis from the remote source;
2. see the automatic exact links, the ambiguous candidates and the rejections;
3. confirm or reject a candidate;
4. see and change the playback preference of a linked track;
5. convert a playlist explicitly, with a preview of the unlinked items;
6. run the verification again after a file or access change.

## Validation

The M5 gate requires at least:

- a unique exact match and a verified whole-file computation over MP3, AAC, FLAC
  and WAV files;
- same size, different content: no link;
- local or remote duplicates: no automatic link;
- a local and a remote move: the link is kept;
- a tag change that alters the bytes: the link goes `stale`, never reassigned;
- identical metadata with no exact hash: no link;
- membership removed from a library: no catalogue leak;
- a playlist conversion with linked, unlinked and ambiguous tracks;
- confirming a link: no implicit mutation of user data;
- real files from the validation library, with no tag writing at all.

## Implementation sequence

1. a Desktop migration and repository for the links;
2. discovery by size then by full hash, with cancellation and progress;
3. the confirmation screen and the state handling;
4. local-first selection with a server fallback;
5. explicit playlist conversion;
6. explicit actions on favourites/ratings, and combined history displays;
7. MBID only after a dedicated data contract;
8. portability across devices only in a later RFC.

## Trade-offs to accept before implementation

This RFC proposes the following conservative values:

1. **local-first playback**, with a server fallback;
2. **no automatic copy** of favourites or ratings when a link is made;
3. histories stored separately, with a combined total in the display only;
4. links **local to the Desktop profile** for the first M5 increment.

Changing any of those four alters the experience or the data model, so they have
to be approved before this RFC moves to `Accepted` and implementation begins.

## Out of scope

- acoustic fingerprinting, or hashing the decoded audio;
- fuzzy matching founded on metadata;
- rewriting tags or audio files;
- physically merging the local and remote catalogues;
- synchronising local paths or inventory to the server;
- biography, artist images or enrichment from third-party services.
