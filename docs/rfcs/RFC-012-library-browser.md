# RFC-012 — The library browser: folders as a view, filing as metadata

- **Status**: Proposed
- **Implemented by**: every step of "The order of the work" below, in that
  order, after this RFC itself ([#297](https://github.com/InstaZDLL/waveflow-server/pull/297)):
  [#298](https://github.com/InstaZDLL/waveflow-server/pull/298) (the
  context menu and Properties, with `relative_path` on the track),
  [#299](https://github.com/InstaZDLL/waveflow-server/pull/299) (download,
  `?download=1` on the stream ticket),
  [#300](https://github.com/InstaZDLL/waveflow-server/pull/300) (uploaded
  lyrics: `track_lyrics_upload`, the TTML parser, the order, and the lyrics
  download), [#301](https://github.com/InstaZDLL/waveflow-server/pull/301) (the
  disk tree, `GET /libraries/{id}/tree`, and the Folders page) and the pull
  request adding the filed tree (`library_folder`, `track_placement`, their
  routes, "Move to folder…", drag and drop, and uploads into a folder). The
  *Status* field above never flips in this project; this line is the one that
  says what is running.
- **Date**: 2026-10-21
- **Revised**: 2026-10-21, after review on #297: the **Received** folder's
  contract, the depth check on a move, the folder cap that bounds the tree's
  listing, and one order for the lyrics, all made explicit. Then the root's
  depth, the cap under concurrency, all-or-nothing placements, empty sheets
  skipped, and lyrics downloads that are never rendered. Last, listeners'
  access to the disk tree, settled in decision 3 rather than left open. And
  where Properties reads its lyrics from, and how the lyrics route projects
  the order. Then, 2026-10-22, after use: the web client draws the filed
  tree only, and the disk tree stays an API (see "What the web client
  draws" under decision 1).
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md),
  [RFC-008](RFC-008-receiving-a-file.md)
- **Amends**: nothing. Every decision below keeps RFC-002's "audio files are
  always read-only" and RFC-008's "the file does not move" as they stand.

## Problem

The web client shows a catalogue: albums, artists, songs, genres. What it
cannot show is **where things are**, and what it cannot let anyone do is **put
them somewhere**. The owner of a library asks for five things, in their words:

1. a file manager to see the music and the folders it lives in;
2. folders of their own, to sort the music into;
3. a right click on a track that opens its properties;
4. uploading and downloading music from that manager;
5. the same for lyrics: `.lrc`, `.txt` and `.ttml`.

Three of these are within what the server already allows. Two of them, read
literally, are not: creating folders and sorting files into them, done on the
disk, is exactly what RFC-008 decision 5 rejected. Filing by tags was turned
down because "tags change", and the server would then have to "move the file,
or keep a path that lies". RFC-008 left one door open: a readable tree is "a
view — an export, if it is ever asked for".

This document walks through that door and no further. **Nothing on disk is
created, moved, renamed or deleted by anything specified here.** The folders a
user makes live in the database, and the disk is shown as it is.

## Decision 1 — two trees, and only one of them can be changed

The browser shows a library as two trees, side by side, and never mixes them.

- **On disk** is the library as the scanner found it: the folders of each
  indexed track's `relative_path`, and the tracks in them. It is **read-only**.
  It has no "new folder" and no "move", because what it would change is the
  owner's file system.
- **Filed** is the library as its owner and managers sort it: folders that
  exist only in WaveFlow, holding tracks. It is **editable** by those who may
  correct metadata (decision 3), and a change to it touches no file.

They are two trees and not one tree with two kinds of node. A single tree that
mixed real folders with virtual ones would let a drag look like a move on disk
while being nothing of the kind. Two trees say, from their titles, which one
can be changed.

### What the disk tree shows, and what it cannot

The disk tree is **derived from `track.relative_path`**, which the scanner
already stores with `UNIQUE(library_id, relative_path)`. It is not a live
directory listing. That has two consequences, and both are accepted:

- a folder holding no indexed audio (cover art only, an empty folder, an
  unsupported format) **does not appear**;
- what appears is what the last scan saw, not what the disk holds now.

A live listing would answer both. It would also make every page of the browser
a walk of the owner's file system under the request's credentials, through the
path guard, and expose names the scanner was never asked to read. The view is
of the catalogue, not of the disk.

Files received through RFC-008 live in `.waveflow-managed/` under their hash.
Showing `9ad0…f9e.flac` would be truthful and useless, so that one folder is
presented differently, and its contract is stated here rather than left to the
client:

- **Its path is its real one.** It is reached as `GET
  /libraries/{id}/tree?path=.waveflow-managed`, the same address scheme as
  every other folder, and the root's listing returns it under that `path`.
  The name **Received** (translated) is a label the client draws; the stable
  identifier is the path, which RFC-008 fixes and nothing renames.
- **It is flat.** RFC-008 writes no sub-folder there, so the folder has no
  sub-folders and no tag grouping is invented for it: its tracks are listed
  directly, ordered by artist then title, case-insensitively and NULLS LAST.
- **Its count is its tracks**, counted like any other folder's: the indexed
  tracks whose `relative_path` sits directly in it.
- **A tag change moves nothing.** A received track's place in the tree is its
  path, which is its hash; correcting its title or artist changes how its row
  reads and where it sorts, never which folder holds it or its address.

### Paths are shown to members, and only to them

`GET /api/v2/tracks/{id}` does not return `relative_path` today; only the
library's own track listing does. The browser and the properties panel need
it. It is added to the track's answer under the existing tenancy: a caller who
may read the track may read where it lives in the library. The **root** of the
library — the absolute path on the server — is never returned, to anyone. A
path is always relative to its library.

### What the web client draws

The two trees shipped side by side (#301, #302), and the owner read them
together on the first day. A file uploaded into the folder **Test** showed
in **Test** under *Filed* and in **Received** under *On disk*: true twice,
and read as the file manager contradicting itself. Two trees whose titles say
which one can be changed did not stop a person from asking where the file
*is*, and each answered differently.

So the web client draws **the filed tree only**. A track has one place on
its Folders page: its folder, or **Unfiled**. The disk tree stays what this
decision defines — `GET /libraries/{id}/tree`, read-only, open to members —
for any client that wants to show where files lie, and its contract does not
change. A client that draws both should not put them on one screen.

## Decision 2 — a filed folder is a place, not a collection

A track sits in **at most one filed folder** per library. Moving it to another
folder takes it out of the first.

That is the difference between a folder and a playlist, and the reason this
document does not just let playlists nest. A playlist holds the same track as
often, and in as many lists, as its owner likes; it has an order and belongs
to one user. A folder answers "where does this go?" and has one answer. If
it allowed many, it would be a second kind of playlist with a tree drawn on it,
and "sorting the music" would stop meaning anything.

A track in no filed folder is **unfiled**. The filed tree shows the unfiled
tracks under a fixed entry at its root, so a freshly scanned library is
entirely visible there before anyone has sorted anything.

### The shape

- A folder has a name, an optional parent in the same library, and a position
  among its siblings is **not** kept: siblings sort by name, case-insensitively
  and NULLS LAST like everything else. An order nobody asked for is an order
  that has to be synchronised, and nobody asked.
- A name is 1 to 120 characters after trimming, holds no control characters,
  and is unique among its siblings, case-insensitively. It holds `/` if its
  author wants it to: no path is ever built from it.
- Depth is capped at 16. The root — `parent_id: null` — is at depth 0, so a
  folder at the root is at depth 1, and a create is refused with `422` when
  its parent is already at 16.
- **A move is checked for the whole sub-tree it carries**, before anything is
  written: the new parent's depth (0 for the root) plus the height of the
  moved folder's sub-tree, the folder itself counting 1, must stay within 16,
  or the move is refused with `422`. Checking
  only the moved folder would let a move bury its deepest descendant below the
  cap. A move that would make a folder its own ancestor is refused with `422`
  too, and neither refusal is repaired.
- A library holds at most **5,000** filed folders. A create past that answers
  `422`. The cap is what makes the whole tree one bounded answer (see the
  surface below): at most 5,000 entries of an id, a parent id, a name of at
  most 120 characters and a count. It bounds the number of entries, not a
  byte size, and promises none.
- **The cap holds under concurrency.** A create counts the library's folders
  and inserts in one transaction, under the process-wide writer gate every
  mutation already takes (CLAUDE.md, "SQLite discipline"), so two creates
  racing at 4,999 cannot both pass: the second sees the first and answers
  `422`. The depth checks and the sibling-name uniqueness are made in that
  same transaction, for the same reason.
- Deleting a folder **files nothing elsewhere and deletes nothing on disk**.
  Its sub-folders go with it, and every track in it or under it becomes
  unfiled. The response says how many tracks were unfiled, so the client can
  say it.

### What follows a track, and what does not

The placement is keyed by the track id. A track keeps its id across a move on
disk and a re-encode that keeps its relocation hint (CLAUDE.md, "Identifiers
and time"), so its filing survives both. A file that comes back as a new track
— moved **and** re-encoded with nothing to match it by — comes back unfiled.
A track deleted by a scan takes its placement with it (`ON DELETE CASCADE`).
Nothing in the scanner reads or writes placements.

## Decision 3 — who may file, and who may only look

Filing is a metadata correction in all but name: it changes how a library is
presented to everyone in it, and never the files. It is therefore gated
exactly like one, by `LibraryRole::may_write_metadata` — **owner and manager**.
A listener browses both trees and opens properties, and is refused every write
with the same `404` a foreign library would give, so the refusal confirms
nothing (CLAUDE.md, "404 blurs everything").

**Both trees, the disk one included.** A listener reads the disk tree through
`GET /libraries/{id}/tree?path=` exactly as an owner does. Hiding it would hide
nothing: every member already reads each track's `relative_path` through the
library's own track listing, and the tree is only those paths, grouped. A disk
layout its owner wants private belongs in a library that is not shared.

The filed tree is **shared by the library's members**, not kept per user. A
per-user tree would be a private way of sorting a shared library, which is
what playlists already are.

## Decision 4 — properties are what the server already knows

The right click opens a menu: **Properties**, **Download**, **Edit tags**,
**Add to queue**, **Play next**, and, for those who may file, **Move to
folder…**. Properties shows, read-only:

- the file: its path in the library (decision 1), format, codec, bit rate,
  sample rate, bit depth, channels, size, duration, the content hash, and the
  date it was added;
- the tags as read, and any override beside them, each marked as such;
- ReplayGain, BPM, ISRC, the MusicBrainz recording id;
- which lyrics the track has and where each came from (decision 6).

Two reads feed it, and the panel merges them. **`SongItem` carries every
field above that is not about lyrics**, the path added through `GET
/tracks/{id}` (decision 1), and the corrections come beside it from `GET
/tracks/{id}/overrides`. **The lyrics, and the source of each sheet, come from
the lyrics projection** of decision 6 — the same answer, in the same order,
that the lyrics route gives every client — and not from `SongItem`, which
carries none. No new analysis is run to fill the panel.

The menu is reachable without a mouse: the context-menu key and Shift+F10 on a
focused row, and a long press on touch. A menu reachable only by right click
is a menu the accessibility gate is right to fail.

## Decision 5 — download is the stream, with a different header

Subsonic's `download` already serves the original as an attachment. The
native API serves originals only as a stream. The browser cannot send an
`Authorization` header on a navigation, which is why streams use tickets.

So download **is** the stream ticket: `GET /api/v2/stream/{ticket}?download=1`
answers the same bytes, through the same path guard and the same re-check of
access, with `Content-Disposition: attachment` and the file's own name. No new
ticket kind, no new route, no second way to read a file. The name is the
file's basename for a scanned track, and `Artist - Title.ext` for a received
one, whose basename is a hash.

Who may download is who may stream. Gating them differently would be a
distinction without effect: anyone who can stream a file can save the stream.

## Decision 6 — uploaded lyrics live beside the file, never in it

`.lrc`, `.txt` and `.ttml` can be uploaded to a track by those who may file
(decision 3). They are stored in the database, **in a table of their own**,
and no file is written next to the audio.

A table of its own because `track_lyrics` belongs to the scanner: every scan
rebuilds a track's rows from what the file and its sidecars hold, and would
erase anything else on the next pass. Uploaded lyrics are a correction, and
corrections live beside the file (RFC-008 decision 5).

- **One upload per track and format.** Uploading again replaces it. Deleting
  it falls back to what the scanner found.
- **Only a sheet with something in it counts.** An upload with no usable
  content is refused at upload with `422`: a `.txt` empty after trimming, an
  `.lrc` with no line, a `.ttml` from which no line parses. And the selection
  below skips any source that is empty for whatever reason — a scanned row
  the constraints let through, an upload stored by an earlier version — and
  goes on down the order, so an empty sheet never hides a real one.
- **Precedence** is one fixed order, the same everywhere a single sheet is
  chosen — the lyrics the clients read, Properties, and the download below:
  1. an uploaded `.ttml`;
  2. an uploaded `.lrc`;
  3. a scanned synced sheet (a sidecar `.lrc`, then embedded synced lyrics);
  4. an uploaded `.txt`;
  5. scanned plain text (a sidecar `.txt`, then embedded plain lyrics).

  Timings beat their absence, and "uploaded" only breaks ties between sheets
  that are alike: an owner who uploads a `.txt` for a track whose file carries
  a synced sheet has not asked to lose the timings. Between the two synced
  uploads, TTML ranks first because it carries strictly more — line timings
  and, kept in the original, word timings — so an owner who uploads both has
  given the richer one last word. The order depends on formats only, never on
  upload times, so two readers of the same track always agree. It is applied
  identically wherever one sheet is chosen: the lyrics served to clients,
  Properties, and the default download.
- **The lyrics route projects that order.** `GET /api/v2/tracks/{id}/lyrics`
  (RFC-002) keeps its `LyricsList` shape and keeps listing every sheet the
  track has, uploaded ones included, **sorted by the order above with empty
  sources left out**, so its first entry is always the selected sheet. A
  client that shows one sheet shows the first, and Properties reads the list
  as it comes. Each entry says where it came from, so the panel can name the
  source. The default download serves that first sheet; `?format=` is the
  only way to ask for another, and only for an upload.
- **The original bytes are kept**, decoded to UTF-8 through
  `waveflow_core::text_encoding` (#296), so the download gives back what was
  uploaded and not a re-serialisation of it. The size cap is the sidecar's,
  1 MiB.
- **TTML** is parsed for its lines: each `<p>` with its `begin`, in the three
  clock forms TTML allows. The word-level `<span>` timings are kept in the
  stored original and **not served**: the native API and the frozen Subsonic
  contract both carry line timings only, and `songLyrics` v2 is deliberately
  not advertised (RFC-002). Serving word cues is a separate change with its
  own contract. The parser is `roxmltree`, already in the lock file, with
  external entities and DTDs refused.
- **Download** of lyrics gives back the sheet the order above selects: the
  uploaded original as it came when that sheet is an upload, and, for lyrics
  the scanner found, `.lrc` when they are synced and `.txt` when they are
  not. `?format=lrc|txt|ttml` asks for one upload by name instead, and
  answers `404` when the track has no upload in that format.
- **A download is never rendered.** It answers `Content-Type:
  application/octet-stream`, `Content-Disposition: attachment` with the
  file's name, and `X-Content-Type-Options: nosniff`, whatever the format.
  An uploaded `.ttml` is XML its uploader wrote; served inline under an XML
  or HTML type it would be a document the browser runs in the server's
  origin. The selection and the bytes are unchanged; only how the browser is
  told to treat them is fixed.

## Decision 7 — uploading from the browser is RFC-008, unchanged

The browser's upload is RFC-008's: same negotiation, same hash naming, same
`.waveflow-managed/` folder, same opt-in per library. A file dropped onto a
filed folder is **received first and filed second**: the client commits the
upload, then places the returned track id. The upload contract does not learn
about folders. A client that cannot file still uploads, and a received track
that is never filed is simply unfiled.

The browser never offers to upload into the disk tree. That tree shows where
files are, and RFC-008 decides where received ones go.

## The surface

All under `/api/v2`, all tenancy-checked in the queries, all `404` for a
library or a track the caller may not see.

| Route | What |
| --- | --- |
| `GET /libraries/{id}/tree?path=` | One level of the disk tree: sub-folders with their track counts, and the tracks directly in `path` (empty for the root; `.waveflow-managed` for **Received**). Paged, like every listing. |
| `GET /libraries/{id}/folders` | The whole filed tree, without tracks, as one flat list of `id`, `parent_id`, `name` and the count of tracks filed directly in each, under `folders`, beside `unfiled_track_count` for the root's unfiled entry. Not paged: bounded by the 5,000-folder cap of decision 2, which is what lets a client draw the tree from one answer. |
| `POST /libraries/{id}/folders` | Create: `name`, `parent_id`. |
| `PATCH /folders/{id}` | Rename, or move under another `parent_id` (or to the root); `422` past the depth cap for the whole sub-tree, or on a cycle. |
| `DELETE /folders/{id}` | Delete with its sub-folders; returns the number of tracks unfiled. |
| `GET /libraries/{id}/folders/{folder}/tracks` | The tracks filed directly in a folder. Paged. `unfiled` in place of a folder id stands for the root's unfiled entry, which is why the library is in the path: unfiled names no folder to find it by. |
| `PUT /libraries/{id}/placements` | File up to 500 tracks into one folder, or unfile them (`folder_id: null`). **All or nothing**: the folder and every track are checked before anything is written, in one transaction under the writer gate; any track the caller may not see, or outside the library, answers `404`, a folder outside it `404`, more than 500 `422`, and no placement changes. |
| `PUT /tracks/{id}/lyrics/{format}` | Upload `lrc`, `txt` or `ttml`. |
| `DELETE /tracks/{id}/lyrics/{format}` | Remove an upload. |
| `GET /tracks/{id}/lyrics/file?format=` | Download the sheet decision 6's order selects, or with `format`, one upload by name. |
| `GET /stream/{ticket}?download=1` | Download the audio (decision 5). |

`GET /tracks/{id}` gains `relative_path`. Nothing else changes shape.

Every write goes through `DomainServices`, so a future façade or the desktop
reaches the same rules (CLAUDE.md, "One set of domain services").

## What this RFC changes elsewhere

- **The Subsonic façade: nothing.** `getIndexes` and `getMusicDirectory` stay
  tag-based. Exposing either tree through them would change wire behaviour the
  frozen v2.0-beta contract promises clients that were validated against it.
  For the same reason `getLyrics` and `getLyricsBySongId` keep answering the
  scanned sheets as they do today; serving uploaded lyrics there is a change
  to that contract, for its next version, not part of this document.
- **The sync contract and the event stream (RFC-003, RFC-007): nothing, for
  now.** Filing is not a user-data change the desktop mirrors, and RFC-007's
  event kinds are a contract the desktop reads. A desktop that wants the filed
  tree would ask for its own event kind, in its own document.
- **The scanner: nothing.** It neither reads nor writes placements or uploaded
  lyrics.
- **Migrations**: two new dated files — `library_folder` and
  `track_placement`, and `track_lyrics_upload`. None edits an applied one.

## The order of the work

Each step is its own pull request, shippable and useful without the next:

1. **This RFC.**
2. **The right click and Properties**, with `relative_path` on the track. The
   menu holds only what already exists: properties, edit tags, queue.
3. **Download**, `?download=1` on the stream ticket, and its menu entry.
4. **Uploaded lyrics**: the table, the TTML parser, the three routes, and the
   lyrics section of Properties.
5. **The disk tree** and the browser page, read-only, with upload from there
   through RFC-008.
6. **The filed tree**: the tables, the routes, drag and drop, "Move to
   folder…", and dropping uploads onto a folder.

## What is still open

- Whether the filed tree should become the Subsonic folder view for clients
  that browse by folder. It cannot under the frozen contract; it could in a
  later contract version.
