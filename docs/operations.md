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

Almost everything is unaffected, because almost nothing is encrypted:

| Lost, and re-entered by hand | Survives untouched |
| --- | --- |
| The dedicated Subsonic password of every account | Every account, and its web password — Argon2id, never encrypted |
| Every external scrobbling link (Last.fm, ListenBrainz, Maloja) | The whole catalogue: tracks, albums, artists, libraries and their members |
| | Favourites, ratings, playlists, play queues, listening history, bookmarks |

Shares are kept but **their public URLs change**: a share token is derived from
the instance key rather than stored, so every link handed out under the old key
stops resolving.

Stream tickets are sealed under the key too. They last an hour and are minted on
demand, so nothing has to be done about them — but it is also why a leaked key
matters even when almost nothing is encrypted under it: whoever holds one can
mint a ticket for any track they can address.

### The procedure

There is no command for this yet
([#221](https://github.com/InstaZDLL/waveflow-server/issues/221)). Until there
is, it is two writes and a deleted file, with the server **stopped**:

```sql
-- Values only the old key could read. They are unreadable now, not damaged.
DELETE FROM subsonic_credential;
DELETE FROM scrobble_link;
-- Release the fingerprint. The next start binds whatever key it finds.
DELETE FROM instance_metadata;
```

```bash
rm data/instance.key
```

Start the server. It generates a fresh 32-byte key, binds its SHA-256 fingerprint
to the database, and comes up on the catalogue you already had. Then re-set the
Subsonic password of each account with `credential set`, and re-link any
scrobbling destination.

**Take a copy of `data/` before you begin.** The three deletions are not
reversible, and a mistyped table name is a restore away from being someone's
evening.

Verified end to end on a real instance: 164 tracks, 130 albums, 160 artists, one
account and one library came back untouched under a new key.

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

## Probes

| Path | Answers |
| --- | --- |
| `/health` | The process is up |
| `/ready` | The database is open and migrated |

Logging is `tracing` with `RUST_LOG`; set `WAVEFLOW_LOG_FORMAT=json` in
production. Traces record `uri.path()` only — never headers, query strings,
tokens or passwords — with share tokens and stream tickets redacted.
