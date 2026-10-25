# What leaves the server

Every host a WaveFlow Server can contact, what it sends there, when, and the
setting that stops it. This was written from the code (RFC-019 decision 4), and
the code is the authority if the two ever disagree: `src/outbound.rs`,
`src/enrichment.rs`, `src/portrait.rs` and `src/scrobblers/`.

**A server left at its defaults sends nothing.** Booting makes no request, and
no background task does unless an operator configured it *and*, for
scrobbling, an account linked a destination. The clients are built at boot;
building one sends nothing.

## Rules every request follows

All outbound clients are built by one function, `src/outbound.rs`:

- **No redirect is followed.** A provider cannot bounce a request to another
  host.
- **`HTTP_PROXY` and the other proxy variables are ignored.**
- **The User-Agent is `WaveFlowServer/<version>`**, for example
  `WaveFlowServer/2.0.0-beta.1`. Nothing else in it identifies the instance.
- TLS is rustls; a connection attempt may take a third of the request's
  timeout.
- No URL and no response body is written to the log.

The server never turns a request it receives into a request of its own. Radio
station URLs are stored and handed to clients, never fetched; remote entries in
an imported playlist are dropped; FFmpeg and ffprobe are only ever given local
files inside a library or the data directory.

## Metadata enrichment: Last.fm, Deezer, TheAudioDB

**Off by default.** On only for the providers named in
`WAVEFLOW_METADATA_PROVIDERS` (an ordered list of `lastfm`, `deezer`,
`theaudiodb`). With the list empty the engine is never built.

**When.** Only when a signed-in user reads about an artist: the native artist
page (`GET /api/v2/artists/{id}`, and the answers of the image-choice routes),
or Subsonic `getArtistInfo`, `getArtistInfo2`, `getSimilarSongs` and
`getSimilarSongs2`. The lookup runs in the background, only for providers whose
cached answer is missing or older than `WAVEFLOW_METADATA_TTL_DAYS` (30) — or
`WAVEFLOW_METADATA_MISS_TTL_DAYS` (7) when the provider knew nothing. Nothing
runs at boot or on a timer. There is no per-user opt-in: the operator's switch
covers every account.

**What.** The artist's display name as the catalogue holds it, and nothing
about who asked.

| Provider | Host (fixed) | Request | Credential sent |
| --- | --- | --- | --- |
| Last.fm | `ws.audioscrobbler.com` | `artist.getinfo`, then `artist.getsimilar` (limit 50), each with `artist=<name>`, `autocorrect=1` | the operator's `WAVEFLOW_METADATA_LASTFM_KEY`, in the query string |
| Deezer | `api.deezer.com` | `GET /search/artist?q=<name>` | none |
| TheAudioDB | `www.theaudiodb.com` | `GET /api/v1/json/<key>/search.php?s=<name>` | the operator's `WAVEFLOW_METADATA_THEAUDIODB_KEY`, in the path |

**Images.** With `WAVEFLOW_METADATA_IMAGES` on (the default), Deezer's answer
leads to one portrait download from `e-cdns-images.dzcdn.net` or
`cdn-images.dzcdn.net`, and TheAudioDB's to fanart from `r2.theaudiodb.com` or
`www.theaudiodb.com`. Last.fm image URLs are discarded and never fetched. Set
`WAVEFLOW_METADATA_IMAGES=off` and Deezer and TheAudioDB are not contacted at
all, since images are all they are asked for.

An image URL comes from a provider's answer, so it is treated as untrusted
before anything is fetched:

- https only, and the host must be on that provider's list above;
- the name is resolved first, and every address must be public: loopback,
  private, link-local, carrier-grade NAT, documentation and reserved ranges are
  refused, and IPv6 must be global unicast;
- the connection is pinned to the address that was checked;
- at most 10 MiB, of JPEG, PNG or WebP whose bytes match the declared type.

**Manual portrait search.** A user allowed to choose an artist's images can
search Deezer with text they type (`GET /api/v2/artists/{id}/image/candidates`,
at most 200 characters): one search, then up to 20 preview images. Available
only while `deezer` is configured and images are on.

