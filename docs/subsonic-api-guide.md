# Subsonic and OpenSubsonic integration guide

WaveFlow exposes a tested Subsonic 1.16.1 façade for existing music clients.
This guide covers server configuration, authentication, wire formats and the
implemented method surface. The frozen compatibility decisions live in
[RFC-002](rfcs/RFC-002-waveflow-server-v2.md), and real-client results are in
[subsonic-compatibility.md](subsonic-compatibility.md).

## Client configuration

Enter the server origin as the server URL:

```text
https://music.example.com
```

Most clients append `/rest/<method>` themselves. Do not enter `/api/v2`, and
only append `/rest` when a particular client explicitly asks for an API path.

Use the WaveFlow username plus its **dedicated Subsonic password**. The web
password is deliberately not accepted by `/rest`. An administrator creates or
rotates the credential with either:

```powershell
$env:WAVEFLOW_SUBSONIC_PASSWORD = "a-different-app-password"
cargo run -- credential set --actor admin --username listener
```

or `PUT /api/v2/admin/users/{username}/subsonic-credential` as documented in
[the native API guide](api-v2-guide.md). Both paths print or return an API key
exactly once.

WaveFlow has been validated with Symfonium, Feishin, DSub, Substreamer and
Juliet. See the compatibility matrix for exact versions and exercised features.

## Request shape

Both route forms are accepted:

```text
/rest/ping
/rest/ping.view
```

Requests may use GET query parameters or
`application/x-www-form-urlencoded` POST bodies. XML is the default; add
`f=json` for JSON. Include the normal client identification parameters for
maximum third-party compatibility:

```text
v=1.16.1
c=my-client
```

Example JSON ping using form POST:

```bash
curl -X POST https://music.example.com/rest/ping.view \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "apiKey=SUBSONIC_API_KEY" \
  --data-urlencode "v=1.16.1" \
  --data-urlencode "c=my-client" \
  --data-urlencode "f=json"
```

Successful JSON is wrapped in `subsonic-response`:

```json
{
  "subsonic-response": {
    "status": "ok",
    "version": "1.16.1",
    "type": "waveflow",
    "serverVersion": "...",
    "openSubsonic": true
  }
}
```

XML uses the standard Subsonic response root and namespace. JSON collection
fields remain arrays even when empty or containing a single item.

## Authentication

WaveFlow accepts three authentication modes.

### API key

```text
apiKey=SUBSONIC_API_KEY
```

The key identifies the account, so `u` is not required. This is the preferred
mode for a client that implements the OpenSubsonic `apiKeyAuthentication`
extension.

### Token and salt

```text
u=listener
s=RANDOM_SALT
t=md5(subsonic_password + salt)
```

The MD5 construction is required by the legacy protocol and protects the
dedicated password from being sent directly. Use a fresh unpredictable salt
per authentication attempt.

### Password compatibility

```text
u=listener&p=a-different-app-password
```

WaveFlow also accepts `p=enc:<hexadecimal UTF-8 password bytes>`. This is only
wire compatibility, not encryption. Use HTTPS for every authentication mode.

Authentication failures use Subsonic error code `40` without distinguishing an
unknown, disabled or incorrectly authenticated user. Repeated failures are rate
limited and reported with that same code, so a throttled client sees exactly
what a wrong password produces. WaveFlow request tracing records only the path, never credentials or
query parameters; clients should still prefer form POST so their own URL logs
do not retain secrets.

## Common examples

The examples use `apiKey` for brevity. Replace it with `u/t/s` or `u/p` when
required by the client.

List music folders:

```bash
curl -X POST https://music.example.com/rest/getMusicFolders.view \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "apiKey=SUBSONIC_API_KEY" \
  --data-urlencode "v=1.16.1" \
  --data-urlencode "c=my-client" \
  --data-urlencode "f=json"
```

Search with independent page controls:

