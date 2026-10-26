# Running a WaveFlow Server

What an operator has to know once the server is up: how to create the first
account, how to back it up, what to do when the instance key is gone, and the
one path whose query string must not reach a reverse proxy's access log. What
the server sends to other hosts, and how to stop it, is in
[privacy](privacy.md).

## The first account needs the setup token

Until an account exists, whoever creates the first one becomes the
administrator. So the setup screen, and `POST /api/v2/setup`, also ask for a
one-time **setup token** ([RFC-019](rfcs/RFC-019-exposure-hardening.md)):

- by default the server draws one at boot and writes it to `setup-token` in
  the data directory. On Unix the file is readable by its owner only; on other
  platforms it takes the data directory's permissions, so protect that
  directory yourself. The log says where, and never prints the token:

  ```
  WARN no account exists yet: the first administrator is created with the setup token stored in this file path=/data/setup-token
  ```

  With the compose file: `docker compose exec waveflow cat /data/setup-token`;

- or choose it yourself with `WAVEFLOW_SETUP_TOKEN` (16 characters at least),
  and no file is written.

The token stops working, and the file is deleted, as soon as the first account
exists. Creating it from the command line instead
(`waveflow-server account create-admin`) needs no token: that already requires
a shell on the host. A server that finds an account at boot asks for nothing.

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

### A copy before every upgrade

When a new version has migrations to apply, the boot first copies the database
and its key into `pre-migration-<epoch ms>/` in the data directory, in the same
layout as `database backup` — so if the upgrade fails, the error names the copy
and the way back is:

```bash
waveflow-server database restore --input /data/pre-migration-1761350000000
# then start the previous release
```

The newest `WAVEFLOW_MIGRATION_BACKUPS` copies are kept (3 by default) and older
ones made this way are deleted; nothing else in the data directory is touched.
A fresh database, or one already up to date, is never copied. Each copy is the
size of the database: if that is a problem, set `WAVEFLOW_MIGRATION_BACKUPS=0`
and keep backups of your own. A copy that cannot be made — a full disk — stops
the boot before anything is applied. So does a missing or mismatched
`instance.key` beside a database that was bound to one: restore the pair, or
see the commands for a lost key below.

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
| `WAVEFLOW_TRUSTED_PROXIES=10.0.0.2` | The proxy's own address, or a CIDR block such as `172.16.0.0/12` for a Docker network. Without it, every client reaches the server from the proxy's address, and they all share one sign-in budget per account |

### Sign-in attempts are limited per client

