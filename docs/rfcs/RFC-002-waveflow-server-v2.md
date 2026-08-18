# RFC-002: WaveFlow Server v2

- **Status:** Accepted
- **Date:** 2026-08-02
- **Supersedes:** the server product model in WaveFlow RFC-001

## Context

The first server architecture treated PostgreSQL, an external Better Auth process and desktop-emitted catalogue operations as the centre of the product. That made self-hosting heavier, duplicated catalogue authority and delayed Subsonic compatibility behind a bespoke web client.

WaveFlow Server v2 is a new self-hosted music server. It scans folders itself, owns the catalogue, streams audio to browsers and compatible clients, and synchronizes only user-owned state with WaveFlow Desktop.

## Decisions

### Process and storage

One Rust/axum binary owns the API, background jobs and eventually the embedded static React application. SQLite is the sole initial database and runs with WAL, foreign keys, a busy timeout and one global mutation coordinator. Parallel extraction is allowed; SQLite commits are serialized and batched.

The v2 schema is fresh. There is no PostgreSQL migration. Operators rescan audio into a new data directory.

### Identity and catalogue authority

The server scanner is authoritative for libraries, folders, tracks, albums, artists, genres and artwork. A track has a stable UUID. Its relative path is a mutable locator, a quick fingerprint narrows relocation candidates, and a full byte hash confirms deduplication or relocation. Tag changes must update the same row. Missing files become unavailable rather than being immediately deleted.

Libraries are private or shared through explicit owner, manager and listener memberships. Repository queries enforce membership before returning catalogue or media rows.

### Authentication and secrets

WaveFlow accounts are local. Web passwords use Argon2id. Access, refresh and API tokens are random opaque values stored only as SHA-256 hashes. Refresh tokens rotate on use and sessions belong to revocable devices.

Every user may have a separate Subsonic app password. It is never the web password. Compatibility with token-and-salt authentication requires reversible verification, so the app password is encrypted with ChaCha20-Poly1305 under a random local instance key. The database and key must be backed up together. SQLite stores only a SHA-256 fingerprint of that key; boot, backup verification and restore reject mismatched pairs before exposing or replacing data.

### Playback

FFmpeg and ffprobe become mandatory at M2. Original files support HTTP Range. Finished cached transcodes support Range. Live transcodes use a temporal seek and chunked response instead of pretending arbitrary output-byte ranges are stable.

Audio files are always read-only. Canonical-path and symlink checks apply before metadata extraction or streaming.

### Protocols and convergence

The M3 beta exposes a tested Subsonic/OpenSubsonic façade. GET, form POST, XML and JSON share the same services. Only implemented extensions are advertised. Credentials in query parameters are removed from request logging.

M4 adds `/api/v2`, Authorization Code with PKCE for WaveFlow Desktop, rotating native tokens and user-data-only synchronization. The server catalogue appears in Desktop as a separate remote source. Existing local and server catalogues are not automatically merged. The cursor, idempotency, ACK and WebSocket contracts are frozen in [RFC-003](RFC-003-waveflow-sync-v2.md).

All web, native and Subsonic writes pass through common services so playlists, favorites, ratings, queue and history converge independent of the calling protocol.

Reads converge on the same rule. Album discovery — the ten orderings, the genre and year filters, and paging — lives once in `DomainServices::list_albums`, with the Subsonic façade and `GET /api/v2/albums` as parameter adapters over it. It had drifted the other way: the orderings existed only in the façade, sorted in memory over a whole-tenant catalogue read, while the native API offered no ordering at all and could not answer "recently added" without paging the catalogue client-side. `GET /api/v2/genres` is the native half of `getGenres`. Album records carry their own `songCount`/`duration`, so no listing loads tracks to size an album.

### Frozen Subsonic v2.0-beta contract

Both `/rest/<method>` and `/rest/<method>.view` accept GET query parameters and `application/x-www-form-urlencoded` POST bodies. `f=json` selects JSON; XML is the default. Repeated parameters preserve wire order. Public IDs and `musicFolderId` are UUID strings. Authentication accepts a dedicated per-user Subsonic password through `u/p` (including `enc:` hexadecimal form), `u/t/s`, or `apiKey`; web passwords are never accepted. Administrative password parameters accept the same plain/`enc:` representation and change only the dedicated Subsonic credential. Authentication failures always use error code 40 and do not distinguish unknown, disabled or incorrectly authenticated users.

Symfonium 14.1.0 performs a discovery request with the exact unauthenticated tuple `GET ping`, `c=Symfonium`, `u=test`, `p=test` after validating the configured account. WaveFlow returns only the standard successful `ping` envelope for that exact probe and does not create a principal or session. Duplicate identity parameters, alternate clients, POST requests, extra token authentication parameters and every method other than `ping` remain authenticated normally.

