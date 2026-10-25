# RFC-022 — Playback and the web client: loudness, discography, install, AAC

- **Status**: Proposed
- **Implemented by**: [#326](https://github.com/InstaZDLL/waveflow-server/pull/326)
  (this RFC, and all seven decisions with it), and [#335](https://github.com/InstaZDLL/waveflow-server/pull/335) (the longer play
  queue, ReplayGain on Subsonic transcodes and offline listening, from *What
  is still open*). The *Status* field above never
  flips in this project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the stream tickets,
  the frozen Subsonic contract, the one set of domain services)

## Problem

A comparative audit of this server against Navidrome, Meelo, kima-hub and
Ampache (2026-10-24) listed five gaps in how music reaches a listener. Each
was checked in the code before this was written, and two of them turned out
smaller or different from what the audit said:

1. **ReplayGain is read and never applied.** The scanner stores the four
   ReplayGain values and `SongItem` carries them on `/api/v2`, but the web
   client's `Song` type does not even declare them, and the player plays every
   track at the level its master was cut at.
2. **An artist page is one flat grid.** `album.release_types` is stored, and —
   contrary to the audit — it *is* on the native wire already: `AlbumItem`
   serialises it on every album listing. What is missing is a decision about
   what the values mean, so the page cannot group by them, and nothing lists
   the albums an artist is credited on without being their album artist.
3. **The web client cannot be installed.** `webapp/public` has no manifest and
   no icon a phone accepts, so "Add to home screen" makes a bookmark.
4. **Transcoding knows two formats, and refuses the rest.** `src/media.rs`
   encodes MP3 and Opus only. A Subsonic `format=aac` or `format=flac` is
   answered error 10, where every other server plays something.
5. **A Subsonic form POST is capped at 64 KiB.** The audit estimated the cap
   at about 1,600 UUIDs. It is lower, and it depends on the name of the
   parameter: `songId=<uuid>&` is 44 bytes, so `createPlaylist` stops short of
   1,490 tracks; `songIdToAdd=<uuid>&` is 49, so `updatePlaylist` stops short
   of 1,338, less in both cases what the credentials take. A test sending the
   playlist ceiling of 10,000 in one `updatePlaylist` was answered
   `Invalid form body` under the old bound. `savePlayQueue` is a
   different case: it is refused above 400 tracks by `MAX_QUEUE_TRACKS`, a
   domain bound shared with the native API, long before the body is.

## Decision 1 — ReplayGain in the web player, through Web Audio

The `<audio>` element keeps doing what it does: fetching the stream ticket,
answering range requests, seeking. When ReplayGain is on, its output is routed
through `createMediaElementSource` into one `GainNode` and on to the speakers.
The element is not replaced and its URL does not change, so tickets and range
requests behave exactly as before.

- **The preference** has three modes: `off`, `track` and `album`, and a preamp
  from −12 to +12 dB in steps of one, 0 by default. It is per browser, stored
  in `localStorage` beside the theme, for the reason the volume is: how loud a
  device should be is not a property of the account.
- **Which gain.** `album` uses the album gain and the album peak, and falls back
  to the track's pair when the album gain is missing. `track` uses the track's
  pair and falls back to the album's when the track gain is missing: an album
  normalisation is still closer than none. A track with neither plays at unity,
  **without** the preamp: the preamp raises normalised tracks toward the level
  of untouched ones, and adding it to an untouched track would undo that.
- **Clipping.** When the peak paired with the chosen gain is known, the linear
  gain is capped at `1 / peak`, so a quiet track with a hot peak is raised only
  as far as it can go without clipping. There is no switch for it: a gain that
  clips is not a loudness preference. The total is also bounded to
  [−60, +18] dB, against a tag that is simply wrong.
- **Off by default, and opt-in.** Routing an element through an `AudioContext`
  is not free on every browser. Some mobile browsers suspend the context with
  the page, which stops playback on the lock screen — the very case an
  installed client (decision 4) makes common. So the graph is built only when the listener turns ReplayGain
  on, on that gesture or the next play, never on page load. Once built, an
  element cannot be detached from it; turning ReplayGain off afterwards sets the
  gain to one, and a reload goes back to the plain element.
- **Same origin, by construction.** `createMediaElementSource` outputs silence
  for cross-origin media without CORS. Stream tickets are relative URLs
  (`/api/v2/stream/{ticket}`), resolved against the page that is playing them,
  so the media is always same-origin. The player checks it anyway and leaves
  the gain at one for a URL on another origin, rather than going silent.
