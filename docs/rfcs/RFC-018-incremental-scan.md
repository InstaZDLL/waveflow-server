# RFC-018 — An incremental scan that reads only what changed, and survives one bad folder

- **Status**: Proposed
- **Implemented by**: in progress: [#322](https://github.com/InstaZDLL/waveflow-server/pull/322)
  (this RFC) and [#324](https://github.com/InstaZDLL/waveflow-server/pull/324)
  (the pre-check and its sidecar signature, streaming, tolerant discovery,
  `database rescan-full`). The *Status* field above never flips in this
  project; this line is the one that says what is running.
- **Date**: 2026-10-24
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the scanner and
  track identity)

## Problem

The audits of 2026-10-24, run against Navidrome, found three problems in
`src/scanner.rs`. Each was confirmed in the code.

1. **Every scan reads every byte of the library.** `extract_file` computes the
   quick hash, then the full BLAKE3 hash of the whole file, then reads its tags
   and extracts its cover. All of that happens *before* the scan compares size
   and modification time to decide that the file is unchanged and can be
   skipped. An "incremental" scan therefore costs as much I/O as a full one.
   For 2 TB of FLAC on a disk reading 150 MB/s, that is about four hours of
   reading, every six hours by default (`WAVEFLOW_SCAN_INTERVAL_SECS`).
   Navidrome skips unchanged folders, then compares file modification times,
   before it reads anything.
2. **Every result is held in memory before anything is written.** The
   extractions are collected into one `Vec`
   (`.buffer_unordered(..).collect()`), sorted, and only then applied in
   batches of 25. Memory grows with the library, and a scan that dies late
   loses all of its work.
3. **One unreadable folder fails the whole scan.** `discover_audio` walks the
   tree and propagates the first error (`let entry = entry?;`). A single
   folder the server cannot read, such as a Synology `@eaDir` with the wrong
   owner or a mount that has gone away, stops discovery for the entire
   library.

## Decision 1 — look before reading

For each discovered file, the scan first asks the database what it already
knows (size, modification time, availability), and the file system for the
same two values plus the sidecars' signature (below). This costs one `stat`
per file and one per possible sidecar, and no reads.

A file is **unchanged**, and skipped without being opened, when all of these
hold:

- its size and modification time equal the stored ones;
- the track is available;
- its sidecar signature equals the stored one;
- this is not a full scan (`waveflow-server database rescan-full`, or the identity-spec
  change that forces one);
- the track already has the identity hint the skip path writes today, so a row
  from before that column still gets its one extraction.

**Anything else reads the file as today**: the quick and full hashes, the tags,
the cover and the lyrics. The match by path, then by content hash, then by
relocation hint, is unchanged.

**The sidecar signature.** The scan already reads lyrics from `<name>.lrc` and
`<name>.txt` beside the audio file. A sidecar added, edited or removed beside
an unchanged audio file must still be picked up. So a new nullable column,
`track.sidecar_signature`, records the size and modification time of each of
the two possible sidecars, absent ones included. It is computed from `stat`
alone, and it is a string that is never `NULL` once written:

```text
lrc:<size>:<mtime_ms>|txt:<size>:<mtime_ms>
```

An absent sidecar writes `-` in place of its size and modification time:
`lrc:-|txt:-` is a file with no sidecar at all. Absence is therefore a value
like any other, and adding, editing or deleting either sidecar changes the
string. `NULL` keeps one meaning only: never computed. Every row right after
the migration is `NULL`, and `NULL` never matches, so the first scan after an
upgrade reads every file once and fills the column; the scans after it skip,
sidecar or not.

**What the trade costs.** A file rewritten in place with its size and
modification time both preserved is missed until the next full scan. Some tag
editors preserve the modification time; none can preserve both while changing
the tags, except by padding to the same size. This is the trade Navidrome and
every incremental scanner make. The full scan stays available for anyone who
suspects it, and `docs/guides/operations.md` says so.

## Decision 2 — stream, in path order

Extraction keeps its parallelism, but results are applied **as they arrive**, in
batches of 25. They are still applied in path order, so the outcome stays
deterministic: paths are sorted once, after discovery, and extracted through an
ordered buffer (`buffered`, not `buffer_unordered`). Memory therefore holds at
most a few batches, not the library. Each batch commits on its own, so a scan
that dies keeps what it wrote, and the next run resumes on a library mostly
already up to date.

Progress (`processed`, `skipped`, …) moves as batches commit, which is also what
the scan's progress stream shows.

## Decision 3 — one bad folder costs that folder

`discover_audio` no longer stops at the first error. An entry it cannot read is
**logged** with its path relative to the library root, never the absolute
path. It is **counted** in the scan's `errors`, and the scan **goes on**.

**What sat under an unreadable folder is not marked missing.** The tracks
whose path lies under a directory that could not be read are left exactly as
they are. They are neither seen nor marked unavailable, so a folder that is
briefly unreadable (a mount coming back, a permission being fixed) does not
make its albums vanish and come back. Only the library root failing to open
still fails the scan, since then nothing can be told apart.

## What does not change

- Track identity, relocation and the derived album and artist ids.
- The full scan, the boot and scheduled scans, and their triggers.
- Symbolic links are still not followed (see below).

## The order of the work

1. **This RFC.**
2. **One PR**:
   - the pre-check and its sidecar signature, with their migration;
   - ordered streaming of the results;
   - tolerant discovery;
   - tests for each rule: a skipped file is never opened, a sidecar change is
     caught, a file under an unreadable folder is not marked missing, memory
     stays bounded, the scan resumes after a failure;
   - `docs/guides/operations.md`.

## What is still open

- **Folder-level skipping**, as Navidrome does: a directory whose modification
  time has not moved need not even be listed. It is a further gain on large
  libraries, but directory modification times are not reliable on every file
  system and network share, so it waits for a measurement.
- **Watching the file system** (`notify`), so that a change is seen in seconds
  rather than at the next interval. The periodic scan would stay as the safety
  net.
- **Ignore rules**: `@eaDir`, `.Trash-*`, `$RECYCLE.BIN`, hidden folders, and
  an `.ndignore`-style file.
- **Following symbolic links**, confined to the library root after
  canonicalisation, for libraries built as link farms (beets).
