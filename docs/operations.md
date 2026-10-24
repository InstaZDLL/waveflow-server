# Running a WaveFlow Server

What an operator has to know once the server is up: how to back it up, what to do
when the instance key is gone, and the one path whose query string must not reach
a reverse proxy's access log.

## Back up two files, together

```bash
waveflow-server database backup  --output /backups/waveflow-2026-09-16
waveflow-server database restore --input  /backups/waveflow-2026-09-16
```

`data/waveflow.db` and `data/instance.key` are **one unit**: the encrypted
Subsonic credentials cannot be recovered with one without the other. The database
stores a non-secret fingerprint of the key, so a mismatched pair is rejected at
startup rather than after it has replaced your data. Restore runs before SQLite
is opened, and moves the previous pair into a timestamped recovery directory.

## When the key is gone, or has to change

A container moved without its volume, a key left out of a copy, a key that ended
up somewhere it should not have been. The server will not start:

```
instance.key does not match waveflow.db; restore the database and key from the same backup bundle
```

**Restore the bundle if you still have one** — that is what the message is for,
and it costs nothing. What follows is for when the key is genuinely
unrecoverable, or when it must be replaced because it leaked.

### What you lose, and what you keep

This is the **adoption** case below, where the key is genuinely gone. A
rotation, where you still hold it, loses nothing from the left-hand column.

Almost everything is unaffected either way, because almost nothing is
encrypted:

| Lost, and re-entered by hand | Survives untouched |
| --- | --- |
| The Subsonic credential of every account — the dedicated password **and** the `wfsk_` API key beside it | Every account, and its web password — Argon2id, never encrypted |
| Every external scrobbling link (Last.fm, ListenBrainz, Maloja), and the listens queued behind it | The whole catalogue: tracks, albums, artists, libraries and their members |
| | Favourites, ratings, playlists, play queues, listening history, bookmarks |

The API key is not encrypted — it is stored as a SHA-256 hash, which no key
change can touch — but it shares its row with the password that is, and the
columns are `NOT NULL`. So the row goes whole. `credential set` mints a new
password **and** a new API key, and both have to reach the client.

Shares are kept but **their public URLs change**: a share token is derived from
the instance key rather than stored, so every link handed out under the old key
stops resolving. The commands below recompute the stored hash under the new
key, which is what keeps the share reachable at its new address instead of
stranding it at an old one nobody can produce again.

Stream tickets are sealed under the key too. They last an hour and are minted on
demand, so nothing has to be done about them — but it is also why a leaked key
matters even when almost nothing is encrypted under it: whoever holds one can
mint a ticket for any track they can address.

### Two commands, because there are two situations

They are named apart rather than separated by a flag, because one preserves
exactly what the other destroys.

Both run with the server **stopped**: they take the database exclusively, and
refuse to run at all while another process is holding it. And both are a reading until you say
otherwise — without `--confirm` they count what the change costs, print it,
and change nothing at all. Not even a pending migration: if the binary carries
migrations this database has not run, the reading says so and stops, because
applying one is a change and this half makes none.

**The old key is in hand**, and you want a different one — it leaked, or it is
simply time:

```bash
waveflow-server key rotate             # what it would cost
waveflow-server key rotate --confirm
```

Every encrypted value is read with the old key and written back under the new
one. Nothing is lost at all; the share URLs change and that is the whole of it.

**The old key is gone**, and nothing can be carried over:

```bash
waveflow-server key adopt
waveflow-server key adopt --confirm
```

The Subsonic credentials and the scrobbling links go, the queued listens with
them, and a fresh key is bound to the catalogue you already had. `adopt`
refuses outright when the key beside the database still opens it: that is a
rotation, and a rotation costs nothing.

Both move the previous pair into a timestamped `data/pre-key-…` directory
before touching anything — before any pending migration as well as before the
key, so what is in there is the database as the command found it. The way
`database restore` does.

Afterwards, re-set the Subsonic credential of each account with
`credential set` and hand each client its new password and API key, re-link
any scrobbling destination, and **take a fresh backup**: the new key is exactly
as unrecoverable as the one you just replaced.

**Not by hand.** Deleting `subsonic_credential`, `scrobble_link` and
`instance_metadata` in SQL — which is what this page documented before the
commands existed, and what was verified end to end on a real instance of 164
tracks, 130 albums and 160 artists — does bring the catalogue back under a new
key. What it also does is strand every share: the stored token hash was
computed from the old key, no later key can produce it again, and the owner is
shown a URL that answers 404. That is the difference the commands carry.

## One path whose query string must not reach your proxy's log

If you link Last.fm **through a browser**, keep the query string out of the
access log on this path — and only its log, since the token in it is what the
route exists to receive:

```text
/api/v2/scrobble-links/lastfm/callback/
```

Last.fm returns the browser there with `?token=…` appended. That token is worth
an hour and worth a profile: whoever holds it can exchange it for a session key
and start receiving somebody else's listens.

This server keeps none of it — traces record the path only, that path is redacted
in them, the answer carries `no-store` and `no-referrer`, and it redirects at
once to an address with no token in it. **A reverse proxy logs the full request
line by default.** The component that keeps nothing sits behind the component
that keeps everything, and only its operator can close that.

On nginx, log that location with `$uri`, which is the path without the query,
rather than `$request`, which includes it:

```nginx
log_format waveflow_no_query '$remote_addr - $remote_user [$time_local] '
                             '"$request_method $uri $server_protocol" $status '
                             '$body_bytes_sent "$http_referer" "$http_user_agent"';

location /api/v2/scrobble-links/lastfm/callback/ {
    access_log /var/log/nginx/access.log waveflow_no_query;
    proxy_pass http://waveflow:4533;   # passes the URI on unchanged, query included
}
```