```bash
curl -X POST https://music.example.com/rest/search3.view \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "apiKey=SUBSONIC_API_KEY" \
  --data-urlencode "v=1.16.1" \
  --data-urlencode "c=my-client" \
  --data-urlencode "f=json" \
  --data-urlencode "query=bjork" \
  --data-urlencode "artistCount=20" \
  --data-urlencode "artistOffset=0" \
  --data-urlencode "albumCount=20" \
  --data-urlencode "albumOffset=0" \
  --data-urlencode "songCount=100" \
  --data-urlencode "songOffset=0"
```

The literal query `""` is match-all for full catalogue pagination. Counts are
capped at 500. Repeated `musicFolderId` parameters select the union of visible
libraries; inaccessible IDs never expose foreign catalogue data.

Create a playlist while preserving repeated track order:

```bash
curl -X POST https://music.example.com/rest/createPlaylist.view \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "apiKey=SUBSONIC_API_KEY" \
  --data-urlencode "v=1.16.1" \
  --data-urlencode "c=my-client" \
  --data-urlencode "f=json" \
  --data-urlencode "name=Road trip" \
  --data-urlencode "songId=FIRST_TRACK_UUID" \
  --data-urlencode "songId=SECOND_TRACK_UUID"
```

Read synchronized lyrics by stable track UUID:

```bash
curl -X POST https://music.example.com/rest/getLyricsBySongId.view \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "apiKey=SUBSONIC_API_KEY" \
  --data-urlencode "v=1.16.1" \
  --data-urlencode "c=my-client" \
  --data-urlencode "f=json" \
  --data-urlencode "id=TRACK_UUID"
```

WaveFlow returns embedded text and UTF-8 `.lrc`/`.txt` sidecars. LRC timestamps
are milliseconds. The legacy `getLyrics` exact artist/title lookup is also
implemented.

Stream an original with a byte range:

```bash
curl "https://music.example.com/rest/stream.view?apiKey=SUBSONIC_API_KEY&v=1.16.1&c=my-client&id=TRACK_UUID" \
  -H "Range: bytes=0-1048575" \
  --output part.bin
```

Request a transcode and temporal seek:

```bash
curl "https://music.example.com/rest/stream.view?apiKey=SUBSONIC_API_KEY&v=1.16.1&c=my-client&id=TRACK_UUID&format=opus&maxBitRate=96&timeOffset=30" \
  --output track.ogg
```

Valid explicit formats are `raw`, `mp3` and `opus`; `ogg` is accepted as an
alias for Opus. `timeOffset` is in seconds.
Without an explicit format, `maxBitRate` is a ceiling: WaveFlow keeps the
original when its known bitrate fits and otherwise transcodes to MP3. Original
and completed cached responses support Range; live transcodes are chunked.

## Implemented methods

Only the following methods are dispatched. Unknown methods answer HTTP `200`
with Subsonic error code `0`, like every other protocol failure.

