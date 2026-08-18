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
| Search | `search3` | Independent artist, album and song pagination. |
| Playlists | `getPlaylists`, `getPlaylist`, `createPlaylist`, `updatePlaylist`, `deletePlaylist` | Shared with native/web user data. |
| Media | `stream`, `download`, `getCoverArt` | Tenant-authorized streaming and artwork. |
| Lyrics | `getLyrics`, `getLyricsBySongId` | OpenSubsonic `songLyrics` v1 plus legacy lookup. |
| Favorites and ratings | `star`, `unstar`, `getStarred2`, `setRating` | Track, album and artist IDs are tenant scoped. |
| Activity and queue | `scrobble`, `getNowPlaying`, `getPlayQueue`, `savePlayQueue` | Queue order and duplicate tracks are preserved. |
| Shares | `getShares`, `createShare`, `updateShare`, `deleteShare` | Creation returns the public URL; later reads omit the bearer token. |
| Users | `getUser`, `getUsers`, `createUser`, `updateUser`, `deleteUser`, `changePassword` | Administrative methods require an admin account. `changePassword` changes only the Subsonic credential. |
| Compatibility | `getBookmarks` | Returns the standard empty container until audiobook progress is implemented. |

Album-list types are `random`, `newest`, `highest`, `frequent`, `recent`,
`starred`, `alphabeticalByName`, `alphabeticalByArtist`, `byYear` and `byGenre`.
Non-random results use stable title/UUID tie-breaking.

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