**Pacing.** `WAVEFLOW_METADATA_RATE_PER_SEC` (4) requests a second per
provider, never faster than one every two seconds to TheAudioDB; at most 64
lookups waiting or running; each bounded by
`WAVEFLOW_METADATA_REQUEST_TIMEOUT_SECS` (10); replies capped at 2 MiB.

**Kept.** Last.fm's biography (HTML stripped) and similar artists' names and
scores; image files, under `artwork/` in the data directory by content hash.

## Scrobbling: ListenBrainz, Maloja, Last.fm

**Two switches.** The operator publishes destinations; then **each account
decides for itself** by linking one. Nothing is queued for an account that
linked none, and linking sends nothing — it only stores the account's secret,
encrypted under the instance key.

| Variable | Default |
| --- | --- |
| `WAVEFLOW_SCROBBLE_LISTENBRAINZ_URL` | `https://api.listenbrainz.org` |
| `WAVEFLOW_SCROBBLE_MALOJA_URL` | none |
| `WAVEFLOW_SCROBBLE_LASTFM_URL` | `https://ws.audioscrobbler.com`, usable only with `WAVEFLOW_SCROBBLE_LASTFM_API_KEY` and `WAVEFLOW_SCROBBLE_LASTFM_SECRET` |

Setting a variable to the empty string publishes no destination for that
provider. A user picks a destination by name, never by URL. Destinations must
be https; plain http needs `WAVEFLOW_SCROBBLE_ALLOW_PLAINTEXT` (off) and a
host written as a loopback or private address, `localhost`, or a name ending
`.localhost`, `.local`, `.internal` or `.home.arpa`. Private hosts are allowed
on purpose — Maloja is self-hosted — so no address filtering applies here.

**When.** A listen is queued only when a client reports a finished play
(`submission=true`, native `POST /api/v2/scrobbles` or Subsonic `scrobble`).
"Now playing" is never sent anywhere. A background drain sends the queue once
at boot and then every `WAVEFLOW_SCROBBLE_DRAIN_INTERVAL_SECS` (60): one listen
per request, at most `WAVEFLOW_SCROBBLE_BATCH` (50) per pass, each bounded by
`WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS` (30), retried with backoff up to
`WAVEFLOW_SCROBBLE_MAX_ATTEMPTS` (30). A track with no title or no credited
artist is not sent.

**What.**

| Destination | Request | Fields | Credential |
| --- | --- | --- | --- |
| ListenBrainz | `POST {base}/1/submit-listens` | `listened_at`, `artist_name` (credited artists joined with `, `), `track_name`, and when known `release_name`, `duration_ms`, `recording_mbid`; `submission_client` `WaveFlow Server` and its version | the account's user token, in `Authorization: Token …` |
| Maloja | `POST {base}/apis/mlj_1/newscrobble` | `title`, `artists`, `time`, and when known `album`, `albumartists`, `length` | the account's key, in the JSON body |
| Last.fm | `POST {base}/2.0/` `track.scrobble` | `artist`, `track`, `timestamp`, and when known `album`, `albumArtist`, `duration`, `mbid` | the account's session key `sk`, the operator's `api_key`, and an `api_sig` computed with the operator's secret — the secret itself is never sent |

**Linking Last.fm.** The browser is sent to `www.last.fm` to approve the
application, with a callback to `WAVEFLOW_PUBLIC_URL`; the server then calls
`auth.getSession` once to obtain the session key. The command-line journey
(`scrobble authorize`, `scrobble exchange`) calls `auth.getToken` and then
`auth.getSession`.

## What the browser contacts

The embedded web client loads nothing from a third party: its fonts are
bundled, every API call is relative to the server, and artwork is served by the
server rather than linked from a provider. Its Content-Security-Policy holds it
to that (RFC-019 decision 2). Three exceptions, each deliberate:

- **Internet radio.** The browser plays a station straight from the station's
  own server, at the URL a user entered. That host sees the listener's address.
  Subsonic clients receive the same URLs and connect themselves.
- **Linking Last.fm**, above: a navigation the user starts.
- **`/reference`**, the API reference page. It loads the Scalar viewer from
  `cdn.jsdelivr.net` into the visitor's browser. It is outside the web
  client's policy, and nobody visits it unless they open it.