Failed sign-ins are counted per client address and account: twenty in a sliding
minute, on the native API, the web client and the Subsonic façade alike. Past
that, the next attempt from that address is refused without being checked
(`429` with `Retry-After: 60`, or Subsonic's error 40) until the minute has aged
out. Another address keeps its own budget, so a stranger guessing at an account
never locks its owner out. API keys are not throttled: they are random 256-bit
secrets. [RFC-017](rfcs/RFC-017-sign-in-hardening.md) carries the reasoning.

Behind a reverse proxy, name it in `WAVEFLOW_TRUSTED_PROXIES` so the server reads
the real client from `X-Forwarded-For`. **List only a proxy that appends the
address it received the connection from** to that header: nginx with
`proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`, Caddy and
Traefik by default. The header is read from the right, past the listed proxies,
so whatever a client writes there itself is never taken for its address. A proxy
that passes a client's header on unchanged would let each guess claim a fresh
address; leave such a proxy out of the list.

Refusals are summarised in the log once a minute at most, with the first
throttled attempt after a quiet minute logged at once. So is the first
`a stored password hash cannot be read` — an account whose stored hash is
damaged answers `503` to every sign-in, and its count in the summary is
`unreadable_hashes`. No line names the account, the address or the key.

### Security headers are set by the server

The web client is served with a Content-Security-Policy, `X-Frame-Options:
SAMEORIGIN`, `X-Content-Type-Options: nosniff` and `Referrer-Policy:
same-origin`; public shares under `/share/` with `Referrer-Policy: no-referrer`,
since their token is the path. If your proxy adds or rewrites these headers,
make sure it does not loosen them — above all, keep framing limited to the
server's own origin. The policy allows
media from any `http:` or `https:` host, because internet radio is played
straight from the station. `/rest` carries none of these headers.

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

## Switching lyrics from LRCLIB on

**Off by default, and an upgrade leaves it off.**
[LRCLIB](https://lrclib.net) is a free lyrics database that needs no key. With
it switched on, a track that has no lyrics of its own — no embedded lyrics, no
`.lrc` or `.txt` beside the file, no uploaded sheet — is looked up the first
time somebody asks for its lyrics. Lyrics from the files and uploads always win;
LRCLIB only fills an empty answer.
[RFC-024](rfcs/RFC-024-lyrics-stats-and-mixes.md) carries the reasoning.

```bash
WAVEFLOW_LYRICS_PROVIDERS=lrclib
```

| Variable | Default | What it does |
| --- | --- | --- |
| `WAVEFLOW_LYRICS_PROVIDERS` | empty | The switch. Empty is off; `lrclib` is the only provider. A name the server does not know refuses to start |
| `WAVEFLOW_LYRICS_TTL_DAYS` | 30 | How long lyrics found are kept before LRCLIB is asked again |
| `WAVEFLOW_LYRICS_MISS_TTL_DAYS` | 7 | How long "LRCLIB has nothing for this track" is kept |
| `WAVEFLOW_LYRICS_RATE_PER_SEC` | 2 | Requests per second |
| `WAVEFLOW_LYRICS_REQUEST_TIMEOUT_SECS` | 10 | How long one request may take before it counts as unanswered |

**It is not the metadata switch.** Naming `lastfm` or `deezer` in
`WAVEFLOW_METADATA_PROVIDERS` does not turn lyrics on, and the reverse: a lyrics
lookup sends a track's title, album and duration at the moment it is played,
which is a different thing to agree to than asking about an artist's name.

**What the server starts contacting.** `lrclib.net`, over https, and only when
somebody opens the lyrics of a track without any — the playing screen, a track's
properties, a Subsonic client calling `getLyricsBySongId` or `getLyrics`. Never
at scan time. Each request carries the artist, the title, the album and the
duration, and a User-Agent naming this project; no redirect is followed, and an
answer over 1 MiB is refused. Simultaneous asks for one track share one request.
The person asking waits at most four seconds; a slower answer is kept for the
next listen. [`privacy.md`](privacy.md) lists exactly what is sent.

**What clients see.** The native API and the web client credit the sheet to
LRCLIB (`source: "lrclib"`); Subsonic clients get the sheet, synced when LRCLIB
has timings, with no credit, because the frozen `structuredLyrics` element has
no field for one.

**What it stores.** In `waveflow.db`, keyed by a digest of what was sent: the
synced and plain lyrics, or that LRCLIB had none or called the track an
instrumental, and when to ask again. A failed request stores nothing, so the
next ask tries again.

**What it logs.** A verdict, such as `answered HTTP 503` — never the request
URL, which carries the title and the artist.

## Switching podcasts on

**Off by default, and an upgrade leaves it off.**
[RFC-025](rfcs/RFC-025-smart-playlists-and-podcasts.md) carries the reasoning;
[privacy](privacy.md) says exactly what is fetched and when.

```bash
WAVEFLOW_PODCASTS=on
```

| Variable | Default | What it does |
| --- | --- | --- |
| `WAVEFLOW_PODCASTS` | off | The switch. Off, no feed is fetched, no episode downloaded, and `podcastRole` is false for everyone |
| `WAVEFLOW_PODCAST_REFRESH_INTERVAL_SECS` | 21600 | How often every channel is fetched again. `0` refreshes only when an administrator asks; anything below 300 is read as 300 |
| `WAVEFLOW_PODCAST_ALLOW_HTTP` | on | `off` refuses plain-http feeds and enclosures. Many enclosures are still served over http |
| `WAVEFLOW_PODCAST_MAX_FEED_BYTES` | 16777216 | The largest feed read |
| `WAVEFLOW_PODCAST_MAX_EPISODE_BYTES` | 1073741824 | The largest episode stored; may not exceed the quota |
| `WAVEFLOW_PODCAST_QUOTA_BYTES` | 21474836480 | What every stored episode may occupy together |
| `WAVEFLOW_PODCAST_FEED_TIMEOUT_SECS` | 20 | One feed fetch, redirects included |
| `WAVEFLOW_PODCAST_DOWNLOAD_TIMEOUT_SECS` | 1800 | One episode download |

**Who manages them.** Administrators: they subscribe, refresh, download and
delete, on the podcasts page or from any Subsonic client, where `podcastRole`
is true for them. Every account reads the channels and plays what is stored.

**What is downloaded, and where.** Nothing on its own: an episode is fetched
when an administrator asks for it, one at a time, into `podcasts/` in the data
directory, named `<episode id>.<format>`. Only audio is kept — the format is
read from the file's first bytes. **Back the directory up with the rest of the
data directory** if you want the episodes back after a restore; otherwise they
can be downloaded again. Deleting an episode deletes its file, and a refresh
does not bring it back.

**A feed that cannot be reached** is marked on its channel with a word —
`unreached`, `http_404`, `address_refused` (it, or a redirect, pointed at a
private address), `invalid_feed` (not RSS or Atom, or it declared a DTD) — and
its episodes stay as they were. **What it logs**: counts and those words,
never a feed address — a private feed carries its token in its path or its
query string. For the same reason only administrators are shown a feed's
address; other accounts see at most its scheme and host.

## Podcast covers and station images

**On by default.** The server fetches the cover a podcast feed names, and the
image an account gives one of its radio stations, into the artwork store
(`artwork/` in the data directory), so the web client and Subsonic clients
show them as they show album covers — the browser never loads them from
their own host. [RFC-025](rfcs/RFC-025-smart-playlists-and-podcasts.md)
carries the decisions; [privacy](privacy.md) says what is fetched and when.

| Variable | Default | What it does |
| --- | --- | --- |
| `WAVEFLOW_REMOTE_ARTWORK` | on | `off` fetches neither: an address is still stored, no image is fetched, and the images already stored stay shown |

**When.** A podcast cover only while `WAVEFLOW_PODCASTS` is on: when a channel
is subscribed, and on a refresh that finds the feed naming a cover it does not
hold — a new address, a failed last attempt, a file gone from the store, or a
copy older than a week. A station image once, when an account sets or changes
it, and only once the station is saved; saving the station again retries one
that failed. An account may have at most ten station images fetched an hour;
past that, a station is saved without its image until a later save. Never on a timer of its
own, never at boot, and never while anybody listens.

**Under which rules.** Those of the podcast fetcher: public addresses only,
checked before connecting and pinned, at most five redirects each checked
before it is followed, 10 MiB, and the bytes must be a JPEG, PNG or WebP —
a page of HTML is refused whatever it is labelled. A podcast cover may be
plain http only while `WAVEFLOW_PODCAST_ALLOW_HTTP` is on; a station image may
always be, as its stream may. **What it logs**: a verdict such as
`image over the size ceiling`, never the address.

**What it stores.** One file per image, `<hash>.<format>`, beside the scanned
covers. The artwork sweep collects it once nothing names it: when the channel
or the station is deleted, or when the feed or the account names another
image.

## Scanning

A scan reads only what changed. A file whose size and modification time are
the ones the catalogue holds, with its `.lrc` and `.txt` lyrics sidecars
unchanged too, is skipped from a `stat` alone, without being opened, so a
rescan of an unchanged library costs a directory walk and not a read of every
byte. The first scan after upgrading to this version reads every file once, to
record what the next ones compare. [RFC-018](rfcs/RFC-018-incremental-scan.md)
carries the reasoning.

A file rewritten in place with both its size and its modification time kept is
therefore not noticed by an ordinary scan. To have every file read again:

```bash
waveflow-server database rescan-full
```

The next scan of every library then skips nothing, and the request is spent.

A folder the scan cannot read, such as wrong permissions or an unmounted share,
is reported in the scan's errors with its path relative to the library, and
the scan goes on. The tracks under it are left as they are, not marked missing,
so a folder that is briefly unreadable does not make its albums vanish and come
back. Only the library root itself failing to open fails the scan.

[RFC-021](rfcs/RFC-021-library-lifecycle-and-scan.md) carries the reasoning
for everything below.

### What a scan leaves out

Some folders are never scanned, whatever else says: anything whose name begins
with a dot (hidden folders and files, `.Trash-*`, the `._` files a Mac leaves
on a share), `@eaDir`, `#recycle` and `#snapshot` (Synology), `$RECYCLE.BIN`
and `System Volume Information` (Windows), and `lost+found`. The one exception
is `.waveflow-managed` at the library root, where received files land.

To leave out more, put a **`.waveflowignore`** file in any folder. Its syntax is
`.gitignore`'s and applies to that folder and below:

```text
# no live recordings, no demos anywhere, no AAC except this one
/Live/
**/demos/
*.m4a
!Keep me.m4a
```

As in git, a folder that is left out is not entered, so a `!` cannot bring back
something inside it.

An **empty** `.waveflowignore` leaves out the whole folder it sits in, as an
empty `.ndignore` does in Navidrome — except at the library root, where it
leaves out nothing. A file the scan cannot use (unreadable, over 64 KiB, or
holding a pattern it rejects) makes its folder count as unreadable: reported in
the scan's errors and left as it is. At the library root, it fails the scan.

**What you leave out is treated as gone.** A track whose file becomes excluded
turns unavailable at the end of the scan, exactly as if the file had been
deleted, and comes back under the same identity when the rule is removed. The
scan's `excluded` counter says how many entries were left out.

### Symbolic links

By default a scan does not follow symbolic links. It no longer skips them in
silence: each one is counted in the scan's `skipped_links` and logged by its
path in the library. To follow them, per library:

```bash
waveflow-server library set-links --actor admin --library-id <uuid> --follow
```

Only a library whose links setting is on follows any link, and a followed link
must resolve **inside that library's root, or inside a folder listed in
`WAVEFLOW_LINK_ROOTS`** (below), and **never into another library's root**. A
link whose target is anywhere else, is broken, or loops back to a folder above
it is skipped and counted. A file inside the root that links lead to is one
track, stored under the file's own path, however many links name it. A link
farm whose files live in the same tree works best with the files' folder
excluded from the direct walk:

```text
/music/.waveflowignore     /store/
/music/store/…             the files
/music/Artist/Album/…      links into ../../store/
```

A link farm whose files live outside the root, such as a beets `link: yes`
import into a separate folder, is followed only into the folders you name in
**`WAVEFLOW_LINK_ROOTS`**. Give absolute paths, separated as `PATH` is (`:`
on Linux, `;` on Windows), for example
`WAVEFLOW_LINK_ROOTS=/srv/beets/library`. The setting applies to every library
whose links setting is on, so name only folders whose files the members of
every such library may hear.

A link that resolves into **another library's root** is skipped, even inside a
named folder. A stored path that has come to resolve there since its scan is
refused when it is played. No link shows one library's files to another
library's members.

The named folders are resolved again at every scan and every stream. A mount
that appears after the server starts is honoured from the next scan. A mount
that disappears no longer resolves: at a scan, links into it are skipped and
counted; when a track stored through one is played, it is refused.

A file reached through such a link is stored under the path the scan reached
it by, and only once, however many links name it. Without the variable, make
the library root a common ancestor of the links and their files, or add the
files' folder as a library of its own.

### Missing files

A track whose file is gone stays in the catalogue, unavailable, with its
favourites, playlist entries and plays, so a share that comes back brings it
back whole. Owners and managers see them on the **Missing files** page, which
says for each what a purge would take; the CLI lists them too:

```bash
waveflow-server library unavailable --actor admin --library-id <uuid>
waveflow-server library purge --actor admin --library-id <uuid> --all
waveflow-server library purge --actor admin --library-id <uuid> --track-id <uuid>
```

**A purge can be taken back for a week.** It takes the tracks out of the
missing files at once and keeps their rows for **`WAVEFLOW_PURGE_GRACE_HOURS`**
(168 by default); until then the page lists them as purged and restores them,
and so does the CLI:

```bash
waveflow-server library restore --actor admin --library-id <uuid> --all
```

A file that comes back within the window restores its track by itself. Once
the window has passed, an hourly pass deletes the tracks and everything that
hangs off them for every member — playlist entries, favourites, ratings, plays,
queue entries, corrections, lyrics — and the albums and artists left with
nothing; each account's sync journal is told what it lost, so a device drops
it at its next pull. A file that comes back afterwards is a new track. Set
`WAVEFLOW_PURGE_GRACE_HOURS=0` to delete at once, as purges did before there
was a way back. Only tracks still missing when the purge runs are taken, and a
purge waits for no scan: it answers that the library is busy instead.

To purge on a schedule, set **`WAVEFLOW_UNAVAILABLE_RETENTION_DAYS`**: once an
hour, tracks missing for longer than that are purged, behind the same grace
window. It is off by default, because a share that stays unmounted for a week
is not a deletion.

### Renaming, moving and deleting a library

From the administration page, or:

```bash
waveflow-server library rename   --actor admin --library-id <uuid> --name "Lossless"
waveflow-server library set-path --actor admin --library-id <uuid> --path /mnt/music
waveflow-server library delete   --actor admin --library-id <uuid> --confirm "Lossless"
```

**Moving** points the library at a new folder and scans it. Tracks are matched
by their path inside the library, so a tree moved intact keeps every identity,
favourite and playlist entry. The owner of a library may rename it; moving it,
its links setting and deleting it are an administrator's.

**Deleting** removes the library from the catalogue for every member — its
tracks, albums and artists with everything that hangs off them, its scans,
folders and memberships — and nothing from the disk. `--confirm` must be the
library's current name. None of these runs while the library is being scanned.

### Watching for changes

The server watches every library folder and scans a library once its changes
have been quiet for `WAVEFLOW_WATCH_DELAY_SECS` (10 by default); a copy that
never pauses is still scanned every thirty delays. The periodic scan stays as
the safety net — **as long as `WAVEFLOW_SCAN_INTERVAL_SECS` is not `0`**, which
turns it off and leaves only the scan at boot. Set `WAVEFLOW_WATCH=false` to
turn watching off everywhere, or switch one library off — from its settings on
the administration page, or with
`waveflow-server library set-watch --actor admin --library-id <uuid> --watch false`.

In a library that does not follow links, a watched change scans **only the
folders it happened in**: the scan reads those folders as a full one would,
ignore files above them included, marks missing only what it walked, and moves
no track it did not look for onto a file it found — a file moved in from a
folder no event named arrives as a new track, and the next full scan settles
the rest. A change at the library's root, a queue overflow, more than 64
folders at once, or a library that follows links, scans the whole library.

**Do not count on it for network mounts.** NFS and SMB report the changes this
machine makes and usually not those another machine makes, so a library on a
share is found up to date at the next periodic scan — and with the interval at
`0`, not before the server restarts. Keep the interval on for any library on a
share. A folder that cannot be watched is logged once and left to the periodic
scan, with the same condition. On Linux, a large library can exhaust the
inotify watch limit; raise it with `sysctl fs.inotify.max_user_watches=524288`
(and in `/etc/sysctl.d/` to keep it), or the library falls back to the periodic
scan.

## Levelling a Subsonic transcode

Most Subsonic clients apply ReplayGain themselves. For those that cannot, set
**`WAVEFLOW_SUBSONIC_REPLAYGAIN`** to `track` or `album` (`off` by default):
a Subsonic `stream` that is transcoded anyway then carries the track's gain,
with the album's or the track's as the fallback and never raising a peak past
full scale. A client may ask for itself with `replayGain=off|track|album` on
`stream`. The original file is never re-encoded for it: a request for the
original is served as it is.

## Offline listening in the web client

The web client registers a service worker, `/sw.js`, that keeps its own shell
and built files so it opens without a network, and the tracks a listener keeps
from a track's menu, which play from the **Offline** page. It never keeps an
API, Subsonic or share answer, a token or a stream ticket's address: a kept
track is fetched once through a ticket and stored under its id, in that
browser only, for the account that kept it: signing out deletes the copies,
and another account signing in on the same browser deletes the previous one's
before it sees them. Nothing needs configuring; a proxy must simply serve `/sw.js`
from the same origin as the client, which it does already.

## Probes

| Path | Answers |
| --- | --- |
| `/health` | The process is up |
| `/ready` | The database is open and migrated |

Logging is `tracing` with `RUST_LOG`; set `WAVEFLOW_LOG_FORMAT=json` in
production. Traces record `uri.path()` only — never headers, query strings,
tokens or passwords — with share tokens and stream tickets redacted.
