# WaveFlow native API v2 integration guide

This guide explains how a native, web or automation client uses WaveFlow's
JSON API. The generated contract remains the source for exact schemas:

- `GET /openapi.json` — OpenAPI document;
- `GET /reference` — interactive Scalar reference;
- [RFC-003](rfcs/RFC-003-waveflow-sync-v2.md) — durable synchronization
  semantics.

Examples use `https://music.example.com` as the server and placeholders such as
`ACCESS_TOKEN`, `TRACK_UUID` and `DEVICE_UUID`. JSON field names are
case-sensitive. Public identifiers are UUIDs and timestamps are Unix
milliseconds.

## Base URL and probes

Do not append `/api/v2` when saving the server URL. The base URL is the origin:

```text
https://music.example.com
```

Unauthenticated probes:

```bash
curl https://music.example.com/health
curl https://music.example.com/ready
```

`/health` proves that the process responds. `/ready` additionally verifies
SQLite access; scan progress and FFmpeg capability do not affect readiness.

### First-run setup

On a new data directory, `GET /api/v2/setup` returns `{"required":true}`. The
embedded browser UI can then create the first administrator. The setup write is
unauthenticated but requires an exact same-origin `Origin` header:

```bash
curl -X POST https://music.example.com/api/v2/setup \
  -H "Origin: https://music.example.com" \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"replace-with-at-least-12-characters"}'
```

It returns `201` with `{"user_id":"USER_UUID"}` and is rejected after setup
has completed. Operators may instead use the `account create-admin` CLI command
shown in the main README.

## Authentication choices

### Native username/password session

Use the web password, not the dedicated Subsonic password:

```bash
curl -X POST https://music.example.com/api/v2/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "listener",
    "password": "correct horse battery staple",
    "device_name": "My integration"
  }'
```

The response has this shape:

```json
{
  "access_token": "wfa_...",
  "refresh_token": "wfr_...",
  "token_type": "Bearer",
  "expires_in": 900,
  "user": {
    "id": "USER_UUID",
    "username": "listener",
    "role": "user"
  },
  "device_id": "DEVICE_UUID"
}
```

Send the access token on protected requests:

```bash
curl https://music.example.com/api/v2/libraries \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

Access tokens are short-lived. Refreshing rotates the refresh token; replace
the stored token pair atomically and never reuse the old refresh token:

```bash
curl -X POST https://music.example.com/api/v2/auth/refresh \
  -H "Content-Type: application/json" \
  -d '{"refresh_token":"REFRESH_TOKEN"}'
```

Revoke the current access session with:

```bash
curl -X POST https://music.example.com/api/v2/auth/logout \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

### Long-lived API token

An operator can create a token for automation:

```powershell
cargo run -- token create --actor admin --username listener --name "Home automation"
```

The command prints the plaintext once. Use it directly as a Bearer token. It
has no refresh flow and must be revoked administratively when no longer needed.

### Browser session

Browser clients use `/api/v2/web/auth/login`, `/refresh` and `/logout`. Login
requires an exact `Origin` matching the server origin. The response exposes
only the short-lived access token to JavaScript; the rotating refresh token is
an `HttpOnly`, `SameSite=Strict` cookie.

Refresh and logout require all of the following:

- credentials/cookies enabled on the request;
- the exact accepted `Origin`;
- the readable `waveflow-csrf` cookie copied into the
  `X-WaveFlow-CSRF` request header.

Set `WAVEFLOW_PUBLIC_URL=https://music.example.com` behind a reverse proxy so
origin validation and `Secure` cookies use the public HTTPS origin. External
browser clients also need their exact origins in `WAVEFLOW_ALLOWED_ORIGINS`.
Wildcards are rejected.

### Authorization Code + PKCE for desktop

Native interactive clients should use the system browser:

1. Generate a 43–128 character PKCE verifier and keep it private.
2. Compute `challenge = base64url_no_padding(sha256(verifier))`.
3. Generate and retain an unpredictable `state` value.
4. Open the embedded UI route below in the system browser:

```text
https://music.example.com/authorize?client_id=my.desktop&redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback&code_challenge=CHALLENGE&code_challenge_method=S256&state=STATE&device_name=WaveFlow%20Desktop
```

5. After login and consent, validate `state` on the redirect and exchange the
   returned `code`:

```bash
curl -X POST https://music.example.com/api/v2/oauth/token \
  -H "Content-Type: application/json" \
  -d '{
    "code": "AUTHORIZATION_CODE",
    "code_verifier": "ORIGINAL_VERIFIER",
    "client_id": "my.desktop",
    "redirect_uri": "http://127.0.0.1:49152/callback"
  }'
```

