# RFC-021 — Libraries and scanning: exclusions, links, missing tracks, the library lifecycle and watching

- **Status**: Proposed
- **Implemented by**: in progress (this RFC and its implementation, in one
  pull request). The *Status* field above never flips in this project; this
  line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the scanner, track
  identity, tenancy), [RFC-018](RFC-018-incremental-scan.md) (the incremental
  scan and its tolerant walk, whose guarantees all hold here)

## Problem

A comparative audit of 2026-10-24 found five gaps around libraries and how they
are scanned. Each was confirmed in the code.

1. **The walk excludes nothing.** `discover_audio` (`src/scanner.rs`) indexes
   everything with an audio extension, so Synology's `@eaDir` thumbnails, the
   `._` AppleDouble files a Mac leaves on a share, `.Trash-1000`,
   `$RECYCLE.BIN` and every hidden folder are read as music. There is no way
   for an operator to say "not this folder" (Navidrome has `.ndignore`).
2. **Symbolic links are dropped in silence.** The walk does not follow links
   and does not say it skipped one, so a library built of links appears empty
   and nothing explains why.
3. **A track whose file is gone is kept for ever and shown nowhere.** It goes
   to `is_available = 0` and stays, holding its favourites, playlist entries
   and plays, with no page listing it and no way to remove it.
4. **A library can be added and never changed.** Nothing renames one, moves
   its root or deletes it: `src/` holds no `UPDATE library` touching the name
   or the path and no `DELETE FROM library`, and the CLI has only `add`.
5. **Changes are seen at the next interval only.** The only trigger besides a
   manual one is the periodic scan, six hours by default
   (`WAVEFLOW_SCAN_INTERVAL_SECS`).

## Decision 1 — what the walk leaves out

**Built-in exclusions**, applied to every entry below the library root (never
to the root itself or to the folders above it), by name:

- any name beginning with `.` — hidden files and folders, which covers
  `.Trash-*`, `.AppleDouble`, `._*` and `.DS_Store` — **except
  `.waveflow-managed` directly under the root**, the folder where received files
  land (RFC-008), which the scan must keep finding;
- `@eaDir`, `#recycle`, `#snapshot` (Synology), `$RECYCLE.BIN`,
  `System Volume Information` (Windows) and `lost+found`, compared without
  regard to case.

Built-in exclusions cannot be overridden: a dot-folder of music is rare, and a
rule that a file can switch off is a rule every library has to check.

**An ignore file**, `.waveflowignore`, in any folder of the library. Its syntax
is `.gitignore`'s, read by the `ignore` crate's matcher, so the rules are the
ones every operator already knows: one pattern per line, `#` comments, a
leading `/` anchors to the file's folder, a trailing `/` matches folders only,
`**` crosses folders, and `!` re-includes. A file applies to its folder and
everything below it, and a deeper file's rules win over a shallower one's, as in
git. A folder that is excluded is not entered, so `!` cannot re-include
something under it, as in git.

An **empty** `.waveflowignore` (nothing but white space) excludes the folder it
sits in and everything below it, as an empty `.ndignore` does in Navidrome.
That rule does not apply at the library root, where an empty file excludes
nothing: one stray empty file must not mark a whole library missing.

The rules are matched against the path the walk sees, so a link (decision 2)
is excluded or not by its own name, whatever its target is called.

**An ignore file that cannot be used**, because it cannot be read, is larger
than 64 KiB, is not a regular file, or holds a pattern the matcher rejects,
makes its folder count as **unreadable**, exactly as RFC-018 decision 3 defines
the word: it is not entered, it is reported in the scan's errors with its path
relative to the library, and the tracks under it are left as they are. The
scan cannot know what the operator meant to exclude, so it neither indexes the
folder nor marks it missing. At the library root the same failure **fails the
scan**, since then nothing at all can be told apart; that is the RFC-018 rule
for a root that cannot be opened.

## Decision 2 — an excluded file is a file the walk did not find

**An excluded file counts as not discovered**, and that is the whole of its
treatment. It is *not* one of the files RFC-018 decision 3 protects, because
the walk did look: it saw the file and was told to leave it out. So:

