# What leaves the server

Every host a WaveFlow Server can contact, what it sends there, when, and the
setting that stops it. This was written from the code (RFC-019 decision 4), and
the code is the authority if the two ever disagree: `src/outbound.rs`,
`src/enrichment.rs`, `src/portrait.rs`, `src/podcast.rs` and
`src/scrobblers/`.

**A server left at its defaults sends nothing.** Booting makes no request, and
no background task does unless an operator configured it *and*, for
scrobbling, an account linked a destination — or, for podcasts, an
administrator subscribed to a feed. The clients are built at boot;
building one sends nothing.

## Rules every request follows

All outbound clients are built by one function, `src/outbound.rs`:

- **No redirect is followed by the client.** A provider cannot bounce a
  request to another host. The one exception is the podcast fetcher, below:
  it follows at most five redirects by hand, and puts every hop through the
  same scheme and public-address checks as the first address before
  contacting it.
- **`HTTP_PROXY` and the other proxy variables are ignored.**
- **The User-Agent is `WaveFlowServer/<version>`**, for example
  `WaveFlowServer/2.0.0-beta.1`. Nothing else in it identifies the instance.
- TLS is rustls; a connection attempt may take a third of the request's
  timeout.
- No URL and no response body is written to the log.
- **Every answer is read under a ceiling** (RFC-023 decision 1): 2 MiB for an
  API answer, 10 MiB for an image. An answer that announces more is refused
  unread; one that keeps coming is cut off at the line. Either is logged as a
  verdict, never with its URL or its body.

The server never turns a request it receives into a request of its own, with
one exception an operator switches on: an administrator subscribing to a
podcast, or asking for an episode, below. Radio
station URLs are stored and handed to clients, never fetched; remote entries in
an imported playlist are dropped; FFmpeg and ffprobe are only ever given local
files inside a library or the data directory, and are told so: every input is
passed with `-protocol_whitelist file`, so neither can open a network or
`concat:` resource whatever a file names inside it (RFC-023 decision 2).

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
`auth.getSession`. Each of these calls is bounded by
`WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS` like a listen.

## Podcasts: the feeds an administrator subscribes to

**Off by default.** On only with `WAVEFLOW_PODCASTS=on`
([RFC-025](rfcs/RFC-025-smart-playlists-and-podcasts.md)). Off, no feed is
fetched and no episode is downloaded, whatever is stored.

**Which hosts.** The ones an administrator names: the feed address they
subscribe to, the enclosure addresses that feed lists, and every redirect
either answers with, up to five. Not a fixed list — which is why every address
is checked before it is contacted:

- http or https (http only while `WAVEFLOW_PODCAST_ALLOW_HTTP` is on, the
  default), no user name or password in the address;
- the name is resolved first, and every address must be public, by the same
  rules as the portrait fetcher above; the connection is pinned to the address
  that was checked, and a redirect is checked the same way before it is
  followed;
- a feed is at most `WAVEFLOW_PODCAST_MAX_FEED_BYTES` (16 MiB) within
  `WAVEFLOW_PODCAST_FEED_TIMEOUT_SECS` (20); an episode at most
  `WAVEFLOW_PODCAST_MAX_EPISODE_BYTES` (1 GiB) within
  `WAVEFLOW_PODCAST_DOWNLOAD_TIMEOUT_SECS` (1800), and every stored episode
  together at most `WAVEFLOW_PODCAST_QUOTA_BYTES` (20 GiB).

**When.** When an administrator subscribes (the feed is read once before it is
stored), when one asks for a refresh, every
`WAVEFLOW_PODCAST_REFRESH_INTERVAL_SECS` (six hours; `0` for never) — the first
one interval after boot, not at boot — and when one asks for an episode.
**Never while somebody listens**: an episode is downloaded once and played from
the server's disk, so the podcast host sees the server, once, and no listener.

**What.** A plain `GET` with the server's User-Agent and nothing about who
asked. The channel's cover address is stored and handed to clients, and never
fetched by the server; a client that shows it fetches it from the feed's host
itself.

**Kept.** The channel's title, description, links and cover address; each
episode's title, description, dates, duration and enclosure address; the
downloaded files, under `podcasts/` in the data directory, named by the
episode's id. The feed address is shown to administrators only — a private
feed carries its subscriber's token in its path or its query — and every other
account is shown at most its scheme and host. Enclosure addresses are shown to
nobody. No feed or enclosure address, and no part of a response, is written to
the log.

## What the browser contacts

The embedded web client loads nothing from a third party: its fonts are
bundled, every API call is relative to the server, and artwork is served by the
server rather than linked from a provider. Its Content-Security-Policy holds it
to that (RFC-019 decision 2). Two exceptions, each deliberate:

- **Internet radio.** The browser plays a station straight from the station's
  own server, at the URL a user entered. That host sees the listener's address.
  Subsonic clients receive the same URLs and connect themselves.
- **Linking Last.fm**, above: a navigation the user starts.

**`/reference`**, the API reference page, was a third until RFC-023: it loaded
the Scalar viewer, unversioned, from `cdn.jsdelivr.net`. The viewer is now part
of the binary (`@scalar/api-reference` 1.73.1) and served by the server itself,
under a policy that admits scripts and connections from this origin only; its
fonts, telemetry, AI agent and publishing toolbar are switched off.