Only S256 is accepted. Supported redirect shapes are loopback HTTP
(`127.0.0.1`, `[::1]` or `localhost`), HTTPS, and reverse-domain private-use
schemes such as `com.example.player://auth`. There is intentionally no client
registry or client secret. An authorization code expires after ten minutes and
is consumed by its first token request, including a failed request; restart the
authorization flow instead of retrying a spent code.

## Catalogue

All catalogue queries are filtered by the authenticated account's library
memberships. An inaccessible or foreign UUID normally returns `404`, not a
distinguishing permission error.

```bash
# Visible libraries
curl https://music.example.com/api/v2/libraries \
  -H "Authorization: Bearer ACCESS_TOKEN"

# Tracks in one library; offset defaults to 0 and limit to 100
curl "https://music.example.com/api/v2/libraries/LIBRARY_UUID/tracks?q=bjork&offset=0&limit=100" \
  -H "Authorization: Bearer ACCESS_TOKEN"

# Cross-library search
curl "https://music.example.com/api/v2/search?q=bjork&offset=0&limit=100" \
  -H "Authorization: Bearer ACCESS_TOKEN"

# Details
curl https://music.example.com/api/v2/tracks/TRACK_UUID \
  -H "Authorization: Bearer ACCESS_TOKEN"
curl https://music.example.com/api/v2/albums/ALBUM_UUID \
  -H "Authorization: Bearer ACCESS_TOKEN"
curl https://music.example.com/api/v2/artists/ARTIST_UUID \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

Browse and search pages accept `offset >= 0` and `1 <= limit <= 500`.
`GET /api/v2/albums` and `/artists` additionally accept an optional
`library_id`. A `SongItem` contains stable `id`, optional `album_id` and
`artist_id`, metadata, `artwork_hash`, and the full-file BLAKE3 `full_hash`. It
also carries the decoded audio properties `sample_rate`, `channels` and
`bit_depth`, the per-account `play_count` and `last_played_at`, and the
structured `artists` (every credit in tag order, each `{id, name}`) and `genres`
lists, plus the tag fields `musicbrainz_id` (the MusicBrainz recording
identifier), `bpm`, `sort_name`, `comment`, the `isrc` list and the four
`replay_gain_*` measurements, the `moods` list and `explicit_status`.
`artist`/`artist_id` stay the display string and the primary credit, so a client
that only wants one name needs no change.

Each tag field is filled by the first scan that runs after the release adding it,
and reads empty before that — the album and artist `musicbrainz_id` are the
most recent, so a library scanned for the earlier tag fields still needs one more
pass to carry them.

An `AlbumItem` carries `song_count` and `duration_ms` for the whole album, so a
listing never has to load the tracks to size it, plus `is_compilation`,
`play_count`, `last_played_at` and `musicbrainz_id`. An `ArtistItem` carries
`musicbrainz_id` too.

Those two are release and artist identifiers, not the recording identifier a
`SongItem` carries under the same name, and neither is read from a single file.
Tracks of one album routinely disagree — a library assembled over years holds
files tagged against different releases of the same record — so an album takes
the identifier most of its available tracks agree on, recomputed at the end of
every scan, with ties falling to the earliest disc and track so two scans of
unchanged files answer the same thing. An artist takes it from the tracks it is
the first credit of, because the tag is one value on a file that may credit
several artists.

## Album discovery

`GET /api/v2/albums` orders and filters in SQL. `sort` takes the same ten values
as the Subsonic `type` parameter — both surfaces resolve to the same query — so
a home screen is one request rather than a full catalogue page-through:

| `sort` | Result |
|---|---|
| `alphabeticalByName` (default) | By title, case-insensitive |
| `alphabeticalByArtist` | By album artist, then title |
| `newest` | Most recently added first |
| `highest` | Rated albums only, best first |
| `frequent` | Played albums only, most played first |
| `recent` | Played albums only, most recently played first |
| `starred` | Favorited albums only, most recently starred first |
| `random` | Shuffled |
| `byYear` | Within `from_year`/`to_year`, ascending |
| `byGenre` | Albums holding at least one track of `genre` |

`byGenre` requires `genre` and answers `422` without it, rather than silently
returning an unfiltered catalogue. Genre matching is on the canonical form, so
`hip hop`, `Hip-Hop` and `HIP HOP` are one genre. For `byYear`, supplying the
bounds reversed (`from_year=2020&to_year=2000`) returns the range in descending
order, matching Subsonic. An unknown `sort` is a `422`.

```bash
# Recently added, first page
curl "https://music.example.com/api/v2/albums?sort=newest&limit=20" \
  -H "Authorization: Bearer ACCESS_TOKEN"

