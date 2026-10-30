# Changelog

Notable changes to WaveFlow Server. Dates are the release date; the version is
the one `Cargo.toml` carries and the tag `release.yml` refuses to publish if the
two disagree.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow)
consumes `/api/v2`, so anything that changes the shape of a response is called
out first, whether or not it breaks a compile.

## [Unreleased]

### Added

- **A compose file that puts the server behind Caddy**, in `packaging/caddy/`.
  Caddy terminates TLS with an ACME certificate for a public name (Let's
  Encrypt, or ZeroSSL as its fallback), or one from its local authority for a
  `.internal` or `.home.arpa` name or an IP address. The server publishes no
  port, and the trusted proxy is already set.
  The operations guide explains both certificates, why an untrusted one stops
  offline listening and Android clients, and gives the nginx equivalent.

### Fixed

- **A server that never asked for Last.fm no longer warns about it at every
  boot.** The default Last.fm instance every server declares, with no
  application registered, is Last.fm switched off, which the destinations
  listing already reports as `no_application_configured`. The warning is kept
  for a Last.fm instance the operator named with `WAVEFLOW_SCROBBLE_LASTFM_URL`.

## [2.0.0] — 2026-10-29

The first stable release: 331 commits since `2.0.0-beta.1`.

### Changed — response shapes