- **A suspended context is resumed** on every play the listener starts, which
  is always a gesture; the browser refuses a resume outside one.

The gain is computed in `webapp/src/lib/replay-gain.ts`, with no React in it,
and tested there.

## Decision 2 — what a release type means, decided once, on the server

`AlbumItem` gains **`release_group`**, one of `album`, `ep`, `single`, `live`,
`compilation`. It is derived where the row is read, from `release_types` and
`is_compilation`, by one function in `src/services/`. The web client, the
desktop application and anything else group by that field rather than each
reinterpreting MusicBrainz's vocabulary in its own way.

`release_types` holds whatever the files carried: MusicBrainz's primary type
(`album`, `single`, `ep`, `broadcast`, `other`) and any secondary types
(`compilation`, `live`, `soundtrack`, `remix`, `dj-mix`, `demo` and the rest),
joined with `;` by the scanner. Picard's ID3v2.3 output joins them with `/`
instead (`album/live`). Each value is therefore split on `;`, `/` and `,`,
trimmed and compared without case. The first rule that matches wins:

| Rule | Group |
|---|---|
| `is_compilation`, or a `compilation` type | `compilation` |
| a `live` type | `live` |
| an `ep` type | `ep` |
| a `single` type | `single` |
| anything else, or nothing at all | `album` |

- **Secondary before primary.** MusicBrainz files a live EP and a compilation
  album under its secondary type on its own artist pages, and so does this.
- **Unknown means album.** Most libraries carry no release type at all; they
  keep the single "Albums" section they had. `broadcast`, `other`, `soundtrack`,
  `remix` and the rest fall to their primary type, or to `album`.
- **The Subsonic wire does not change.** Nodes are built field by field, so
  `releaseTypes` stays what the file said and the new field is native only.

## Decision 3 — "Appears on"

`GET /api/v2/artists/{id}` gains **`appears_on`**: the albums of which at least
one available track credits the artist in its `artist` role
(`track_participant`), and on which the artist is **not** an album artist
(`album_participant`, role `albumartist`). In the order the page's own albums
use, year then title.

- **The `artist` role only.** A composer, a producer or a performer credit is a
  different relation; a page listing every record someone engineered would be
  a credits page, not a discography.
- **Built on `album_select!`**, so the first bind is the user and every album
  passes through `library_member`. The artist is resolved through the same
  `artist_summary` lookup first, so an artist in another library answers 404
  and its appearances are never computed.
- **The native page only.** It is filled by `artist_page`, as the enrichment is.
  The Subsonic `getArtist` path does not compute it: it has nowhere to put it,
  and a client syncing every artist would pay for a query it never reads.
  `GET /api/v2/artists/{id}/albums` stays the album-artist list it is.

The artist page then shows, in this order and each only when it holds
something: Albums, EPs, Singles, Live, Compilations, Appears on.

## Decision 4 — an installable client, with no service worker

- **A manifest** at `/manifest.webmanifest`: name, `start_url` and `scope` at
  `/`, `display: standalone`, the application's background and theme colours,
  and PNG icons at 192 and 512 pixels, the second also as a **maskable** icon
  whose mark sits inside the safe zone. An `apple-touch-icon` at 180 pixels
  covers iOS, which reads that tag rather than the manifest's icons.
- **No service worker.** Chromium no longer requires one to install, and iOS
  never has. A worker is the one piece of a PWA that can serve stale or
  private data: a cached shell pointing at hashed assets a deploy removed, or
  a response that should never have outlived the session. Not having one is
  the strongest form of the rule that API responses, stream tickets and
  authenticated data are never cached. Offline playback is out of scope; if it
  comes, it comes with its own RFC.
- **Served as itself, never as the shell.** `rust_embed` types the file
  `application/manifest+json`. The router fallback answers the client shell
  for any path it does not know, so a build without the manifest would hand a
  browser HTML where it expects JSON. The manifest and the icons are therefore
  **asset-only paths**: present, they are served with their type; absent, they
  answer a JSON 404, never `text/html` — the treatment `favicon.ico` already
  has, for the same reason.

## Decision 5 — AAC, in ADTS

The third output format is **AAC-LC in an ADTS stream** (`-c:a aac -f adts`,
`audio/aac`, `.aac`), at 192 kbit/s unless a bitrate is asked for.