# One decade, oldest first
curl "https://music.example.com/api/v2/albums?sort=byYear&from_year=1990&to_year=1999" \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

`GET /api/v2/genres` lists the genres the account can see with their song and
album counts, optionally narrowed by `library_id`. Counting groups by canonical
name, so one genre spelled differently across libraries is a single row:

```bash
curl https://music.example.com/api/v2/genres \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

```json
[{ "name": "Ambient", "song_count": 128, "album_count": 11 }]
```


Artwork accepts either an `artwork_hash` or an authorized track, album or
artist ID:

```bash
curl https://music.example.com/api/v2/artwork/ARTWORK_HASH \
  -H "Authorization: Bearer ACCESS_TOKEN" \
  --output cover.jpg
```

Lyrics return embedded or UTF-8 `.lrc`/`.txt` sidecar content. Synchronized
line starts are milliseconds; plain lines omit `start`:

```bash
curl https://music.example.com/api/v2/tracks/TRACK_UUID/lyrics \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

## Playback

Clients able to send headers can stream directly:

```bash
# Original bytes, including HTTP Range support
curl "https://music.example.com/api/v2/tracks/TRACK_UUID/stream?format=raw" \
  -H "Authorization: Bearer ACCESS_TOKEN" \
  -H "Range: bytes=0-1048575" \
  --output part.bin

# Opus transcode at 96 kbit/s, seeking 30 seconds into the source
curl "https://music.example.com/api/v2/tracks/TRACK_UUID/stream?format=opus&bitrate=96&offset_ms=30000" \
  -H "Authorization: Bearer ACCESS_TOKEN" \
  --output track.ogg
```

Valid formats are `raw`, `mp3` and `opus`. Byte ranges apply to originals and
completed cached transcodes. A live transcode uses temporal `offset_ms` and a
chunked response; it does not implement arbitrary output-byte ranges.

Browser media elements cannot attach a Bearer header. Mint a sealed ticket,
then resolve the returned relative URL against the same server origin:

```bash
curl -X POST https://music.example.com/api/v2/tracks/TRACK_UUID/stream-ticket \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

```json
{
  "url": "/api/v2/stream/SEALED_TICKET",
  "expires_at": 1787000000000
}
```

The default ticket lifetime is one hour and is configurable with
`WAVEFLOW_STREAM_TICKET_TTL_SECS`. Cache the URL only until `expires_at`.
Redeeming a ticket rechecks current library access. An invalid, expired or
revoked-access ticket returns `404`.

## User-data mutations and idempotency

User data written through the native API and Subsonic façade uses the same
domain services. For every logical mutation, generate one UUID and keep it
stable across transport retries:

```text
X-WaveFlow-Operation-Id: OPERATION_UUID
X-WaveFlow-Device-Id: DEVICE_UUID
```

`X-WaveFlow-Device-Id` is optional, but when present it must belong to the
authenticated account. `X-WaveFlow-Operation-Id` is also optional; omitting it
makes the server generate an ID and therefore loses client-side replay safety.
Replaying the same operation and normalized intent is safe. Reusing an
operation ID for another target or payload returns `409` with code `conflict`.

Example playlist creation:

```bash
curl -X POST https://music.example.com/api/v2/playlists \
  -H "Authorization: Bearer ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -H "X-WaveFlow-Operation-Id: OPERATION_UUID" \
  -H "X-WaveFlow-Device-Id: DEVICE_UUID" \
  -d '{"name":"Road trip","track_ids":["TRACK_UUID"]}'
