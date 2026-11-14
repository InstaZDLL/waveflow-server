# Changelog

Notable changes to WaveFlow Server. Dates are the release date; the version is
the one `Cargo.toml` carries and the tag `release.yml` refuses to publish if the
two disagree.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow)
consumes `/api/v2`, so anything that changes the shape of a response is called
out first, whether or not it breaks a compile.

## [Unreleased]

**A response grows one field.** Every native track (`SongItem`, and through
it `TrackDetail`, the listings and the catalogue snapshot) carries `clip`:
`{ "format", "byte_size" }` or `null`. The Subsonic façade is untouched.

### Added

- **A track's music video** ([RFC-029](docs/rfcs/RFC-029-track-music-video.md)).
  The clip is the video named like the track, case ignored, beside it or in
  a `Clips` folder beside it, or the track's own `.mp4` when it holds a
  moving picture; several candidates are taken in the order `mp4`, `m4v`,
  `mov`, `webm`, `mkv`, each confirmed by its content. The rule is the
  desktop's, and a test holds the server's answer to
  `waveflow_core::scanner::clips::find_clip` over every case it
  distinguishes.
  - The scan records the link in a new table, `track_clip`, and drops it only
    where it could see the clip go, so a `Clips` folder that cannot be read
    keeps its links. A link that changes is the track's `upsert` on the
    change feed.
  - `GET /api/v2/tracks/{id}/clip`, `POST /api/v2/tracks/{id}/clip-ticket` and
    `GET /api/v2/clip-stream/{ticket}` serve the file as it is, whole or by
    range, never transcoded, to whoever may play the track, re-checked at
    every request. The ticket is a fourth kind, `0x04`, which opens neither
    the audio nor a canvas, nor is opened by theirs; no ticket minted before
    the upgrade stops working. Its path is redacted from traces as the other
    tickets' are.
  - The web client plays it. On the playing screen, **Watch the clip** shows
    the video in place of the cover, with its own sound: the track pauses,
    and takes over at the clip's position when the clip closes or ends,
    playing on if it was playing. A ticket that expires while the clip is
    paused is renewed once, at the same position.

### Fixed

- **A music video beside its song is no longer a second track**
  ([RFC-029](docs/rfcs/RFC-029-track-music-video.md) decision 4). `mp4` is
  an audio extension, rightly, so a video named like its song,
  `Album/Song.mp4` beside `Album/Song.flac` or in `Album/Clips/`, was indexed
  as a track whose audio was the clip's soundtrack. The scan now leaves it
  out, by the rule the desktop applies. An `.mp4` is left out only when a song
  of its name sits beside it and the file really holds a moving picture: an
  audio `.mp4` always stays a track, and so does a video with no song of its
  name. A change in a `Clips` folder now scans the folder above it, where
  the song is.

  **On a server that already indexed such a video**, the next scan finds that
  track gone, as it would a deleted file: it becomes unavailable, then is
  purged under the library's rules, and whatever a member had on it goes
  with it: a favourite, a rating, plays, a playlist entry. It was never a
  track; it was the clip of one, played without its picture.

### Changed

- **waveflow-core is pinned to its `v2.1.0` tag**, from `v2.0.0`. Two things
  it changes in a scan, for a file read from now on:
  - an empty picture is passed over in the fallback too: a file with no
    front cover whose first picture holds no bytes now takes the next
    picture, where it used to have no cover at all;
  - a cover's format is read from its bytes (JPEG, PNG, WebP) before the
    MIME type its tag declares, so a WebP thumbnail declared `image/jpeg`,
    or not declared at all, is stored as `.webp` where it was stored as
    `.jpg` or as `.bin`, the latter never served. A cover already in the
    catalogue keeps the format its row was written with, even through
    `database rescan-full`: the row is addressed by the content's hash and
    is not rewritten.

  The rest of the release is the desktop's or is not called by the server:
  the artist identity by MusicBrainz id, the tag workshop, and
  `scanner::clips`, which RFC-029 will use. `canonical_name`, the audio
  extensions, the text decoder and lofty are unchanged, so no derived
  identifier moves.

## [2.2.2] — 2026-11-13

No response changes shape and the schema is 2.2.1's. The Subsonic façade
changes in one way only: a range answer may be longer (see below).

### Fixed