| Group | Methods | Notes |
|---|---|---|
| System | `ping`, `getLicense`, `getOpenSubsonicExtensions`, `tokenInfo` | Extensions are listed below. `tokenInfo` returns the username the presented credential resolves to. |
| Catalogue roots | `getMusicFolders`, `getIndexes`, `getArtists`, `getArtist`, `getAlbum`, `getSong`, `getGenres`, `getMusicDirectory` | IDs and `musicFolderId` values are UUIDs. |
| Artist and album information | `getArtistInfo`, `getArtistInfo2`, `getAlbumInfo`, `getAlbumInfo2` | Validate tenant access and return the standard empty container; enrichment is not implemented. |
| Album discovery | `getAlbumList`, `getAlbumList2`, `getRandomSongs`, `getSongsByGenre` | Both legacy and ID3 album-list containers are supported. |
| Search | `search3`, `search2` | Independent artist, album and song pagination. `search2` is the same payload in the `searchResult2` container. |
| Playlists | `getPlaylists`, `getPlaylist`, `createPlaylist`, `updatePlaylist`, `deletePlaylist` | Shared with native/web user data. |
| Media | `stream`, `download`, `getCoverArt` | Tenant-authorized streaming and artwork. |
| Lyrics | `getLyrics`, `getLyricsBySongId` | OpenSubsonic `songLyrics` v1 plus legacy lookup. |
| Favorites and ratings | `star`, `unstar`, `getStarred2`, `getStarred`, `setRating` | Track, album and artist IDs are tenant scoped. `getStarred` is the same payload in the `starred` container. |
| Activity and queue | `scrobble`, `getNowPlaying`, `getPlayQueue`, `savePlayQueue` | Queue order and duplicate tracks are preserved. |
| Shares | `getShares`, `createShare`, `updateShare`, `deleteShare` | Creation returns the public URL; later reads omit the bearer token. |
| Users | `getUser`, `getUsers`, `createUser`, `updateUser`, `deleteUser`, `changePassword` | Administrative methods require an admin account. `changePassword` changes only the Subsonic credential. |
| Library maintenance | `startScan`, `getScanStatus` | Rescans every library the account owns or manages and reports progress. |
| Bookmarks | `getBookmarks`, `createBookmark`, `deleteBookmark` | One position per account and track; setting it again moves it. |
| Compatibility | `getTopSongs`, `getSimilarSongs`, `getSimilarSongs2`, `getInternetRadioStations` | Return the standard empty container. WaveFlow computes no recommendations and hosts no radio. |
| Missing data | `getAvatar` | No avatars are stored, so it answers error code 70. |

### Genres

`getGenres` folds spelling variants onto one row: case, punctuation and
spacing are normalised, so "Hip-Hop", "hip hop" and "HIP  HOP" are one
genre with one count. `getSongsByGenre`, `getRandomSongs?genre=` and
`getAlbumList2?type=byGenre` match the same way, so any spelling of a genre
returns everything in it. Until this release the two song methods compared
the raw display string with an ASCII case fold, so asking for a genre
`getGenres` had just listed could return a fraction of its tracks, or none.

### Bookmarks

`createBookmark` takes `id` (a track) and `position` in milliseconds, plus an
optional `comment`. There is one bookmark per account and track — it answers
"where did I stop in this file" — so calling it again moves the existing one
and omitting `comment` clears it. `deleteBookmark` takes `id` and succeeds
whether or not a bookmark was there.

`getBookmarks` returns each one with `position`, `username`, `created`,
`changed`, an optional `comment` and an `entry` holding the full media item,
which additionally carries `bookmarkPosition`. A bookmark on a track that has
become unavailable, or in a library the account has lost, stops being listed
rather than being returned pointing at nothing.

Bookmarks are user data like favorites and ratings, so they reach
`/api/v2/sync/changes` under the `bookmark` entity type and the bootstrap
`/api/v2/sync/snapshot`.

### Rescanning

`startScan` takes no library parameter — it rescans every library the
authenticated account may scan, and answers with the same `scanStatus`
element `getScanStatus` returns, so a client that only calls `startScan`
still learns the state:

```xml
<scanStatus scanning="true" count="1234"/>
```

`count` is the number of available tracks **this account** can reach, not
what the instance holds. `scanning` is true while any of those libraries has
a queued or running job. Scans are asynchronous: a `scanning="false"`
immediately after `startScan` means the work finished, not that it never
started. The equivalents are `POST /api/v2/libraries/{library_id}/scans`,
which scans one library, and `GET /api/v2/scans/{scan_id}` with its
server-sent progress stream.

Being a member of a library is not enough to rescan it. A scan walks the
owner's files and takes the instance's write lock, so it is reserved to the
`owner` and `manager` roles; a `listener` reads the catalogue only. Libraries
the account may only listen to are skipped, not refused: `startScan` names no
library, so an account whose every library is read-only queues nothing and
still answers `ok`, and `count` keeps reporting what that account can reach.
The per-library native route, which does name one, answers `404` — the same
answer a library that does not exist gets.

### OpenSubsonic fields on media items

