# RFC-029 — A track's music video

- **Status**: Proposed
- **Implemented by**: [#449](https://github.com/InstaZDLL/waveflow-server/pull/449)
  (the core pin), [#450](https://github.com/InstaZDLL/waveflow-server/pull/450)
  (decision 4, with decision 3's `Clips` scope),
  [#451](https://github.com/InstaZDLL/waveflow-server/pull/451) (decisions 2
  and 3, the link the scan records),
  [#452](https://github.com/InstaZDLL/waveflow-server/pull/452) (decisions 5
  to 8, the routes, the ticket and the `clip` field),
  [#453](https://github.com/InstaZDLL/waveflow-server/pull/453) (the web
  client, on the playing screen's detail view); the desktop's side (decision
  9) is its own repository's. The *Status* field above never flips in this project; this line is the one that
  says what is running.
- **Date**: 2026-11-14
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md),
  [RFC-007](RFC-007-library-event-stream.md) (a link travels in the track's
  `upsert`), [RFC-009](RFC-009-track-canvas.md) (the ticket discriminator, the
  shape of the canvas routes), [RFC-018](RFC-018-incremental-scan.md) (what an
  incremental scan reads), [RFC-021](RFC-021-library-lifecycle-and-scan.md)
  (exclusions, links, scoped scans)
- **Requires**: `waveflow-core` v2.1.0, which carries
  `waveflow_core::scanner::clips`. The server pins v2.0.0 today; moving the pin
  is its own pull request and is not part of this RFC.

## Problem

The desktop app plays a track's music video (WaveFlow discussion #1073): the
video file named like the track, beside it or in a `Clips` folder beside it.
The clip plays in the now-playing panel or the immersive view with its own
sound and its own length; the track pauses while it plays and resumes at the
clip's position after it. Nothing is stored: the disk is asked at each track
change.

For a track that comes from a WaveFlow Server, the desktop offers no clip. It
cannot: the server has no notion of one, and the desktop's lookup reads the
local disk, which a server track does not have. The rule the desktop follows
lives in `waveflow-core`, so the server can follow the same rule — but it has
to decide how it records what it finds, how it serves a file that is often
hundreds of megabytes, and who may see it.

There is also a defect today, independent of any feature. `mp4` is an audio
extension (`waveflow_core::scanner::AUDIO_EXTENSIONS`), legitimately: an `.mp4`
often holds nothing but AAC. So a music video dropped beside its song as
`Song.mp4` is indexed by this server as **a second track**, whose audio is the
clip's soundtrack. The desktop fixed the same defect in its walk
(`drop_clips`); the server has not.

## What the desktop does

Read from `InstaZDLL/WaveFlow` at `7b101640`, so that the decisions below can
be checked against it.

- **The rule** — `src-tauri/crates/core/src/scanner/clips.rs`, `find_clip`:
  1. a video in the track's folder with the track's file name, extension
     aside, case ignored (`ClipOrigin::Sibling`);
  2. else the same name in a folder called `Clips` (case ignored, not a link)
     beside the track (`ClipOrigin::ClipsDir`);
  3. else the track's own file, when it is an ISO-BMFF file with a moving
     picture (`ClipOrigin::Itself`) — a library made of music videos alone.

  A track inside a `Clips` folder is never offered its neighbours. Several
  candidates are taken in `VIDEO_EXTENSIONS` order: `mp4`, `m4v`, `mov`,
  `webm`, `mkv`.
- **The content is checked, not the name.** An `mp4`, `m4v` or `mov` must hold
  a `vide` track whose first sample entry is a video codec (`has_video_track`,
  a bounded walk of the box headers, so a chapter still in an audiobook is no
  clip); a `webm` or `mkv` must start with the EBML magic. Links are refused:
  what is found enters the webview's asset scope.

  The check is weaker for the second pair, and that is a known limit of the
  rule: the EBML magic says the file is Matroska, not that it holds a picture,
  so a WebM carrying audio alone is taken for a clip. The server keeps that
  behaviour for parity (decision 2) rather than correcting it on its own side;
  a fix belongs in the core, where both sides receive it together.
- **The walk** drops an `.mp4` that the rule makes a clip and whose content
  `has_video_track` confirms (`clip_candidates`, `drop_clips`). An `.mp4` with
  no video stays a track whatever its neighbours; a video `.mp4` with no song
  of its name stays a track.
- **Nothing is stored.** No table, no migration: `get_track_clip` looks at the
  disk each time the current track changes, and answers `None` for a track on a
  share or from a server.
- **Playback** — `ClipHost`, `ClipVideo`, `clipPlayer`: one `<video>` per
  opening, moved between surfaces rather than reloaded; media keys reach the
  clip while it is open (`player_set_clip_open`). On Linux a local clip goes
  through the loopback server, because WebKitGTK cannot play the asset
  protocol; a remote URL is handed to the element as it is — which is how the
  server's canvas already plays (`remote/canvas.rs`).

## Decision 1 — a clip lives in the library, beside its track

A clip is a file the operator put in their collection, like the audio beside
it. The server finds it there, and only there.

**Not received.** RFC-009 gave the canvas a server-owned store because a human
supplies it and no scan can find it. A music video is the opposite case — the
desktop's rule finds it on the disk — and it is two to three orders of
magnitude larger than a canvas: a quota, a resumable transfer and a store would
be RFC-008's whole machinery, for a file the operator can copy into the folder
themselves. Receiving a clip is left out of this RFC, not refused for ever: it
needs its own answer to RFC-008's question — spending somebody else's disk —
at a scale where the answer matters, and nobody has asked for it yet.

**The disk stays the authority.** There is no route that places or removes a
clip. Deleting the file removes it; renaming it unlinks it. The audio files are
read-only to this server, and so are the clips: nothing is remuxed, defragmented
or rewritten (decision 6).

**An exclusion applies to a clip as to a track.** A clip under an ignored
folder, a dot folder or a built-in exclusion (RFC-021 decision 1) is not seen,
so it is not found. That is the operator's way to keep clips off the server
without a setting: put them where the walk does not go.

## Decision 2 — the desktop's rule, read from the core, held to it by a test

Two implementations of "which video is this track's clip" would disagree on the
first edge case, and a client showing a clip for a local track and none for the
same track from the server is the bug that results. The rule is therefore the
core's, from `waveflow_core::scanner::clips` (v2.1.0), with the same three
origins, the same extension order, the same content checks and the same refusal
to offer a `Clips` folder's neighbours to a track inside it.

**One deliberate difference: links.** The desktop refuses a linked clip because
whatever it finds enters the webview's asset scope. The server has no asset
scope; it has `secure_track_path`, which confines every byte it serves to the
library under the library's link policy (RFC-021 decision 3). A clip follows
that policy exactly as the audio beside it does: a library that follows links
finds a linked clip, re-checked on every request; one that does not, does not.

**The scan does not call `find_clip` per track.** `find_clip` lists the folder
for every track it is asked about — twelve listings of one album folder, and
twelve more of its `Clips` folder. The walk has already listed every folder
once, so the server matches names in memory over what the walk saw, and asks
the disk only to confirm the content of a candidate. The matcher is the
server's own code and the place two rules could drift apart, so it is **held
to `find_clip` by a test**, the way RFC-016 holds the quality analysis to the
desktop's: a fixture tree covering the three origins, the case folding, the
extension order, an audio `.mp4`, a chapter still, a `Clips` folder with a
track in it, an `.mp4` with no song, a `Song.mp4` and a `Song.webm` with no
audio file of that name (decision 4), and a name whose first candidate in
extension order fails the content check while the next one passes — and for
every track in it, the server's answer equals `find_clip`'s. A core bump that changes the rule fails
that test before it changes a catalogue.

If the core later exposes a matcher over a listed folder, the server's copy is
replaced by it and the test stays.

## Decision 3 — the scan records the link, in a table of its own

The desktop stores nothing because it asks its own disk, cheaply, once per track
change. The server cannot afford that answer, for two reasons:

- **A client must know which tracks have a clip without asking for each.** The
  desktop's context menu and the web client's song table would otherwise make
  one request per row, each a directory listing on what may be a spun-down
  drive.
- **Every other fact about the disk is recorded by the scan.** The catalogue is
  already a scan-derived picture of the library, and a clip is one more fact
  about a folder. Recording it is what makes it a column a query can join, with
  tenancy held in that query like everything else.

```sql
CREATE TABLE track_clip (
    track_id      TEXT PRIMARY KEY NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    library_id    TEXT NOT NULL REFERENCES library(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    format        TEXT NOT NULL CHECK (format IN ('mp4','m4v','mov','webm','mkv')),
    origin        TEXT NOT NULL CHECK (origin IN ('sibling','clips_dir','itself')),
    byte_size     INTEGER NOT NULL,
    modified_at   INTEGER NOT NULL
);
CREATE INDEX track_clip_library ON track_clip (library_id);
```

**Not a column on `track`.** RFC-009 decision 2 kept the canvas off `track` so
that `apply_catalog_track_in_transaction`'s `ON CONFLICT DO UPDATE SET` list
could never erase it. Here the scan owns the link, so that argument is turned
around rather than repeated: an incremental scan skips an unchanged audio file
without writing its row (RFC-018 decision 1), and a clip added beside it must
still be recorded. The link therefore has to be written by a pass of its own,
which is a table's write, not a column the track's upsert carries.

**`library_id` is the track's**, read in the same transaction, never computed
from the clip's path — the rule `track_canvas` follows.

**The pass, per scanned scope.** After the walk, and whether or not any audio
file was read, the scan computes the link of every track in the scope from the
names the walk saw, and replaces the rows of those tracks: inserted, updated or
deleted, under the writer gate, in one transaction per batch as the catalogue
writes are. A track whose link did not change is not written.

**A link changes only on what the walk could see.** The matcher's answer for a
track is the first confirmed candidate in the rule's order: the track's folder,
then its `Clips` folder, then the track's own file. That answer says something
only about the places the walk actually looked. Its blind spots are the ones
`Walked::saw_missing` already names for a track's file (RFC-018 decision 3). A
place is **unseen** when:

- the walk is `incomplete`;
- the place is under a folder the walk could not read (an unreadable `Clips/`
  folder is not an empty one);
- the place is outside the `scope` of a scoped scan, which did not look there.

A candidate whose content check could not *read* the file counts as unseen
too (below).

The pass trusts the matcher's answer only when every place the rule visits
**before** that answer was seen. When the answer is no clip at all, that means
every place. Otherwise the existing link is kept exactly as it is, neither
deleted nor replaced. Two consequences:

- **No fallback past a blind spot.** With `Clips/` unreadable, a track whose own
  file is a video would otherwise fall to `itself` and lose the `clips_dir`
  link it had. The desktop's `find_clip` does fall back there, since a folder
  it cannot list yields nothing. The server does not, because a folder it could
  not read is not a folder without a clip. This is the one place the two
  answers may differ, and only on a disk the server cannot read. The parity
  test (decision 2) runs on a readable tree.
- **A better candidate the walk did see still wins.** A video dropped beside
  the track replaces a link into an unreadable `Clips/`. Nothing comes before
  the track's own folder in the rule's order, so the answer is trusted.

When the answer is trusted, the link follows it, and it goes in two cases:

- **The clip's path is no longer discovered.** That includes a clip an
  exclusion now hides: as far as the server is concerned it is gone, which is
  decision 1's rule and not an accident of it.
- **The file is there and the content check rejects it.** For example, an
  `.mp4` re-encoded without its picture, or another file renamed onto the
  clip's name.

A rejection is an answer about the content, and **a read that failed is not**.
An unreadable file keeps the link, as an unreadable folder does. The two are
told apart by calling `has_video_track`, which returns `io::Result<bool>`,
rather than `file_has_video_track`, which folds an error into `false`. The
EBML check is read the same way.

**The content check is not repeated for nothing.** A candidate whose
`relative_path`, `byte_size` and `modified_at` equal the row's is taken as
confirmed without opening it, the trade RFC-018 already makes for audio; a full
scan reads it again.

**The walk lists video files**, without indexing them: the matcher needs them,
and a `.webm` or `.mkv` is not an audio extension and is never seen today. The
list is `waveflow_core::scanner::clips::VIDEO_EXTENSIONS`, not a copy of it.

**Two sets, never merged.** The walk's result holds them apart:

- **the indexable audio files** — what `indexes_extension` accepts. Only this
  set goes to `drop_clips` and `clip_candidates` (decision 4), to extraction,
  to the scan's counters, to the detection of vanished files and to
  relocation;
- **the video candidates** — every file whose extension is in
  `VIDEO_EXTENSIONS`, an `.mp4` included whichever set it is also in. Only the
  clip matcher reads this set.

The reason is in the core's contract, not in tidiness: `clip_candidates`
assumes its input is the audio list. Given `Song.mp4` and `Song.webm` with no
audio file of that name, a list that held the `.webm` would make the `.webm`
the `.mp4`'s song, and drop the `.mp4` — which the desktop keeps as a track,
because its walk never lists a `.webm`.

**A change in a `Clips` folder rescans its parent.** The watcher scopes a scan
to the folder a change happened in (RFC-021); a clip dropped into
`Album/Clips/` would otherwise scan a folder holding no track and link nothing.
A scope whose last component is a `Clips` folder is widened to its parent.

## Decision 4 — a video `.mp4` beside its song stops being a track

The walk drops the `.mp4` files the rule makes clips, with `drop_clips` from the
core and the content confirmed by `has_video_track`, as the desktop's walk does
— over the indexable audio files alone, never over the video candidates
(decision 3, "two sets").
What stays a track is what the desktop keeps: an `.mp4` with no video in it,
whatever its neighbours, and a video `.mp4` with no song of its name — whose
clip is then itself (`origin = 'itself'`).

**This changes existing catalogues.** On a server that indexed such a file, the
next scan finds it gone from the walk and handles it as any vanished file:
unavailable, then purged under RFC-021's rules — and whatever a member had on
that "track" (a favourite, a rating, plays, a playlist entry) goes with it. The
release notes say so, in those words. It was never a track; it was the clip of
one, played without its picture.

## Decision 5 — three routes, and a fourth ticket kind

| Route | For whom |
| --- | --- |
| `GET /api/v2/tracks/{id}/clip` | Bearer; the clip the track has now, with ranges |
| `POST /api/v2/tracks/{id}/clip-ticket` | Bearer; mints a ticket, answers a **relative** URL |
| `GET /api/v2/clip-stream/{ticket}` | what `<video src>` puts in its attribute |

This is the canvas's set minus what does not apply:

- **No route by hash.** A clip is not content-addressed: hashing a 300 MB file
  at every scan would be the most expensive thing the scan does, to obtain an
  address nobody needs, since a clip is reached only through its track.
- **No `PUT` and no `DELETE`.** Decision 1: the disk is the authority.

**The ticket is `TicketKind::Clip`, byte `0x04`.** RFC-009 decision 3 set out
why the kind is sealed with the rest, and every reason holds with more force:
a clip ticket that opened the audio, or an audio ticket that opened a clip, is
the privilege nobody granted, and here the bandwidth difference runs the other
way. `0x04` is the next free value after `0x03` (`ShareVisit`, RFC-020); `0x00`
stays no kind. The payload's shape does not change — `kind || user || track ||
expiry` — so unlike RFC-009, **no ticket minted before the upgrade stops
validating**. The test vectors gain a round trip for the new kind, and the
cross-kind refusals in both directions for every route that redeems one.

**The TTL is the audio's** (`stream_ticket_ttl`, an hour), for the audio's
reason: the browser reuses that URL for every range request while seeking, and
a clip is seeked into far more than a canvas.

**The answer to the mint is relative** — `/api/v2/clip-stream/<ticket>` — for
the reason `StreamTicketResponse` states and the desktop's `remote/canvas.rs`
already enforces: a client prepends the base URL it authenticated against, and
refuses an absolute or protocol-relative value.

`clip-stream` and not `clip/{ticket}`, as `canvas-stream` and for the same
reason: a literal segment under the prefix of a parameter is a trap for the
next reader. The router's redaction table gains `/api/v2/clip-stream/`, so a
ticket is never written to a trace.

## Decision 6 — the bytes as they are, ranges included

**Served whole or by range, never transcoded.** `serve_opened` already answers
a single `Range` (with #446's bound to the end of the span), and a video player
seeks by ranges. No FFmpeg is involved: RFC-009 decision 8 refused to make a
canvas a compute load per playback, and a clip would be a far heavier one.

**Not remuxed, not defragmented.** WebKitGTK stops a *fragmented* MP4 a couple
of seconds in when it plays it over HTTP; the desktop rewrites such a file in
its own loopback server. The server does not: the operator's files are
read-only to it, and a rewritten copy would be a cache the server keeps on
somebody else's disk. Decision 9 says where that leaves the desktop on Linux.

**The format is not judged.** The server serves a `mkv` that a given browser may
refuse; whether a client can play a container is the client's to report when
the clip opens, as the desktop already does locally. The MIME type follows the
recorded format: `video/mp4` (`mp4`, `m4v`), `video/quicktime` (`mov`),
`video/webm`, `video/x-matroska`.

**Caching** is the audio's (`MEDIA_CACHE_CONTROL`): the URL resolves the link of
the moment, which the next scan can change.

**The file is resolved at every request**, through `secure_track_path` with the
library's root, link roots and link policy, exactly as the audio is. A clip
deleted since the scan answers 404, not a 500 and not stale bytes.

## Decision 7 — who may see it

Whoever may play the track may watch its clip; nothing else.

- **Tenancy is in the query.** `clip_for_track(user, track)` joins
  `track_clip` → `library_member` → `account` with `a.disabled = 0`, as
  `canvas_for_track` does, and also requires `t.is_available = 1`: an
  unavailable track's folder is usually the clip's folder too, and the client
  cannot play the track anyway.
- **404 blurs everything.** No track, somebody else's track, a track with no
  clip, a clip gone from the disk, a ticket of another kind, an expired or
  forged ticket — one answer.
- **Re-checked at every redemption**, so a removed member or a disabled account
  loses the clip at once, not at the ticket's expiry.
- **The mint resolves the file, not only the row.** `POST
  /tracks/{id}/clip-ticket` runs the clip's path through `secure_track_path`,
  as the stream route does, and answers 404 when the file is gone. Without
  that, a clip deleted between two scans would still be minted a ticket from
  its stale row. A client renewing an expired ticket (decision 9) would then be
  handed one ticket after another, each answering 404 on its first read.
- **No setting of its own.** The canvas has `accepts_canvas` because accepting
  one spends the operator's disk. A clip spends nothing the operator did not
  already put there, and decision 1's exclusions are how they keep one off the
  server.

## Decision 8 — what a track says about its clip

The native track (`SongItem`, and through it `TrackDetail`, listings and the
catalogue snapshot) gains one field:

```json
"clip": { "format": "mp4", "byte_size": 182734112 }
```

or `"clip": null`. Enough for a client to offer "Watch the clip" without
asking; nothing that names the file, since a track's path is sent only by
`GET /tracks/{id}` (RFC-012 decision 1) and a clip's path is not sent at all.
`origin` stays in the table: it is a fact about the folder, useful to an
operator diagnosing a link, and no client decision depends on it.

**A link change is a track `upsert`.** RFC-009 decision 7 and RFC-007 already
settled it: a clip appearing, changing or going emits the track's `upsert`
through `LibraryChange`, with no origin device since the scan made it, and the
payload does not change — the client re-reads the track, which now carries
`clip`.

## Decision 9 — what the desktop does

In `InstaZDLL/WaveFlow`, once the server ships decision 5:

- **The lookup.** `get_track_clip` (or the frontend's `useTrackClip`) stops
  answering `None` for a server track whose mirrored row carries a `clip`: it
  asks `POST /tracks/{id}/clip-ticket` through `RemoteClient`, prepends the
  binding's `base_url`, refuses an absolute or protocol-relative answer, and
  hands the URL to `ClipVideo` as a remote source — `remote/canvas.rs`'s shape,
  including "404 is no clip, a transient failure is an error".
- **Not cached, and renewed while the clip is open.** A ticket is a permission
  valid for an hour, not a property of the track; the desktop already treats
  the canvas's that way. But a clip, unlike a canvas, can outlive its ticket
  while it is open: a long pause, then a seek, and the range request that
  follows meets an expired ticket. The renewal is decided by **the ticket's
  expiry, never by an HTTP status**. A `<video>` element does not expose the
  status of its requests: its `error` event carries a `MediaError` whose code
  is generic. A 404 at load often surfaces as `MEDIA_ERR_SRC_NOT_SUPPORTED`,
  the same code as a container the webview cannot read. Only the mint's answer
  is observable, since it is a `fetch`. So:

  - **Keep `expires_at`.** The mint already answers it (`StreamTicketResponse`).
    The client keeps it with the URL, and no route is added.
  - **Renew before it breaks.** When the clip resumes after a pause, or seeks,
    and the ticket has expired or expires within a minute, `ClipVideo` first
    mints a fresh one with `POST /tracks/{id}/clip-ticket`. When the mint
    starts, it captures the clip's current position: the seek's target when a
    seek started the renewal, the position it was paused at when a resume did.
    It then points the element at the new URL and applies:

    - **the position**: the captured one, unless the user sought while the
      mint was in flight, in which case the latest seek's target wins;
    - **the state** (playing or paused): the one **last asked for**, whatever
      the position.

    A seek, a pause or a play made during the mint is therefore kept: it
    replaces the one that started the renewal rather than being undone by it. A mint answer that arrives after
    a newer one was started, or after the visit ended, is dropped: each mint
    carries the clip session and a sequence number, and only the latest one of
    the current session is applied. This is the ordinary case, and it never
    shows the user an error.
  - **Renew after an error only for an expired ticket.** If the element raises
    `error` while the ticket is past or within a minute of `expires_at`, the
    client renews once, as above. If the ticket was still valid, the error is
    not an expiry: the clip is reported as one that cannot play, as a local one
    is, and nothing is renewed. Neither case reads the `MediaError` code.
  - **One renewal per incident.** If the element errors again on the renewed
    URL before its first `playing` event, the visit ends. That covers a file
    that vanishes between the mint and the read. Renewing and failing
    therefore cannot go on for ever, whatever the server answers.
  - **A 404 from the mint** means "no clip any more" and ends the visit. The
    mint answers it for a file deleted since the scan (decision 7), not only
    for a missing row.
- **A track present both locally and on the server** (RFC-004's reconciliation)
  plays its local clip when it has one: no network, and the loopback path on
  Linux.
- **The DTO** gains `clip` in `remote/dto`, and the mirror stores whether a
  track has one, so the context menu can offer it offline-aware without a
  request.
- **Linux and fragmented MP4.** A server clip plays as a remote URL, so
  WebKitGTK meets a fragmented file over HTTP and stops. Two answers exist and
  the choice is the desktop's: route a remote clip through the loopback server
  (which already rewrites a fragmented file, at the cost of downloading it
  whole first), or accept the limit and report it as a clip that cannot play.
  The server does nothing either way (decision 6).

## Decision 10 — what this RFC does not do

- **Nothing in the Subsonic façade.** The contract is frozen; a song keeps
  `isVideo="false"`, and `getVideos`, which the façade does not implement,
  stays unimplemented. Subsonic's video model is
  a separate catalogue of video entries, not a clip attached to a song, and no
  validated client asks for either. The `SongItem` field is never mapped by the
  façade, and a contract test holds that.
- **Nothing in a public share.** `/share/{token}` serves audio to an anonymous
  visitor; adding a video surface there is decided separately, not in passing —
  RFC-009 decision 8, word for word.
- **No upload** (decision 1), **no transcoding, no poster frame, no
  thumbnail**, no duration or resolution read at the scan: the player reads
  them from the element when it opens the clip.
- **One clip per track.** Several candidates resolve by the core's order; a
  choice between versions of a video is not a question anybody has asked.
- **No core bump in this work.** The pin moves to v2.1.0 in its own pull
  request, which this RFC's implementation waits for.

## The order of the work

| # | Pull request | Contents |
| --- | --- | --- |
| 0 | `chore(deps): …waveflow-core v2.1.0` | The pin. Its own work, not this RFC's; everything below waits for it. |
| 1 | `fix(scanner): leave a music video out of the walk` | Decision 4: `drop_clips` in the server's walk, with `has_video_track`. Tests in the `scanner` target: a video `.mp4` beside a song is not a track; an audio one is; a lone video one is; a video `Song.mp4` beside a `Song.webm` and no audio file of that name is. Release notes. |
| 2 | `feat(library): record a track's music video at the scan` | Decision 3: the migration, the walk's two sets, the in-memory matcher, the link pass per scope, the `Clips` scope widening in the watcher, and decision 2's parity test against `find_clip`. Tests for the deletion rule: a clip removed from the disk loses its link; an unreadable `Clips/` folder keeps it, and does not let a track that is itself a video fall to `itself`; a clip an exclusion now hides loses it; a clip whose content now fails the check loses it; a clip that cannot be read keeps it. |
| 3 | `feat(api): serve a track's music video` | Decisions 5 to 8: `TicketKind::Clip`, the three routes, the redaction entry, the `clip` field, the OpenAPI document; tests in `media` (ranges, MIME, cross-kind refusals, 404 blur, membership and disabled account re-checked at redemption, a clip deleted after the scan refused by the stream and by the mint) and `subsonic_contract` (nothing changes on the wire). The native guide and the operations guide say where clips go. |
| 4 | `feat(webapp): play a track's music video` | The embedded client, after the desktop's behaviour: the track pauses, the clip plays in the now-playing view, the track resumes at the clip's position. Optional, and may follow the desktop. |
| D | WaveFlow: `feat(clip): play a server track's music video` | Decision 9, in the desktop repository, once 3 is released. |

1 can ship alone and fixes a defect present today. 2 changes nothing a client
sees. 3 is the feature.

## What is still open

- **Linux and fragmented MP4** (decision 9): the desktop's to settle.
- **Receiving a clip** (decision 1): a later RFC, if somebody asks.