- **ADTS, not fragmented MP4.** Every transcode is written to a pipe as it is
  produced. ADTS is a sequence of self-describing frames, so it streams from
  the first byte, and a cached file seeks by byte range the way a constant
  rate MP3 does. A fragmented MP4 written to a pipe has an empty `moov` and no
  index, so its duration is unknown and seeking means walking every fragment;
  a regular MP4 needs its index at the front, which FFmpeg can only write by
  rewinding a file it has finished — not a pipe. ADTS is what internet radio
  has streamed AAC in for twenty years, and Safari, Chrome, Firefox and
  Android's players all decode it.
- **FFmpeg's own `aac` encoder.** `libfdk_aac` is better at low rates and is
  missing from most builds, because its licence is not GPL-compatible. The
  native encoder is in every FFmpeg, which is what lets AAC be offered
  unconditionally.
- **On every surface.** `format=aac` on the native stream routes and on the
  stream ticket; `aac`, `m4a` and `adts` on Subsonic.
  `GET /api/v2/transcode/status` gains `formats`, the outputs this server can
  produce, so a client can offer AAC without guessing from a version number.

## Decision 6 — a format the façade cannot produce falls back

A Subsonic `format` the server does not produce — `flac`, `wav`, `alac`, a
typo — is treated **as if no format had been named**: the original when
`maxBitRate` admits it, MP3 at the cap when it does not. The response's
`Content-Type` names what was actually sent, which is how every Subsonic client
already learns it.

- **Why the original.** A client asking for `flac` usually streams a FLAC
  library and wants the file; this is exactly what it gets. A client asking for
  `flac` over an MP3 gets the MP3, which plays, where an error plays nothing.
  This is also what Navidrome does with a format it has no transcoding for.
- **Only Subsonic.** The native API keeps refusing an unknown format with a
  `400`: its clients are versioned with this server and read `formats` from
  the status route, and an error is the honest answer to a request this server
  cannot meet.
- **The frozen contract.** This changes observable behaviour on `/rest`: a
  request answered error 10 now answers audio. No client validated against the
  contract can depend on that error — it is a refusal to play — and
  `docs/subsonic-compatibility.md` records the change beside the others.

## Decision 7 — the form body holds the largest playlist

The Subsonic form body is bounded at
`MAX_PLAYLIST_TRACKS × 64 bytes + 64 KiB`, **705,536 bytes** today. Derived from
the playlist ceiling rather than written down, like the playlist import route's
limit and for the same reason: raising the ceiling must not quietly start
refusing the requests it has just permitted.

- **64 bytes per identifier** covers the longest pair the façade accepts,
  `songIdToAdd=<uuid>&` at 49, and every shorter one. UUIDs need no percent
  encoding.
- **64 KiB of envelope**, the whole of the old limit, for the credentials, the
  playlist's name and comment and whatever else a client sends beside them.
- **Still bounded.** A request is read into memory at most once, up to that
  size; one byte over is refused with error 10 as before. The writer gate is
  bounded by the domain's own ceilings, which this does not move.

## What does not change

- The stream tickets, the range semantics, the transcode cache and its keys
  for MP3 and Opus.
- `savePlayQueue`'s ceiling of 400 tracks (see below), and the native API's
  16 KiB JSON limit.
- Every Subsonic response for a request that worked before.

## The order of the work

One pull request, holding this RFC and all seven decisions, with tests for
each:

- the gain computation and the preference parsing in `webapp/src/lib/`, and the
  discography grouping beside them;
- the release-group rules, one case per row of the table and the separators;
- "Appears on", including an album on which the artist is album artist and also
  credited on a track, which must not appear, and an outsider who gets 404;
- the manifest's type, and its absence answering JSON rather than the shell;
- an AAC transcode identified by `ffprobe` as ADTS AAC, cached and then served
  by range;
- `format=flac` and `format=aac` on Subsonic;
- an `updatePlaylist` of exactly `MAX_PLAYLIST_TRACKS` identifiers in one form
  POST, and a body one byte over the bound refused.

## What is still open

The follow-up pull request named in *Implemented by* closed three of the items
below; each says what was decided.

- **`savePlayQueue` above 400 tracks — done.** `MAX_QUEUE_TRACKS` is now
  `MAX_PLAYLIST_TRACKS`, ten thousand, for both surfaces: the common way to
  fill a queue is to play a playlist whole, and a queue smaller than the
  playlist it came from lost its end on the next save. The cost under the
  writer gate is a playlist's — one `DELETE` and one `INSERT … SELECT` over
  `json_each` — and the tracks are checked with one count instead of a
  hydrated song per id. On the wire, the native queue route carries its own
  body bound, `MAX_QUEUE_TRACKS × 64 + 4 KiB`, as the import route does, and a
  `savePlayQueue` of ten thousand `id=<uuid>&` pairs is 400,000 bytes, inside
  the `formPost` bound of decision 7. The four hundred stays where it bounds
  something else, as `MAX_REQUEST_IDENTIFIERS`: a share, whose opening mints a
  pass per track, and Subsonic `star`/`scrobble`, each identifier a mutation
  of its own.