- a track already in the catalogue whose file is now excluded **becomes
  unavailable at the end of the scan, exactly like a deleted one**, with the
  `delete` event the library feed already sends for that;
- it may be the source of a relocation like a deleted one: a file elsewhere
  with the same content (or, failing that, the same relocation hint) takes its
  identity, favourites and playlist entries included;
- removing the rule makes it available again on the next scan, under the same
  identifier, as a file that comes back does today.

Only an unreadable folder (decision 1) and an unreadable entry (RFC-018) keep
their tracks as they are. The scan job counts what was left out in a new
column, `excluded`: one per excluded entry, a folder counting once whatever it
holds.

## Decision 3 — symbolic links, followed inside the library only

**Following links is a per-library setting, `follow_links`, off by default**,
set by an administrator (API, CLI or the web client). Per library rather than
global because a link farm is a property of one tree, and because a server
holding several libraries should not change what all of them read for the sake
of one.

**With it off**, the walk does not follow any link, as today, but it no longer
does so silently: each link it passes is **counted** in the scan job's new
`skipped_links` column and **logged** (at `info`, by its path relative to the
library, the first twenty of a scan one by one and the rest as a count).

**With it on**, a link is followed only when its canonical target — every link
on the way resolved — **stays inside the library root**. Anything else is
skipped, counted and logged in the same way:

- a link whose target lies outside the root;
- a broken link;
- a loop (a link to one of its own ancestors), which `walkdir` detects.

The rule exists because a link is a way for anyone who can write into the
library folder to make the server read and serve any file the server can read.
Confinement keeps the library the boundary it is everywhere else in this
server (streaming already refuses any path that leaves the root).

**A file reached through a link is indexed under its canonical path**, the path
of its target relative to the root, not under the link's name. Three things
follow, all deliberate:

- **one file is one track** however many links name it, and a file reached both
  directly and through a link is not indexed twice;
- **the stored path never contains a link**, so the streaming path check
  (`secure_track_path`, which refuses any link component) is unchanged;
- **renaming a link moves nothing**: identity follows the file.

The cost is that the disk tree (RFC-012) shows the targets' layout, not the
links'. The use this serves is a link farm whose files live inside the library,
for example in a store folder the walk does not enter directly:

```text
/music                     ← library root, follow_links on
/music/.waveflowignore     ← "/store/"
/music/store/…             ← the files, excluded from the direct walk
/music/Artist/Album/01.flac → ../../store/ab/cd.flac
```

A link farm whose files live **outside** the root (a beets `link: yes` import
into a separate folder, say) is not followed. The operator either makes the
root a common ancestor of both, as above, or adds the files' folder as a
library of its own.

## Decision 4 — missing tracks: a page, a purge, and an optional retention

**`unavailable_since`**, a new nullable column on `track`, records when a track
stopped being available. It is set when a scan marks a track missing (excluded
or deleted), cleared when one comes back, and filled from `updated_at` for the
tracks already unavailable when the migration runs.

**A view**, for the library's owners and managers and for administrators:
`GET /api/v2/libraries/{id}/unavailable-tracks` lists the unavailable tracks,
oldest first, each with its path, title, artist, album, when it went missing,
and **what purging it would cost**: how many playlist entries, favourites and
recorded plays hold it. The web client shows it on its own page, "Missing
files", beside "Needs attention".

**A purge**, explicit: `POST /api/v2/libraries/{id}/unavailable-tracks/purge`
with either `{"track_ids": […]}` or `{"all": true}`, and never neither. Only
tracks that are **still** unavailable when the purge runs are removed: one that
came back between the listing and the click is kept. It is also offered by the
CLI (`library unavailable`, `library purge`).

**What a purge loses**, stated here because none of it comes back:

- the track rows, and everything that cascades from them: playlist entries,
  favourites and ratings of the track, play history, queue entries, shares'
  entries, bookmarks, lyrics (embedded, sidecar and uploaded), corrections
  (`track_override`), credits, genre links, folder placements, canvas links and
  quality measurements;