Songs carry `mediaType`, `isVideo`, `samplingRate`, `channelCount`, `bitDepth`,
`playCount`, `displayArtist`, `artists[]`, `albumArtists[]`,
`displayAlbumArtist`, `contributors[]`, `displayComposer`, `genres[]`,
`musicBrainzId`, `bpm`, `sortName`, `comment`, `isrc[]`, `moods[]`,
`explicitStatus` and `replayGain`; albums add `isCompilation`, `playCount`,
`displayArtist`, `sortName`, `artists[]` and `genres[]`; artists add `sortName`
and `roles[]`. Both songs
and albums carry `played` when they have been played.

`albumArtists[]` and `displayAlbumArtist` are the **album's** credit, not the
track's: a guest appearance names the guest in `artists[]`, while the album still
belongs under the album artist. An album's `artists[]` is the album's own credit
— the artists it is credited to — while its `genres[]` are the union of its
available tracks', folded on the canonical name, so an album spelling "Hip-Hop"
on some tracks and "Hip Hop" on others reports one genre.

`contributors[]` names everyone else the file credits: composer, lyricist,
conductor, arranger, producer, director, engineer, mixer, remixer, DJ mixer and
performer. Each entry is the role, the instrument when a performer names one
(`subRole`), and an artist reference. `displayComposer` is the composers joined
with `•`. `roles[]` on an artist is the capacities it is credited in anywhere in
the catalogue. All three are ordered deterministically, so two responses for one
record are byte-identical.

An artist credited in no album — a composer, say — is reachable by identifier
and by search, but `getArtists` and `getIndexes` list only the artists an album
is credited to.

What WaveFlow does not implement is absent rather than empty, which under the
presence rule is what says so: `moods[]`, `explicitStatus`,
`originalReleaseDate`, `releaseDate`, `releaseTypes[]`, `recordLabels[]` and
`discTitles[]` on an album. Those need album columns the schema does not have.

`sortName` on an album and an artist is no longer among them, and neither are
`contributors[]`, `displayComposer` or `roles[]`: every credit a file names is
stored under the role it names it under, so a composer, a producer or a
performer is an artist row like any other.

These follow the OpenSubsonic presence rule: a supported field is present even
when WaveFlow has no value for it, so an untagged track answers `samplingRate=0`,
`displayArtist=""` and an empty `genres` array. **Do not read an absent field as
an empty one** — absence means the field is not implemented at all. `explicitStatus` is normalised to `explicit` or `clean`; a tag that says "no
advisory" is not a claim that the work is clean, so it sends the empty value.

`musicBrainzId` means a different entity on each item, which is why it is not
the same value everywhere. On a song it is the MusicBrainz **recording**
identifier — the performance. On an `album` it is the **release**, and on an
`artist` the **artist**; both are also under the presence rule, so an untagged
album answers `musicBrainzId=""`. The release and artist identifiers are never
sent at track level, where they would name a different entity.

An album's identifier is derived, not read from one file. Tracks of one album
routinely disagree — a library assembled over years holds files tagged against
different releases of the same record — so the album takes the identifier most
of its available tracks agree on, recomputed at the end of every scan. Ties fall
to the earliest disc and track, so two scans of unchanged files answer the same
thing. An artist takes the identifier from the tracks it is the *first* credit
of, because the tag is one value on a file that may credit several artists.

`getAlbumInfo` and `getAlbumInfo2` carry that release identifier as their
`musicBrainzId` element. They remain otherwise empty: WaveFlow queries no remote
source, so there are no notes and no biography images. Being a classic Subsonic
response rather than an OpenSubsonic one, the element is omitted when the album
has no identifier instead of being sent empty.

Browsing entries are the exception. `getMusicDirectory` renders artists and
albums as `child` elements, and on a `child` the specification defines
`musicBrainzId` as the recording id; a folder standing for an artist or a
release has no recording, so the field is dropped there rather than carrying a
different identifier under that name. Read album and artist identifiers from
`getAlbum`, `getArtist`, `getAlbumList2`, `getArtists` and `search3`.

`replayGain` is an object whose *members* are omitted when unknown, on the
specification's instruction; the object itself is always present, so an untagged
track answers an empty one. `isrc` is an array, repeated `<isrc>` elements in
XML. Both are filled by a scan: a library indexed before this release reports
these fields supported and empty until it is rescanned, which is exactly what the
presence rule means and needs no client change.