- **A playlist cannot hold a track twice.** Found while writing the test for
  decision 7: `playlist_track` is keyed on `(playlist_id, track_id)`, so a
  `createPlaylist` or `updatePlaylist` naming one track twice fails on the
  insert and answers error 0, an internal error, rather than either keeping
  the duplicate or refusing it cleanly. The domain's lenient lookup expands
  duplicates, so the refusal comes from the schema alone. A migration, or a
  domain decision to refuse duplicates with error 10, is its own change.
- **ReplayGain on Subsonic `stream` — done, on a transcode only.** Most
  clients apply it from `replayGain`; for those that cannot, a Subsonic
  `stream` that is transcoded anyway carries the gain into FFmpeg's `volume`
  filter. Opt-in twice over: `WAVEFLOW_SUBSONIC_REPLAYGAIN` sets the
  operator's default (`off`, `track` or `album`; `off` unless set), and
  `replayGain=off|track|album` on the request overrides it either way, an
  unknown mode being error 10. **The original is never re-encoded to apply
  it**: that would turn every direct play into a transcode, which is the
  objection this item recorded. The rules are decision 1's, without the
  preamp — a server-wide preamp would be one listener's preference made for
  all — in `src/replay_gain.rs`: album falls back to track and track to album,
  an untagged track plays at unity, the peak caps the gain at full scale, the
  total is held to [−60, +18] dB. The gain, rounded to a hundredth of a
  decibel, is part of the transcode cache key only when there is one, so every
  key written before stays the key of the same encode, and a levelled encode is
  never handed to a request that did not ask for it. The native routes are
  unchanged: the web player levels through Web Audio.
- **Offline playback — done, with a service worker.** Decision 4 shipped no
  worker because a worker is the one piece of a PWA that can serve stale or
  private data. The worker that now ships, `/sw.js`, is built around that
  objection:
  - **it never answers or stores anything of the server's**: every request
    under `/api/`, `/rest/` and `/share/`, the probes, the reference, any
    request carrying a query string, any request that is not a GET and any
    request to another origin goes to the network untouched. No API response,
    token, stream ticket or share pass reaches a cache through it;
  - **it keeps the application shell**: a page is fetched from the network
    first, and the one shell every client route answers is kept for when there
    is none; the content-hashed files are kept by their name, and those a new
    shell no longer names are let go, so the cache holds one build;
  - **a track is kept only when the listener asks** ("Keep for offline" in a
    track's menu): the page fetches the original once through a stream ticket
    and stores it under `/offline/tracks/{id}`, an address of the client's
    own that names the track and carries no credential, beside the song's
    descriptive fields under `/offline/meta/{id}`. The worker answers that
    address with byte ranges, so the player seeks. The ticket's address is
    never a key;
  - **the copies are one account's, and a sign-out deletes them.** Each
    account keeps its copies in a cache named after its id, and an owner
    marker in Cache Storage — read by the page and by the worker alike — names
    the account whose copies are current: the one whose session this browser
    holds, or last held. A **sign-out deletes every copy** and the marker; a
    **different account signing in** deletes the previous one's before it is
    shown anything, then becomes the owner; an expiry or a renewal that fails
    for want of a network deletes nothing, since that is not the account
    leaving. The worker answers `/offline/tracks/{id}` only from the cache the
    marker names, and nothing with no marker. Deleting rather than hiding is
    the safer default on a shared or family computer, where hidden copies
    would still be one storage inspector away; the offline page says that
    signing out deletes them. What stays reachable without a session is
    therefore only what the account that last signed in kept and did not sign
    out of — the reach its open session already had;
  - **`/offline` plays them with no server**: a page beside the sign-in page,
    outside the signed-in shell, with its own `<audio>`, as the share page
    has; the sign-in page links to it when something is kept, and the signed-in
    player plays a kept track from this browser.

  `/sw.js` is an asset-only path, never answered with the shell. The kept
  copies live in that browser only, and are the listener's to remove; a track
  later removed from the library, or from the listener's reach, stays playable
  there until they are, which is what keeping a copy means.
- **A per-device default for ReplayGain** once the lock-screen behaviour of
  Web Audio has been measured on real phones rather than inferred.