```

Playlist updates remove zero-based positions first, then append `add`:

```json
{
  "name": "Road trip 2026",
  "comment": null,
  "public": false,
  "remove_indexes": [3, 1],
  "add": ["TRACK_UUID"],
  "clear": ["comment"]
}
```

Omitting an optional field leaves it unchanged. To erase a playlist comment,
name `comment` in `clear`. Shares use the same rule for `description` and
`expires_at`.

Other user-data routes:

| Purpose | Routes |
|---|---|
| Playlists | `GET/POST /api/v2/playlists`, `GET/PATCH/DELETE /api/v2/playlists/{id}` |
| Favorites | `GET /api/v2/favorites`, `PUT/DELETE /api/v2/favorites/{track|album|artist}/{id}` |
| Ratings | `GET /api/v2/ratings`, `PUT /api/v2/ratings/{track|album|artist}/{id}` with `rating` 0–5 |
| Playback activity | `POST /api/v2/scrobbles`, `GET /api/v2/history`, `GET /api/v2/now-playing` |
| Queue | `GET/PUT /api/v2/queue` (`track_ids` allows repeated tracks; maximum 400) |
| Shares | `GET/POST /api/v2/shares`, `PATCH/DELETE /api/v2/shares/{id}` (1–400 tracks) |

`POST /api/v2/shares` returns the bearer URL at creation and on an authenticated
idempotent replay. Later list, snapshot and update responses omit it because
only its hash is stored. Preserve the first response if the URL must be shown
again. Public access uses `GET /share/{token}` and needs no account credential.

## Synchronization

Start or recover from a full snapshot:

```bash
curl https://music.example.com/api/v2/sync/snapshot \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

Persist the returned `cursor`, then page changes in ascending order:

```bash
curl "https://music.example.com/api/v2/sync/changes?after=CURSOR&limit=100" \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

Apply every event idempotently, advance to `next_cursor`, and continue while
`has_more` is true. Limits are 1–500. After durable local application, ACK the
device cursor:

```bash
curl -X PUT https://music.example.com/api/v2/sync/ack \
  -H "Authorization: Bearer ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"device_id":"DEVICE_UUID","cursor":CURSOR}'
```

`GET /api/v2/sync/socket?after=CURSOR` upgrades to a Bearer-authenticated
WebSocket. A text frame is only a wake-up notice:

```json
{"cursor": 1234}
```

Always fetch `/sync/changes` after a notice. The server sends Ping frames every
30 seconds and closes a connection that does not answer before the next
heartbeat. If changes returns `409` with `code=cursor_expired`, discard the
local projection, obtain a new snapshot and resume from that snapshot's cursor.
Do not confuse it with `code=conflict`.

## Administration and scans

Admin Bearer tokens can manage:

| Purpose | Routes |
|---|---|
| Users | `GET/POST /api/v2/admin/users`, `PATCH/DELETE /api/v2/admin/users/{username}` |
| Subsonic credential | `PUT/DELETE /api/v2/admin/users/{username}/subsonic-credential` |
| Libraries | `GET/POST /api/v2/libraries` |
| Membership | `PUT/DELETE /api/v2/libraries/{library_id}/members/{user_id}` |
| Scans | `POST /api/v2/libraries/{library_id}/scans`, `GET /api/v2/scans/{scan_id}`, `GET /api/v2/scans/{scan_id}/events` |

Starting a scan needs more than membership. A scan walks the owner's files and
takes the instance's write lock, so `POST /api/v2/libraries/{library_id}/scans`
is reserved to the `owner` and `manager` roles; a `listener` gets `404`, the
same answer as for a library that does not exist. Reading the catalogue is
unaffected.
| FFmpeg status | `GET /api/v2/transcode/status` |

Creating a library starts its first scan and returns both `library_id` and
`scan_id`. The scan event route uses Server-Sent Events and still requires the
Bearer token.

Setting a Subsonic credential returns a new API key exactly once:

```bash
curl -X PUT https://music.example.com/api/v2/admin/users/listener/subsonic-credential \
  -H "Authorization: Bearer ADMIN_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"password":"a separate app password"}'
```

## Errors and retry rules

Native JSON errors use:

```json
{"code":"validation_error","message":"The request is invalid"}
```

| HTTP | Code | Client action |
|---:|---|---|
| 401 | `unauthorized` | Authenticate again; never infer whether an account exists. |
| 403 | `forbidden` | Stop or request the required role/origin. |
| 404 | `not_found` | Treat the resource as missing or inaccessible. |
| 409 | `conflict` | The operation ID was reused for another intent; reconcile and use a new ID only for a genuinely new operation. |
| 409 | `cursor_expired` | Discard the sync projection and take a fresh snapshot. |
| 422 | `validation_error` | Correct the request; retrying it unchanged cannot succeed. |
| 429 | media response | Back off; a transcode concurrency limit was reached. |
| 503 | `service_unavailable` | Retry with bounded exponential backoff. |

Never branch on status `409` alone: its two codes require opposite recovery
paths.

## Endpoint inventory

The tables above describe common workflows. The authoritative route and schema
inventory is always the running server's `/reference` or `/openapi.json`; use
that contract when generating a client.