- **`getPlayQueue` with nothing saved sends no element at all** (#259), the
  empty `<subsonic-response>` the reference documents. This closes the deviation
  `2.0.0-beta.1` listed under its known issues: the bare `playQueue` it sent had
  no valid form.
- **`PlaylistImport.encoding` is an open set** (#296): the encoding's WHATWG
  name in lower case (`utf-16le`, `shift_jis`, `gbk`, …) where it was
  `utf-8` or `windows-1252`. The two old values are spelled as before.
- **A write on a removed track or an emptied album is refused as not found**
  (#370, #377): 70 on the façade, 404 on `/api/v2`, and nothing is written or
  deleted. A replay under the operation id of a write that succeeded before
  the files went answers as that write did. `savePlayQueue` is the exception:
  it saves the queue without the tracks that went. An album whose files have
  all gone leaves every catalogue listing and search, native or Subsonic; the
  account's favourites and ratings keep its id.
- **Legacy Subsonic clients get the Subsonic answer** (#374, #375).
  `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS` (`DSub` by default) names clients that
  receive no OpenSubsonic fields and XML as `text/xml`, errors included. Every
  other client receives what it did before.
- **Smaller Subsonic corrections**, most found by the third client campaign: an
  unsynced lyrics line is an object with a `value` in JSON (#367); `getIndexes`
  lists tracks with no album as root `child` elements (#372); `getShares` gives
  the owner each share's `url`, and `createShare` accepts an album and shares
  its tracks (#373); `getTopSongs` answers from this account's listening
  history instead of an empty container (#252).

### Added

- **Metadata enrichment, opt-in** (#271–#276) — Last.fm biographies and similar
  artists, and Deezer portraits, behind `WAVEFLOW_METADATA_PROVIDERS`, off by
  default. `getArtistInfo`, `getSimilarSongs` and the artist page read them.
- **Artist imagery** — a portrait and a banner chosen by hand or from a Deezer
  search (#309–#311), and an automatic banner from TheAudioDB's fanart (#314).
- **Splitting joined artist credits** by decision, and a "Needs attention" page
  (#316).
- **The real quality of a file**, measured from its audio in the background,
  with a Quality page and a spectrogram (#319, #320).
- **The library browser** — a track's Properties and download from its menu,
  uploaded `.lrc`, `.txt` and `.ttml` lyrics, and folders of the library's own
  to file tracks into (#298–#306).
- **Libraries and scanning** — an incremental scan that opens only what changed
  and survives an unreadable folder (#324); built-in exclusions and
  `.waveflowignore`, symbolic links, missing tracks and their purge, renaming,
  moving and deleting a library, and a filesystem watcher (#328, #335).
- **Playlists and shares** — a track can repeat in a playlist, public playlists
  are readable by other accounts, and a share link opens a player page (#327,
  #335).
- **Playback** — ReplayGain in the web player, discography by release type and
  "Appears on", an installable web client, and AAC transcoding (#326, #335).
- **Smart playlists, podcasts and internet radio** (#258, #333, #338, #342):
  podcasts behind `WAVEFLOW_PODCASTS`, off by default; stations and episodes
  play through the play bar, with their artwork.
- **Discovery** — lyrics from LRCLIB (opt-in), listening statistics, mixes, and
  similar songs drawn from the catalogue when no provider is set (#334).
- **`.m3u` playlists** imported and exported, with a report of the entries that
  did not resolve (#253).
- **Recovery from a lost or compromised instance key** — `key rotate` and
  `key adopt` (#261).
- **Security** — sign-in throttled against guessing (#323); a one-time token
  guarding first-run setup, security headers on the web client and on shares,
  and a copy of the database and key before every migration (#325); bounded
  reads from other hosts, FFmpeg restricted to local files, `/reference` served
  from the binary, and a disabled account's shares and tickets closed (#332).
- **Compression** of what the server sends (#243).
- **The web client** grew a Songs page with search as you type (#244), a
  file's technical fields and quality meter (#246), a Settings hub with the
  account's own clients and Subsonic credential (#247, #249, #337), a
  Maintenance hub (#340), lyrics and immersive views (#250), an artist's most
  played (#252), links to each credited artist (#341), and a redesign across
  every theme (#289, #318, #347–#349).
- **The image on Docker Hub** as well as GHCR, same tags and digest (#313).
- **Running outside Docker** — `ffmpeg` and `ffprobe` found beside the
  executable before `PATH` (#380), and laptop mode: a `laptop.env` beside the
  executable, `WAVEFLOW_LAPTOP` moving the data folder to the system's
  application-data folder and the cache to 2 GiB, and a second copy on the same
  port handing over instead of starting (#383, #385). The standalone archives
  that will carry them are not published yet.
- **The source of the running version offered to its users** (#384), as
  AGPL-3.0 §13 asks: a link under the sign-in form and in Settings, `source` in
  `/health`, and the licence's URL in the OpenAPI document.

### Changed

- **`.lrc`, `.txt` and `.m3u` files in other encodings are read** through
  `waveflow-core`'s decoder (#296): GB18030, Big5, Shift_JIS, EUC-KR, UTF-16 and
  the Windows code pages, where non-UTF-8 sidecars were dropped and playlists
  fell back to Windows-1252.
- **The image no longer compiles libopus**, and `cmake` is gone from it (#285).

### Fixed

- **A seek probed by a bounded range is answered from its own encode** (#368):
  a `206` with a length, where iOS players got a live stream they could not
  probe.
- **Search no longer finds an album by the title of a removed track** (#371).
- **A write no longer answers 503 while another connection still holds the
  lock** (#331), and a cold boot gets longer than five seconds to open its
  database (#240).
- **A genre named twice no longer fails a whole scan batch** (#358), with the
  other defects the restructuring reviews found.
- **A track listed twice in a playlist keeps two rows**, each with its own
  rating (#279).
- **rustls 0.23.46**, for GHSA-c8m5-c952-6cm4, GHSA-w44r-93j7-qv28,
  GHSA-h2v8-394m-p7vv and GHSA-vp88-798v-h67f. rustls only carries the
  server's outbound requests, through reqwest; the server terminates no TLS.

### Verified for this release

- **The OpenSubsonic façade, replayed on 2026-10-27 and 2026-10-28** against
  **Juliet 8** on a physical iPhone, **Feishin 1.17.0** on Windows desktop, and
  **Symfonium 15.1.0** and **DSub 5.5.3** on an Android 17 emulator, with every
  result read back from server state. It found the defects fixed by #368 and
  #370–#375, and DSub was replayed against those fixes the same day. Juliet
  and DSub keep a `limit` for their own behaviour; see
  [the compatibility matrix](docs/compatibility/subsonic-compatibility.md).

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
  pinned by a test and left to decide after the beta, on #229.

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
  place. See [the compatibility matrix](docs/compatibility/subsonic-compatibility.md).

### Known issues

The four the client campaigns found were all present in `2.0.0-beta.0` as well,
and all four were fixed before this tag rather than carried into it: they are
listed above as #219, #224, #225 and #226.

One deviation is knowingly left in place: **`getPlayQueue` with nothing saved
answers a bare `playQueue`**, carrying none of the `username`, `changed` or
`changedBy` the schema requires. The schema has no way to say "empty queue" —
the reference omits the element — so conforming means removing it, which is a
further wire change on the one call every client makes at startup. No client has
been observed to mind. It is pinned by a contract test so it cannot drift
unnoticed, and #229 carries the decision.

## [2.0.0-beta.0] — 2026-08-23

First beta of WaveFlow Server v2: one axum binary, SQLite as the only database,
FFmpeg for transcoding, an OpenSubsonic façade, and a React client compiled into
the binary. The accepted design is
[RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md).

[Unreleased]: https://github.com/InstaZDLL/waveflow-server/compare/v2.0.0...main
[2.0.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0
[2.0.0-beta.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.1
[2.0.0-beta.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.0