- **The image starts on a data directory it did not create** (#444, #445). It used
  to start as its account (uid 4533), and in a folder the host provides,
  Home Assistant's above all, it stopped on *unable to open database file*.
  It now starts as root, gives the data directory to that account and runs
  the server as it, not as root unless `WAVEFLOW_UID=0` asks for it;
  `docker exec … waveflow-server` steps down
  the same way. `WAVEFLOW_UID` and `WAVEFLOW_GID` name another account, and
  `--user` still runs the server as the account given, without the hand-over.
  The group is now 4533 too, where it was whatever number Debian drew: the
  first start changes the group of an existing data directory, once. The
  hand-over gives the account nothing that was not the data directory's: a
  mount inside it, a hard link, and the target of a symbolic link keep their
  owner, and a data directory set to `/`, reached through a link or under a
  folder that is not root's alone is not handed over at all.
- **A range read runs to the end of the span asked for** (#444, #446). An open
  range, `bytes=N-`, stopped 8 MiB after its start, on the native stream and
  on the Subsonic façade alike. HTTP allows it and `Content-Range` said so,
  but the desktop app took the end of that 206 for the end of the file: a
  track it streamed from the server stopped about a minute in, sooner for a
  high-resolution FLAC. The cap dated from when a range was read into memory
  whole, and has guarded nothing since the body is streamed. A client that
  already asked again from where the answer stopped sees fewer requests and
  nothing else.

## [2.2.1] — 2026-11-10

No response changes shape, the Subsonic façade is untouched and the schema is
2.2.0's.

### Changed

- **waveflow-core is pinned to its `v2.0.0` tag** (#442), instead of the bare
  revision `102df003`. Two things it changes in a scan, for a file read from
  now on:
  - the cover taken from a file's tag is its front cover, not its first
    picture, so a back cover or a booklet page stored ahead of the front no
    longer becomes the track's and the album's cover. A file already in the
    catalogue keeps its cover until the
    file changes, or until `waveflow-server database rescan-full` has every
    file read again;
  - a cover goes to the artwork folder through a partial file and a `rename`,
    so a write cut short by a crash or a full disk no longer leaves a
    truncated image under the name later scans reuse.

  The core now also lists WavPack (`wv`) and Monkey's Audio (`ape`). The
  server leaves them out of the scan and of uploads, as before: the FFmpeg its
  images and archives ship decodes neither. The standalone packaging accepts
  a core pinned by tag, and `SOURCE.txt` still names the commit.
- **Dependencies.** Every Rust dependency at its latest compatible version
  (#438, #442); the web client on vite 8.3.7, `@vitejs/plugin-react` 6.3.0 and
  the patch and minor updates of #439 and #440; the workflow actions of #441.
  The web client is built with bun 1.4.4, pinned in the workflows, the
  `Dockerfile` and the GitLab pipeline instead of following `latest` or `1`.

## [2.2.0] — 2026-11-07

### API

- **`GET` and `PUT /api/v2/server/update`** (#435, RFC-028), administrators
  only, as the other administrative routes: `401` without a credential, `403`
  for an account or a token without administrative authority. The `GET`
  reports the update check: its policy, an administrator's stored answer, the
  running version and its channel, the manifest last read, whether it names a
  newer version, when the check last ran and why it last failed. The `PUT`
  records an administrator's answer and is `409` when the environment decided.
  Nothing else changes shape, and the Subsonic façade is untouched.

### Added

- **The update check** (#435, RFC-028). `WAVEFLOW_UPDATE_CHECK` is `ask` by
  default: nothing is sent until an administrator agrees, and the answer is
  kept in a new `instance_update_check` table. Once on, the server reads its
  channel's manifest on `download.waveflow.app` five seconds after boot, then
  daily and at once when it is turned on, through the outbound client every
  request uses, under the 2 MiB ceiling, the whole request bounded by its
  timeout. A pre-release follows `beta.json`, which every release moves; any
  other version `stable.json`. A manifest is refused unless coherent — a
  Semantic Version, its tag, no pre-release on stable, and a page and notes
  that are `http(s)` addresses — and a failure is kept as a one-word verdict.
  `WAVEFLOW_UPDATE_URL` names another folder (`https`, or `http` on loopback).
  The web client asks the first administrator, then announces a newer version
  with its page and how to update, dismissable per version; Settings ›
  Administration holds the switch.
- **Releases keep a second home** (#434). `release.yml`'s `mirror` job copies
  the archives, the FFmpeg sources, `SHA256SUMS` and the release page to the
  R2 bucket behind `download.waveflow.app`, under `server/releases/<tag>/`,
  reads every file back through the public address against `SHA256SUMS`, and
  writes the channel manifests the update check reads, which never move
  backwards. `mirror-gitlab.yml` pushes `main` and the tags to a GitLab
  backup, and `.gitlab-ci.yml` is a standby twin of the workflows, off until a
  switch; the release logic both run is in `scripts/release/`. The archives of
  2.1.0-rc.1 to 2.1.2 were copied by hand.

### Changed

- **The `Dockerfile` accepts an optional `gitconfig` build secret**, which
  the GitLab pipeline uses to fetch `waveflow-core` from its backup; a build
  without it is unchanged.

## [2.1.2] — 2026-11-06

### Security

- **The chapter guard follows lofty out of a `LIST INFO` it stops reading
  partway** (#431). lofty reads the items of a WAV's `LIST INFO` as chunks of
  their own, and when it stops among them — at a name that is no FourCC, at an
  item longer than the list, or behind an item that runs past its end — it
  goes on with the file's chunks from there rather than from the end of the
  list. A chunk header standing there could send lofty anywhere in the file,
  deep into the audio, onto a tag chunk the guard never reached, since the
  guard went on from the end of the list: a crafted WAV carried nested chapter
  frames past it to lofty, the crash 2.1.1 guards against. 2.1.1 is affected:
  measured on its tag with nine files built for the purpose, the server's own
  call to lofty read the tag in all nine and the guard passed the hostile twin
  of six — the three names that are no FourCC, the item longer than the list,
  the item running past it, and the odd item behind a zero. What was measured
  is the tag reaching lofty's ID3v2 parser, through the call every tag chunk
  goes through; no process was crashed along this path. The guard now works
  out the places lofty goes on from after a `LIST INFO` and walks on from
  each, and tests hold the three ways lofty stops, both continuations of an
  odd item, and the padding of an odd list, against lofty itself. It also
  walks the items of every `LIST`, whether or not the list fits the file:
  lofty keeps its count of what is left of the file apart from where it reads,
  each list whose last item runs past its end leaves that count eight bytes
  ahead, and after enough of them lofty takes a last `LIST INFO` longer than
  the file and reads the tag its items lead to. That way past the guard never
  shipped — 2.1.1 refuses those files, measured on its tag; it was found in
  review of #431, in the walk that pull request first proposed. A seeded sweep
  of 100,000 malformed WAV and 100,000 malformed AIFF layouts now runs with
  the tests. Each layout is built twice around one tag chunk: lofty reads the
  twin whose tag it refuses with an error of its own, which shows it handed
  that chunk to its ID3v2 parser, and the guard's chunk walk (`check_chunks`,
  not the scan of the file's head) reads the twin that nests chapter frames.
  No layout whose tag lofty parses may pass the walk.

### Fixed

- **The chapter guard no longer refuses a large WAV, AIFF or FLAC that nests
  nothing, in the shapes a conforming writer produces** (#428, the known issue
  2.1.1 shipped). The guard gave up on a file once it had read 64 MiB of it,
  and two honest shapes reached that budget: a WAV or an AIFF with a chunk of
  odd length ahead of its audio, where the walk read the audio's header one
  byte off and, whenever the size it misread landed inside the file — which
  depends on the size of the audio, not on how large the file is — read the
  audio itself as a chunk; and a file that opens on neither but carries
  `RIFF` or `FORM` further in, such as a FLAC that keeps the chunks of the WAV
  it was made from (`flac --keep-foreign-metadata`), where the walk left the
  embedded structure and read the audio as chunk headers. The guard now walks
  a WAV or AIFF only when `RIFF`/`FORM` opens the file and carries the
  matching form type, the only shape lofty reads as chunks. Within that walk
  it stops where lofty stops: at a chunk name that is no FourCC, which is how
  the audio's header looks when read one byte early from a zero padding byte,
  and at a chunk declaring more than the file holds, from which lofty reads no
  tag. And it reads no chunk body but a tag's, since lofty skips every other
  body with one seek to its end: behind a padding byte that is missing or
  written as a letter, where the walk goes on inside the audio — as lofty does
  in the second case — it steps from header to header there without reading
  the audio. Tests hold each of those behaviours of lofty, so a change in
  lofty fails them. A tag nested behind an odd chunk, or behind the audio that
  follows one, is still reached and refused, whatever stands where the padding
  byte goes, because both continuations are still walked. A refusal for an
  exhausted budget, or for a read that fails partway through the file, is now
  told apart from a nesting and no longer claims the file nests chapter
  frames. On a full rescan a file the guard refuses marks its track
  unavailable rather than deleting it; the file is untouched and a later scan
  that reads it restores the track. #430 left a residue behind a padding byte
  that is missing or a letter, a few files in a thousand of those refused for
  the budget; #431 closes it by reading no body lofty skips. Measured on built
  files with pseudo-random audio of 100 MiB to 3.5 GiB, 1,600 sizes for each
  padding: none refused.
- **The chapter guard refuses a file it fails to read partway** (#429). It
  took any read error for the end of a clean file and passed the file, while
  the parser that runs next reads on and could still meet chapter frames the
  guard never reached. A read that fails partway now refuses the file — not
  indexed by a scan, an upload answered `422` — where 2.1.1 passed it; a read
  that is merely interrupted is retried, as the parsers retry it, rather than
  refusing a benign file. #430 closes the reads #429 left out in the walk of
  a WAV or AIFF: the seek to the file's end that measures it, and the header
  and the body of a chunk, whose failure still skipped the check of the tags
  behind it.
- **The chapter guard refuses a DSD file whose header it fails to read**
  (#433). The refusal of a failed read (#429) covered the scan of a file's
  head and the read of a tag once located, but not the short reads that locate
  the tag of a `DSF` or `DFF` file: the four bytes naming the container, the
  offset a `DSF` gives its tag, the length and the chunk headers of a `DFF`. A
  failure of one of those still passed the file, as in 2.1.1, and the DSD
  reader's own read of the same bytes could then succeed and hand the tag to
  the `id3` crate unchecked. They are now refused like any other failed read,
  and so is a reader that cannot tell where it stands. A file that merely ends
  before such a header still passes, since the DSD reader meets the same end,
  and an interrupted read is retried. Read from the code in review of the
  2.1.2 release, raised by CodeRabbit on #432; outside the tests, no file was
  made to fail a read on demand.

### Known issues

- **A file that changes between the guard and the reader is not covered.**
  The guard reads a file, then lofty or the DSD reader opens it again by its
  path. A file that grows or is rewritten between the two, such as one still
  being copied into a scanned folder, is parsed as it then is, not as the
  guard saw it. Where the guard met the end of the file ahead of a tag — fewer
  than ten bytes where a DSF says its tag stands, for one — it passed a file
  with no tag there, and the reader may find one. This has been so since
  2.1.1: the guard and the reader each open the file and read it at their own
  moment, so a change between the two cannot be ruled out. It takes write access
  to the library folder and a change that lands between the two reads. An
  upload opens no such window by itself, being received whole, hashed and
  moved into place before it is read, but nothing keeps a file from being
  changed in the folder afterwards, by someone who can write there.
  Once the file is whole, every later read of it meets the guard again, the
  scan at the next start included. Read from the code, not measured.

## [2.1.1] — 2026-11-05

### Security

- **A file whose ID3v2 chapter frames nest no longer crashes the server**
  (#426). A `CHAP` or `CTOC` frame can hold frames of its own, and the tag
  readers the server uses, lofty and the `id3` crate, recurse once per nested
  level with no limit, so a crafted tag of a few kilobytes overflowed the stack
  and aborted the whole process. An upload put the file in the library before
  reading it, and a file in a scanned folder was read by the next scan, so the
  scan at start-up met it again and the server crashed on every restart until
  someone removed the file by hand. A check now runs ahead of both readers, for
  every format and for the ID3 tag inside a DSF or DFF file, and refuses a tag
  whose chapter frames nest more than three levels under the first, well short
  of the depth that overflows the stack; one chapter frame inside another is
  not refused. An upload of a refused file is answered `422` and the file is
  taken back out of the library; should the disk allow neither moving it back
  nor deleting it, the answer is `503` and the file and its session stay. A
  scan records an error for the file and goes on. A real chapter layout, with
  a table of contents, titled chapters, links and pictures, is read as before.
  Every version from 2.0.0 to 2.1.0 is affected.

### Known issues

- **The check refuses some large files that nest nothing** (#428). It gives up
  on a file once it has read 64 MiB of it, and two shapes make it read that
  much with no chapter frame in them: a WAV or an AIFF with a chunk of odd
  length ahead of its audio, and a file that is neither with the letters
  `RIFF` or `FORM` in its first 64 KiB, such as a FLAC that keeps the chunks
  of the WAV it was made from. Measured on files built for the purpose:
  refused at 70 MiB, read at 10 MiB. The file is not indexed, or its upload is
  answered `422`, for a reason that speaks of nested chapter frames; the file
  itself is never changed.

## [2.1.0] — 2026-11-03

### Added

- **The web client in seven more languages**: German, Spanish, Italian,
  Japanese, Korean, and Simplified and Traditional Chinese, beside English and
  French. Settings › Language offers each, and a browser set to one opens in
  it. A Chinese browser gets the script its reader uses: Traditional for
  Taiwan, Hong Kong and Macao, Simplified elsewhere. Native speakers reviewed
  all seven.
- **`waveflow-server health`**, which asks `/ready` at `WAVEFLOW_BIND` and
  exits 0 on its readiness answer, or prints one line and exits 1. The Docker
  image's `HEALTHCHECK` now runs it, and curl, installed for that check alone,
  has left the image.
- **Laptop mode opens the browser** on start, and a second copy opens it on
  the one already running (#397). On a first start on Windows and macOS,
  unless it listens on a wildcard address, it lands on the sign-in screen with
  the setup token filled in, handed over in the URL fragment so it reaches no
  request and none of the server's logs.
  `WAVEFLOW_LAPTOP_OPEN_BROWSER=false` turns it off; an installed server never
  opens one.
- **A minimal LGPL FFmpeg, built in CI** by `packaging/ffmpeg/build.sh` for
  Windows, macOS (arm64 and x86_64) and Linux (x86_64 and arm64), from pinned
  and checksummed tarballs, with only the components the server uses and no
  protocol but `file` and `pipe`; each build passes the `media`, `quality` and
  `canvas` tests before it is uploaded. The standalone archives ship it; the
  image keeps Debian's FFmpeg for now.
- **Standalone archives on every release** (#399), to run the server on one
  computer without Docker: `waveflow-server-<version>-windows-x86_64.zip`,
  `-macos-arm64.zip`, `-macos-x86_64.zip`, `-linux-x86_64.tar.gz` and
  `-linux-arm64.tar.gz`, each one folder holding the server, its own FFmpeg,
  `laptop.env`, the licences and a `README.txt`, with
  `ffmpeg-<version>-sources.tar.gz` and a `SHA256SUMS` over all of them. Windows
  and macOS warn on the first launch, and the `README.txt` says what to click
  past. The Linux server is static against musl and runs on any distribution;
  the Windows one needs no Visual C++ redistributable. Each archive is started
  and made to play a file on its own platform before the release attaches it,
  and the image's tags wait for them too, so a tag whose archives fail publishes
  neither.
- **The standalone executables are signed** (#401), as the desktop app's builds
  are: with a self-signed Authenticode certificate on Windows, ad hoc on macOS.
  A file changed after signing no longer matches its signature, but neither
  system trusts these signatures, so the warnings of the first launch stay. A
  release needs the `SIGNTOOL_PFX_BASE64` and `SIGNTOOL_PFX_PASSWORD` secrets:
  without them the archives stop before they build and nothing is published,
  though the image's build still runs.
- **Transcoding profiles** (#403, [RFC-027](docs/rfcs/RFC-027-transcoding-profiles.md)):
  an account can say once what an app that does not ask should get, as a default
  and per Subsonic client name, in Settings › Playback or through
  `/api/v2/transcoding-profiles`. A request naming a format or a bit rate is
  answered as before, and so is every request of an account with no profile.
  Under one, a Subsonic `stream` or a native stream that names neither may now be
  transcoded, a source at or under the bit rate excepted. Stream tickets, and so
  the web player, downloads and public shares keep the original. An app that seeks
  by byte range may fail to seek in a transcode still being encoded; a row of
  Original for it puts it right.
- **The transcoding state in the web client's administration**: whether the
  server can transcode (without it, listeners get the original files only),
  the transcodes running against the global limit, the per-account limit and
  the output formats, read from `GET /api/v2/transcode/status` and asked again
  every ten seconds while the screen is open.
- **The web client follows the library event stream** (#405): an open tab
  shows what a scan, an upload from another device or a tag correction changed
  without a reload. `GET /api/v2/libraries` answers a new field for it,
  `events_cursor`, the head of each library's change feed — where a client that
  has just read the catalogue starts following, which the feed alone cannot
  say once retention has trimmed it.

### Changed

- **The image runs on Debian 13 (trixie)** instead of Debian 12, and its FFmpeg
  goes from 5.1 to 7.1 (#395). **Its first boot measures every lossless file
  again**: a quality measurement is current only for the FFmpeg that took it
  (RFC-016), so the boot deletes every row of `track_quality` and the
  background analysis takes each lossless file anew, one at a time by default
  and never while a scan runs. On a large library that is hours of one core,
  and each file reads `pending` until its turn comes.
  `WAVEFLOW_QUALITY_PARALLELISM` paces it: raise it to finish sooner on a
  machine with cores to spare, or set it to `0` to hold the analysis off until
  a quieter moment.

## [2.0.1] — 2026-10-30

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

[Unreleased]: https://github.com/InstaZDLL/waveflow-server/compare/v2.2.0...main
[2.2.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.2.0
[2.1.2]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.1.2
[2.1.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.1.1
[2.1.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.1.0
[2.0.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.1
[2.0.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0
[2.0.0-beta.1]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.1
[2.0.0-beta.0]: https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.0.0-beta.0