The response root freezes `status`, `version=1.16.1`, `type=waveflow`, `serverVersion` and `openSubsonic=true`. XML uses the Subsonic namespace. JSON collection fields are arrays even when they contain one item. Media items freeze the common fields `id`, `parent`, `isDir`, `title`, optional `album`/`artist`/`genre`/`year`/disc-track numbers, seconds-based `duration`, `bitRate`, `size`, `suffix`, `contentType`, `type=music`, optional `coverArt`/`albumId`, and ISO-8601 `created`. Album and artist records include UUID, display name, counts and available artwork/year metadata. Frozen 1.16 metadata is omitted when unknown; the OpenSubsonic additions follow the presence rule instead and are emitted with their default value, see *Deliberate deviations from the v2.0-beta freeze*. Media items carry `mediaType`, `isVideo`, `samplingRate`, `channelCount`, `bitDepth`, `playCount`, `played`, `displayArtist`, `artists[]` and `genres[]`; albums add `isCompilation`, `playCount`, `played` and `displayArtist`. `artists[]` is the credited list in tag order from `track_artist`, of which `artist`/`artistId` remain the display string and the primary credit; `genres[]` comes from `track_genre` ordered by name. Fields the scanner does not read — `bpm`, `comment`, `sortName`, `musicBrainzId`, `isrc`, `moods`, `replayGain`, `explicitStatus` — stay absent, because emitting them empty would claim support that does not exist.

Pagination is capped at 500 items per page. `offset` is zero-based; `search3` applies its independent `artistOffset`, `albumOffset` and `songOffset` after tenant filtering. Repeated `songId`, `songIdToAdd`, `songIndexToRemove`, scrobble `id`/`time`, queue `id`, star IDs, share IDs and `musicFolderId` values retain request order. Catalogue endpoints accept the union of repeated authorized `musicFolderId` values and return no foreign data for inaccessible IDs. `createUser` grants every current library when the parameter is absent or exactly the repeated selection when present; `updateUser` replaces Subsonic-managed listener memberships only, preserving owner/manager roles. `getUser` and `getUsers` expose the effective UUID list as `folder[]`. Default catalogue order is case-insensitive display name through the SQLite `NOCASE` collation, which folds ASCII; indexes group by uppercase ASCII initial with `#` fallback. Album lists implement `random`, `newest`, `highest`, `frequent`, `recent`, `starred`, both alphabetical modes, `byYear` (including reversed ranges) and `byGenre`; non-random ties use stable title/UUID ordering. All ten resolve in `DomainServices::list_albums` and are ordered, filtered and paged in SQL. `byGenre` matches on the canonical genre name — case, punctuation and spacing folded — rather than on a raw display string, so one genre is never split in two; it requires `genre` and is rejected with error code 10 without it, rather than answering with an unfiltered catalogue. A `size` of zero is an empty container, not an error. Random-song year and genre filters are applied after repository authorization. Playlist positions are contiguous and removals are applied from highest index downward before additions. The legacy `getAlbumList` method remains an alias for older clients and uses the `albumList` response container; `getAlbumList2` uses `albumList2`. `star` and `unstar` accept the generic `id` form for tracks, albums and artists in addition to the typed album/artist parameters; resolution remains tenant-scoped and rejects ambiguous or invisible entities.

For full-catalogue pagination, `search3` treats the literal query `""` as match-all, as used by Symfonium. `getBookmarks` returns the standard empty `bookmarks` container until audiobook progress is implemented; it never fabricates bookmark state.

`getArtistInfo` and `getArtistInfo2` resolve the requested artist through tenant-scoped catalogue access and return their standard empty containers until artist biography enrichment is implemented. This preserves compatibility with clients such as DSub without fabricating biography or similar-artist metadata. `getAlbumInfo` and `getAlbumInfo2` behave identically for albums: Feishin and Symfonium call them as soon as an album page opens, and an unimplemented-method error there reads to the client as a broken album rather than as absent enrichment.

`playlist.owner` and `share.username` carry the authenticated username. Both collections are read scoped to their owner, so no other name is reachable; the empty string previously emitted made Feishin treat every playlist as another account's and refuse to edit it.

Mutation methods whose Subsonic result is empty (`updatePlaylist`, `deletePlaylist`, stars, ratings, scrobbles, queue save, share deletion and user-management writes) return only the successful protocol envelope. They do not add implementation-specific child elements.

`getOpenSubsonicExtensions` advertises only capabilities covered by protocol tests: `formPost` v1, `apiKeyAuthentication` v1, `transcodeOffset` v1 and `songLyrics` v1. `apiKeyAuthentication` v1 includes `tokenInfo`, which returns the username the presented credential resolves to; advertising the extension without serving that method was itself a conformance failure. No extension is advertised merely because a similarly named endpoint exists. `songLyrics` v1 exposes embedded textual lyrics and UTF-8 `.lrc`/`.txt` sidecars through `getLyricsBySongId`; line timestamps are milliseconds and an unknown language is `xxx`. The legacy `getLyrics` lookup remains available by exact artist/title metadata. The native equivalent is `GET /api/v2/tracks/{track_id}/lyrics`. Word-level cues, translations and other `songLyrics` v2 enhanced fields are not declared.

