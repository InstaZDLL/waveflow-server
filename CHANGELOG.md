# Changelog

Notable changes to WaveFlow Server. Dates are the release date; the version is
the one `Cargo.toml` carries and the tag `release.yml` refuses to publish if the
two disagree.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow)
consumes `/api/v2`, so anything that changes the shape of a response is called
out first, whether or not it breaks a compile.

## [2.0.0-beta.1] — 2026-09-16

354 commits since `2.0.0-beta.0`.

### Changed — `/api/v2` response shapes

Two changes a native client must follow.

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
- **The OpenSubsonic façade, replayed on 2026-09-16** against DSub 5.5.3 and
  Symfonium 15.0.1 on an Android 17 emulator, with every result read back from
  server state. Playback and seeking on cold transcodes, catalogue browsing,
  the full playlist cycle, favourites, ratings, scrobbles and the server-side
  play queue all behave. See
  [the compatibility matrix](docs/subsonic-compatibility.md).

### Known issues

All of these are present in `2.0.0-beta.0` as well; none is a regression.

- **#224** — a role separator cuts inside an unclosed parenthesis, so a composer
  credit naming a parenthesised publisher becomes several artists.
- **#225** — `getPlayQueue` and `getNowPlaying` name their children `song` where
  the Subsonic schema says `entry`.
- **#226** — a `416` on a cold transcode announces a total length of zero.
- **#219** — a `401` from a session that has ended still renews and replays
  under the next one.

## [2.0.0-beta.0] — 2026-08-23

First beta of WaveFlow Server v2: one axum binary, SQLite as the only database,
FFmpeg for transcoding, an OpenSubsonic façade, and a React client compiled into
the binary. The accepted design is
[RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md).

[2.0.0-beta.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.1
[2.0.0-beta.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.0