`played` is the one exception: it is sent only once the item has been played,
because its empty value would be an empty string rather than a timestamp.
`playCount` is always present and signals the same support.

`artists[]` is every credited artist in tag order, each with `id` and `name`;
`artist` and `artistId` remain the display string and the primary credit.
`genres[]` is the split, deduplicated genre list ordered by name, while `genre`
remains the raw tag string. In XML both are repeated child elements
(`<artists id="..." name="..."/>`, `<genres name="..."/>`); in JSON both are
arrays, `[]` when empty.

Album-list types are `random`, `newest`, `highest`, `frequent`, `recent`,
`starred`, `alphabeticalByName`, `alphabeticalByArtist`, `byYear` and `byGenre`.
Non-random results use stable title/UUID tie-breaking. `byGenre` matches the
canonical genre name, folding case, punctuation and spacing, so `Hip-Hop` and
`hip hop` select the same albums; it requires `genre` and answers error code 10
without it, rather than returning the catalogue unfiltered. A reversed
`fromYear`/`toYear` pair returns the range in descending order. `size=0` answers with an empty container. All ten
types are ordered and paged in SQL; the same vocabulary is available natively as
`GET /api/v2/albums?sort=`.

Repeated parameters such as `songId`, `songIdToAdd`, `songIndexToRemove`,
`musicFolderId`, scrobble `id`/`time`, queue `id` and share IDs retain wire
order. Playlist removals are applied from the highest index downward before
additions.

## Advertised OpenSubsonic extensions

`getOpenSubsonicExtensions` advertises only tested behavior:

| Extension | Version | Behavior |
|---|---:|---|
| `formPost` | 1 | Form POST requests are accepted. |
| `apiKeyAuthentication` | 1 | `apiKey` can replace `u/p` or `u/t/s`; `tokenInfo` resolves it to a username. |
| `transcodeOffset` | 1 | `timeOffset` seeks transcoded playback. |
| `songLyrics` | 1 | Plain and line-synchronized lyrics by song ID. |

Word-level lyrics, translations and other `songLyrics` v2 fields are not
declared. Do not infer support for an extension that is absent from this list.

## Errors

Protocol errors keep the normal Subsonic envelope and include numeric `code`
and `message`. Important codes include:

| Code | Meaning |
|---:|---|
| 0 | Generic error or method not implemented. |
| 10 | Required parameter missing. |
| 40 | Authentication failed. |
| 50 | User lacks the required role. |
| 70 | Requested resource is missing or inaccessible. |

Every protocol answer is HTTP `200`, success and failure alike: the outcome
lives in the envelope `status` and, when it failed, in `error/code`. Do not
branch on the HTTP status — it carries no protocol meaning here. Byte-range
responses on `stream` and `download` are the exception and still answer `206`
and `416`, because those are transport facts rather than protocol outcomes.
Resource lookups are tenant-scoped; code `70` does not reveal whether a foreign
UUID exists. Authentication throttling is reported as code `40`, identical to a
wrong password.

## Browser-hosted Subsonic clients

Set an exact comma-separated origin allow-list, for example:

```dotenv
WAVEFLOW_ALLOWED_ORIGINS=http://127.0.0.1:9180,https://player.example.com
```

Allowed origins may use GET, form POST and OPTIONS and can read the range
headers needed for web audio playback. Wildcard origins are intentionally not
supported with credential-bearing requests.

## Public shares

`createShare` returns an absolute URL when `WAVEFLOW_PUBLIC_URL` is configured,
otherwise a relative `/share/{token}` URL. Preserve that creation response:
`getShares` and synchronization snapshots omit the plaintext bearer token. A
native `/api/v2` caller that supplied an operation ID can recover the same URL
by replaying the identical creation operation, but the Subsonic method has no
operation-ID parameter. The public metadata payload contains token-scoped
stream URLs and cannot stream tracks outside the share.