- the albums and artists that hold no track any more, with their favourites
  and ratings; a pass of the scan's own consolidation runs in the same
  transaction, so the catalogue is never left with an empty album;
- nothing on disk but what the server made: audio files are never touched,
  covers nothing references any more are left to the existing artwork sweep,
  and a canvas (RFC-009) whose last link went with these tracks is erased after
  the commit, exactly as removing that canvas would erase it.

A file that comes back after its track was purged is a new track with a new
identifier.

The purge takes the writer gate and runs in **one transaction**: tracks,
favourites, search index rows and the consolidation land or fail together. It
**refuses with 409 while the library is being scanned** (it takes the
library's scan lock without waiting), because a running scan may be about to
relocate one of those tracks onto a file it just found. It is audited.

The library feed is not written again: it already said `delete` for each of
these tracks when they went missing. The user sync journal is not written
either; a device holding a favourite or a playlist entry of a track that no
longer exists drops it with the track (see "What is still open").

**Retention, off by default**: `WAVEFLOW_UNAVAILABLE_RETENTION_DAYS` (unset or
`0`: never). When set, a background pass once an hour purges, through the same
code, the tracks unavailable for longer than that, in every library that is not
being scanned. Off by default because the purge is not reversible and a share
that stays unmounted for a week is not a deletion.

## Decision 5 — the library lifecycle

Three operations, each in the native API, the CLI and the web client, all
through one implementation in `src/services/`.

| Operation | Who | API |
| --- | --- | --- |
| Rename | the library's owner, or an administrator | `PATCH /api/v2/libraries/{id}` `{"name": …}` |
| Change the path | an administrator | `PATCH /api/v2/libraries/{id}` `{"path": …}` |
| Follow links on or off | an administrator | `PATCH /api/v2/libraries/{id}` `{"follow_links": …}` |
| Delete | an administrator | `DELETE /api/v2/libraries/{id}` `{"confirm": "<name>"}` |

The path and the links setting are an administrator's for the reason creating a
library is: they decide which part of the server's file system the server
reads. A `PATCH` naming either needs an administrative credential, not only an
administrator's account; a rename needs a credential that may write.

**Administrative standing comes from the credential as well as the account**,
on every operation here, the missing-tracks view and purge included. An
administrator whose token's scopes stop short of `admin` holds only the
memberships of the account behind it: such a token cannot rename, purge or
list the missing tracks of a library that account is not a member of, and the
deletion refuses it outright. That is the rule `AuthUser::scopes` states for
every narrowed credential.

**Changing the path** validates the new one as creating a library does (an
existing directory, not itself a link, stored canonical, not another library's
root), then queues a scan with the new trigger `library_moved`. Tracks are
matched by their path relative to the root, so **a root moved with its tree
intact keeps every identifier**, favourite and playlist entry. What the new
root does not hold is marked missing, and relocation by content still applies
to what moved within it. Changing `follow_links` queues a scan too.

**Deleting** removes the library from the catalogue and nothing from the disk.
Its cascade, all in one transaction under the writer gate:

- every track, with everything decision 4 lists for a purge — for **every
  member**, not only the one deleting;
- every album, artist and genre of the library, their favourites and ratings,
  and their search index rows;
- the memberships, the scan history and its errors, the library feed and its
  acknowledgements, folders (RFC-012), credit decisions, upload sessions;
- audited as `library.deleted`.

Received files stay where they are: the server wrote them, but they are the
library's files now. The staging files of upload sessions still open are
removed, after the commit, and so are the canvases no other library still links
to; covers are left to the artwork sweep. A client polling the library's feed
gets 404 from then on, which is its signal to drop the library.

**Confirmation is in the request's shape.** The body must carry `confirm`, equal
to the library's current name; anything else is 422 and deletes nothing. The
web client asks for the name to be typed; the CLI takes `--confirm <name>`.

All three refuse with 409 while the library is being scanned, for the reason
the purge does. Every refusal of an operation the caller may not perform on a
library they cannot see is a 404, as everywhere else.

## Decision 6 — watching the file system

**A watcher**, on the `notify` crate, observes every library root recursively
and triggers an **incremental scan of the affected library once its changes
have settled**: no event for `WAVEFLOW_WATCH_DELAY_SECS` (10 by default). A
stream of changes that never settles, a long copy for instance, still triggers
a scan once thirty delays have passed since its first event, and the next quiet
moment triggers another. The debouncing is the server's own, per library, on
top of the raw events: one crate rather than two, and a rule this RFC can state.

- **The periodic scan stays**, as the safety net for everything a watcher can
  miss.
- **`WAVEFLOW_WATCH`** (`true` by default) turns it off.
- **It never starts a scan while one runs for that library.** A library whose
  changes settle during a scan stays pending and is scanned once that one ends,
  so a change the running scan walked past is not lost. Scans keep everything
  else they had: the per-library lock, the writer gate, concurrency between
  libraries and never between writers.
- **Events that cannot be changes to the music are ignored**: reads and access
  times (the scanner reads files, and would otherwise trigger itself for ever),
  and anything under `.waveflow-managed`, which an upload writes and applies
  itself.
- **A queue overflow** (the kernel dropping events) marks every library pending.
- **The watched set follows the libraries**: it is refreshed when a library is
  added, moved or deleted through the server, and every five minutes for the
  changes the CLI makes from another process.

**A watch that cannot be established is not an error.** If the watcher cannot
start, or a root cannot be watched (a mount that does not support it, the
inotify limit `fs.inotify.max_user_watches` reached), that is logged once with
the library's identifier and the library is left to the periodic scan. Network
mounts are the known case: NFS and SMB report changes made by this machine and
usually not those made by another, so a library on a share should not count on
the watcher. `docs/operations.md` says so.

## Decision 7 — what the scan job says

The scan job gains two counters, `excluded` and `skipped_links` (decisions 2
and 3), in the table, `GET /api/v2/scans/{id}` and both frames of the progress
stream, and two triggers, `watch` and `library_moved`. SQLite cannot alter the
`CHECK` on `scan_job.trigger`, so the table is rebuilt; migrations run with
foreign keys off, so the rows that reference it (`scan_error`,
`track.last_seen_scan_id`) are carried and checked by the boot's
`PRAGMA foreign_key_check`.

## What does not change

- Track identity, relocation, the incremental pre-check, ordered streaming and
  RFC-018's rules for what the walk could not see.
- Audio files are read-only; nothing here writes to a library.
- Creating a library, its membership rules, and the Subsonic façade, whose
  frozen contract has no library management.

## The order of the work

1. **One PR**, holding this RFC and:
   - the migration (`library.follow_links`, `track.unavailable_since`, the
     rebuilt `scan_job`);
   - the exclusions, `.waveflowignore` and link handling in the walk;
   - the missing-tracks listing and purge, and the retention pass;
   - rename, path change, links setting and deletion, in the services, the
     API, the CLI and the web client;
   - the watcher and its two settings;
   - tests for each rule, among them: an excluded track becomes unavailable
     and an unreadable ignore file keeps its tracks; a link outside the root is
     skipped and counted; a purge removes only what is still unavailable and
     leaves no empty album; a moved root keeps identities; a deletion leaves no
     row behind; a watched change triggers a scan, and none starts while one
     runs;
   - `docs/operations.md`.

## What is still open

- **The user sync journal on a purge or a deletion.** A favourite or a
  playlist entry removed by the cascade is not written to the journal of the
  account that held it, so a device keeps its copy until that playlist or
  favourite next changes. The library feed's `delete` for the track is the
  signal a client has today.
- **Following links outside the root**, to an allowlist of further roots, for
  link farms that live elsewhere.
- **Undoing a purge**, for instance by keeping purged rows for a grace period.
- **A per-library watcher switch**, for a library on a share where watching
  costs and finds nothing.
- **Folder-level skipping** (RFC-018) and watching would combine well: a
  watcher that names the changed folder could let a scan walk only that
  folder. Not done; every watched change is a full incremental walk.
