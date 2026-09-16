# Changelog

Notable changes to WaveFlow Server. Dates are the release date; the version is
the one `Cargo.toml` carries and the tag `release.yml` refuses to publish if the
two disagree.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow)
consumes `/api/v2`, so anything that changes the shape of a response is called
out first, whether or not it breaks a compile.

## [2.0.0-beta.1] — 2026-09-16

361 commits since `2.0.0-beta.0`.

### Changed — response shapes

Three changes a client must follow — two on `/api/v2`, which the desktop app
consumes, and one on the Subsonic façade.

- **`GET /api/v2/scrobble-destinations`** — the `unavailable` field now carries
  a **case**, not an English sentence: `no_application_configured` or
  `browser_journey_needs_https`. It was printed verbatim into a client that
  ships in two languages, which told a French reader in English what to change
  in their configuration. The wording now belongs to whoever is doing the
  telling.

- **`GET /api/v2/scans/{id}/events`** — the `progress` frame now speaks the same
  shape as `GET /api/v2/scans/{id}`, where it used to send a second shape of the
  same reading: `id`, `total_files` and `processed_files` instead of `scan_id`,
  `total` and `processed`. A watcher that bound `total_files` showed `undefined`
  from the first progress frame on. The type behind it is deliberately no longer
  serialisable, so the other shape cannot come back.

- **`getPlayQueue` and `getNowPlaying`** name their children **`entry`**, not
  `song`. The Subsonic schema renames a media item inside those two containers
  exactly as it does inside a playlist, and this server sent `song` in both
  until now. A client that decodes against the schema read an empty queue and an
  empty now-playing list; three client campaigns passed over it because every
  client that met it was lenient. `playQueue` also gains the **`username`** the
  schema requires beside `changed` and `changedBy`.

  Both names are deliberately **not** emitted together: a response carrying
  `entry` *and* `song` would conform to neither contract and would need a second
  wire change later to undo. The freeze on the Subsonic contract protects the
  clients that were validated against it, not a divergence from the
  specification — which is what a beta is for.

  `getPlayQueue` with nothing saved still answers a bare `playQueue`, unchanged
  and not schema-conforming. Making it so means omitting the element, which is a
  further wire change on the one call every client makes at startup; it is
  pinned by a test and left to decide after the beta.

### Added

- **External scrobbling.** ListenBrainz, Last.fm and Maloja, each a named
  instance whose URL is part of its identity. A listen leaves by a durable
  queue that survives a restart, is claimed before it is emitted, retries once,
  honours a standard `Retry-After`, and reports a health that cannot read
  `healthy` while three thousand listens wait. Last.fm authorises through a
  browser, and a machine with no browser on it has its own journey.
- **Canvas** — the looping visual a track can carry: its own store, six routes,
  a ticket, an event, and a sweep that collects what a failed upload left behind.
- **Uploads** — a library can be told to accept files; the server decides
  whether it wants one, receives it in chunks under a bounded session, and makes
  it a track.
- **Track corrections** — a track's tags, artists and genres can be corrected
  without rewriting the file. A correction leaves alone what it does not
  mention, and an empty patch writes nothing and announces nothing.
- **A library change feed** — `/api/v2` gained a change feed with a watermark a
  device acknowledges for itself, a retention trim, and an answer that says
  where the stream was cut.
- **Subsonic album output fields** — `originalReleaseDate`, `releaseDate`,
  `releaseTypes[]`, `recordLabels[]` and `discTitles[]` are answered where the
  tags carry them.
- **Search scoped to one library**, on both surfaces.
- **The web client** grew the screens it was missing: uploads, canvas placement,
  the track-tag editor, a live view of a running scan, API tokens, library
  membership, sort and filter on the browse pages, player modes, and the four
  gestures external scrobbling leaves to a client.

### Changed

- **The Subsonic façade is split by method family** rather than living in one
  module, and the domain services are split by domain. Seven defects introduced
  by the move were found and fixed before this release; the façade's observable
  behaviour is unchanged except where this file says otherwise.
- **A song is not a release** — a track no longer answers with fields that
  belong to the album it sits on.

### Fixed

Four of these were found by the client campaigns of 2026-09-16 and cleared
before the tag. All four shipped in `2.0.0-beta.0`; none is a regression this
release introduces.

- **A role separator no longer cuts inside parentheses** (#224). A `COMPOSER`
  reading `Kobee (Melange / INHOUSE), Holy M (Melange / INHOUSE)` names two
  people; the two slashes made three entities, each carrying a parenthesis it
  never opened, and they reached the catalogue as artists that search answered
  with. The reference's rule is untouched — `Bach/Gounod` is still two people
  and `AC/DC` is still one band.
- **`getPlayQueue` and `getNowPlaying` name their children `entry`** (#225), and
  `playQueue` carries `username`. See the response-shape section above: this one
  changes the wire.
- **A `416` no longer announces a length of zero** (#226). A transcode still
  being produced has no complete length, and `bytes */0` did not say "unknown" —
  it said the resource is empty, which a client may believe and never ask about
  again. The header is now omitted instead. A complete file, whose length *is*
  known, states it and answers `Accept-Ranges: bytes`, where it used to claim
  `none` and contradict every other answer for the same file.
- **A refusal that outlived its session neither renews nor replays** (#219). A
  sign-out and a sign-in fit inside one round trip, so a 401 raised for one
  account could arrive after another had signed in — renewing spent the new
  account's rotating refresh token for a request the old one made. Five routes
  did this, including the scan event stream.

- **The transcode a seek abandons is now cached**, and a cache fill has a
  deadline. A seek used to throw away the work it interrupted.
- **Artist favourites survive a change of artist identity spec.**
- **The track pid is a relocation hint**, asked only when path and content hash
  have nothing to say, and dropped wholesale when the track spec changes — a
  hint written under one spec must never be compared against a lookup computed
  under another.
- Numerous fixes across the scrobbling queue, the canvas store, the upload
  session, the event feed's tenancy, and the web client's session handling.

### Verified for this release

- **The upgrade from `2.0.0-beta.0`**: the tagged binary built a database from a
  real scan of 164 files, and the current binary opened it — **16 migrations,
  all successful**, schema 22 → 38, catalogue and FTS5 search intact. Nothing in
  the repository would have caught this: every integration target starts from an
  empty database. Tracked by #223.
- **The OpenSubsonic façade, replayed on 2026-09-16** against the whole
  replayed set — **Symfonium 15.0.1** and **DSub 5.5.3** on an Android 17
  emulator, **Juliet** on a physical iPhone, and **Feishin 1.15.1** on Windows
  desktop — with every result read back from server state rather than from what
  a client displayed. Playback and seeking on cold transcodes, catalogue
  browsing, the full playlist cycle, favourites, ratings, scrobbles and the
  server-side play queue all behave, and seeking by byte range was
  re-established on iOS and on desktop. Substreamer is not part of the set: that
  build no longer installs on the current Android device, and Juliet took its
  place. See [the compatibility matrix](docs/subsonic-compatibility.md).

### Known issues

None outstanding. The four the client campaigns found were all present in
`2.0.0-beta.0` as well, and all four were fixed before this tag rather than
carried into it — #219, #224, #225 and #226, above.

## [2.0.0-beta.0] — 2026-08-23

First beta of WaveFlow Server v2: one axum binary, SQLite as the only database,
FFmpeg for transcoding, an OpenSubsonic façade, and a React client compiled into
the binary. The accepted design is
[RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md).

[2.0.0-beta.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.1
[2.0.0-beta.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.0