Caddy can drop the parameter itself rather than the whole query, through its
access log's `query` filter with a `delete token` action — see their log-filter
documentation for the syntax your version takes. For anything else, the question
to ask of its access-log format is whether the query string can be excluded;
dropping the whole path field is blunt but valid.

These are illustrations to adapt, not configuration this repository tests —
nothing here can reach your proxy. **The command-line journey avoids the whole
question**: no browser comes back, so no token ever travels in a URL. See the
[native API guide](api-v2-guide.md#external-scrobbling).

## Behind a reverse proxy, generally

| Variable | Why |
| --- | --- |
| `WAVEFLOW_PUBLIC_URL=https://music.example.com` | So a created share returns an absolute, externally usable URL |
| `WAVEFLOW_ALLOWED_ORIGINS=…` | Browser-hosted clients such as Feishin. Listed explicitly — **wildcards are rejected**, so credential-bearing requests can never be opened to arbitrary sites |

Every tunable is a field on `Config` in
[`src/config.rs`](../src/config.rs), with its environment variable documented on
it. That file is the authority; this page is not a copy of it.

## Switching metadata enrichment on

**Off by default, and an upgrade leaves it off.** A server that has not been
told otherwise makes no request to any metadata provider.
[RFC-011](rfcs/RFC-011-metadata-enrichment.md) carries the reasoning.

```bash
WAVEFLOW_METADATA_PROVIDERS=lastfm
WAVEFLOW_METADATA_LASTFM_KEY=…   # your own Last.fm API application's key
```

| Variable | Default | What it does |
| --- | --- | --- |
| `WAVEFLOW_METADATA_PROVIDERS` | empty | The switch: an ordered, comma-separated list. Empty is off. The providers are `lastfm` (biographies and similar artists), `deezer` (artist portraits, no key) and `theaudiodb` (artist banners only — not its biographies) |
| `WAVEFLOW_METADATA_LASTFM_KEY` | — | Required when `lastfm` is listed; the server refuses to start without it. No key ships in the binary |
| `WAVEFLOW_METADATA_THEAUDIODB_KEY` | — | Required when `theaudiodb` is listed; the server refuses to start without it. TheAudioDB's public test key is not used |
| `WAVEFLOW_METADATA_IMAGES` | on | `off` keeps the lookups and refuses artist image downloads; `deezer` and `theaudiodb` are then not asked at all, since images are all they are asked for |
| `WAVEFLOW_METADATA_TTL_DAYS` | 30 | How long an answer with content is kept before it is asked again |
| `WAVEFLOW_METADATA_MISS_TTL_DAYS` | 7 | How long "the provider knows nothing about this artist" is kept |
| `WAVEFLOW_METADATA_RATE_PER_SEC` | 4 | Requests per second, per provider. TheAudioDB is never asked faster than once every two seconds, its free tier's ceiling, whatever this says |
| `WAVEFLOW_METADATA_REQUEST_TIMEOUT_SECS` | 10 | How long one request may take before it counts as unanswered |

**Scrobbling's Last.fm key does not switch this on.** An application you
registered for submitting listens is a different consent from lookups about your
whole catalogue. You may place the same key in both variables; each one switches
on only its own feature.

**What the server starts contacting.** `ws.audioscrobbler.com`, over https, and
only when somebody opens an artist. Nothing is fetched at scan time. Each
request carries the artist's name and your key, and no redirect is followed.
Requests are spaced to the rate above, and simultaneous visits to one artist
share one request.

With `deezer` listed, also `api.deezer.com`, which needs no key, and then the
picture it names — but only from Deezer's image hosts
(`e-cdns-images.dzcdn.net`, `cdn-images.dzcdn.net`), only over https, only at
a public address (the address checked is the one connected to), only a JPEG,
PNG or WebP by its own bytes, and at most 10 MiB. A picture that fails any of
these is not fetched and the artist is remembered as having none until the
short TTL runs out.

With `theaudiodb` listed, also `www.theaudiodb.com`, with your key in the
request path, and then the artist's **fanart** — the wide photograph the
artist page draws behind its header when no banner was chosen by hand —
under the same rules, from TheAudioDB's image hosts (`r2.theaudiodb.com`,
`www.theaudiodb.com`). Its fanarts are tried in order and the first real one
is kept; the 1000×185 strips TheAudioDB calls banners are never asked for.
A banner chosen by hand always wins over it, and the page credits TheAudioDB
whenever its fanart is the one shown. If a later answer no longer offers the
fanart, the artist goes back to having no automatic banner; a request that
fails changes nothing.

**What it stores.** In `waveflow.db`, for each provider and each artist name
asked about: the biography, the ranked list of similar artists by name, the
portrait's and the banner's hashes, whether the provider knew the artist, and
when to ask again.
The portrait itself goes in the artwork store beside the covers, and every
artist with that name shows it through the `coverArt` it already carries —
**back it up with the rest of `artwork_dir`**: unlike a cover, it cannot be
extracted again from your files, only downloaded again once the cache entry
expires. Nothing in it belongs to an
account, and nothing records who asked. A failed request stores nothing, so the
next visit asks again.

**What it logs.** The provider and a verdict, such as `answered HTTP 503` —
never the request URL, which carries your key.

## Probes

| Path | Answers |
| --- | --- |
| `/health` | The process is up |
| `/ready` | The database is open and migrated |

Logging is `tracing` with `RUST_LOG`; set `WAVEFLOW_LOG_FORMAT=json` in
production. Traces record `uri.path()` only — never headers, query strings,
tokens or passwords — with share tokens and stream tickets redacted.