Cross-origin access is disabled unless the operator supplies an exact comma-separated allow-list through `WAVEFLOW_ALLOWED_ORIGINS`. Allowed origins may use GET, form POST and OPTIONS and may read the byte-range response headers needed for web playback; wildcard origins are not accepted.

Original downloads and streams use repository authorization and the M2 path guard. They forward valid byte ranges to originals and completed cache entries, including 206/416 response semantics; live transcodes still require temporal `timeOffset` seeking. Requested MP3/Opus transcodes use the same FFmpeg/cache service as `/api/v2`. Without an explicit output format, `maxBitRate` is a ceiling: WaveFlow serves the original when its known bitrate is at or below the ceiling and otherwise transcodes to MP3; unknown source bitrate is conservatively transcoded. `getCoverArt` accepts an authorized track, album, artist or content hash. Public share URLs contain a high-entropy bearer token derived with keyed BLAKE3 from the instance key and immutable share UUID; only its lookup hash is persisted. The URL is returned by the successful creation response and authenticated idempotent replays of that operation, while later share reads, updates and synchronization snapshots omit it. `WAVEFLOW_PUBLIC_URL` supplies the external HTTP(S) origin; otherwise the creation response uses a relative URL. The public metadata response supplies token-scoped per-track stream URLs with the same Range/transcode service, and a share cannot stream a track outside its persisted membership. Share tokens are redacted from request trace paths.

### Deliberate deviations from the v2.0-beta freeze

The freeze exists to protect clients validated against v2.0-beta, not to preserve
divergences from the specification. Where the two conflict, WaveFlow now follows
OpenSubsonic. Two such deviations are accepted and both are implemented. Each
requires the compatibility matrix in `docs/subsonic-compatibility.md` to be
re-run before the next tag.

**Every `/rest` answer is HTTP 200.** The Subsonic contract carries the outcome
in the response body: `status="failed"` and an `error` element with its code.
Returning 401, 403, 404, 409 or 429 in addition put the same information in two
places and let proxies, HTTP-level client error handling and offline caches
discard the body before the Subsonic layer read it. The five validated clients
tolerated the old behaviour; a sixth had no reason to. Transport statuses remain
in use where they are transport concerns rather than protocol outcomes: Range
responses still answer 206 and 416, and `/share` and `/api/v2` are unaffected.
Authentication throttling is no longer distinguishable from a wrong password on
the wire, by design — both are error code 40.

**A supported field is emitted even when empty.** OpenSubsonic uses presence to
advertise support: a client tells "this server does not implement `comment`" from
"this track has no comment" only by whether the field appears at all. The
original rule — omit unknown optional metadata — made every unset field look
unsupported. It is replaced for the OpenSubsonic additions: a field WaveFlow
populates from a real source is emitted whenever that source exists, default
value included, so an untagged track answers `samplingRate=0`, `displayArtist=""`
and `genres: []` rather than nothing at all. A field WaveFlow does not implement
stays absent. Frozen 1.16 common fields keep their existing shapes and their
omission on absence, so no client validated against v2.0-beta sees a field it
already reads change meaning.

One field departs from that rule, deliberately. `played` is an ISO-8601 timestamp
whose empty value is the empty string, which is not a timestamp: a client parsing
it strictly would fail on every track nobody has played, which is most of a fresh
library. It is therefore emitted only when the track has been played. `playCount`
is always present and already tells the client that play statistics are
supported, so nothing is lost by it.

### Reconciliation

Reconciliation is an isolated M5 RFC. A unique verified full hash may link automatically. MBIDs create candidates but require confirmation where editions or copies are ambiguous. Metadata-only fuzzy matching never links automatically.

## Delivery and release gates

- **M0 foundations:** empty-directory boot, local admin, registered library, encrypted Subsonic credential, health/readiness/OpenAPI and hermetic tests.
- **M1 catalogue:** deterministic scan and search across MP3, FLAC, AAC, OGG, WAV and DSD; relocation, disappearance, compilations and multi-artists tested.
- **M2 playback:** native playback, transcode, seek, cache concurrency, cancellation and path security tested.
- **M3 v2.0-beta:** OpenSubsonic golden fixtures, backup/restore and a release matrix for Symfonium, Feishin, Substreamer and DSub.
- **M4 v2.0 stable:** web, Subsonic and WaveFlow Desktop convergence; `/api/v1` and the legacy track apply pipeline removed.
- **M5:** conservative local/server linking.
- **M6 v2.1:** complete studio-nocturne web experience, bilingual UI, WCAG AA and Playwright coverage.

Each gate requires formatting, clippy with warnings denied, all-target compilation and tests before the next milestone begins.

## Explicit non-goals for v2.0

- PostgreSQL support or migration from v1.
- Writing tags or modifying audio files.
- Streaming from Spotify, Deezer or other commercial services.
- Fuzzy automatic merging of desktop and server catalogues.
- Full Navidrome endpoint parity beyond the published and tested M3 method matrix.
