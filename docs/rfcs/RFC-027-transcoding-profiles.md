# RFC-027 — Transcoding profiles

- **Status**: Proposed
- **Implemented by**: [#418](https://github.com/InstaZDLL/waveflow-server/pull/418)
  (this RFC and all of it). The *Status* field above never flips in this project;
  this line is the one that says what is running.
- **Date**: 2026-11-02
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the frozen Subsonic
  contract, the one set of domain services, the stream tickets),
  [RFC-007](RFC-007-library-event-stream.md) (the originating device),
  [RFC-022](RFC-022-playback-and-web-client.md) (a format the façade cannot
  produce)
- **Issue**: [#403](https://github.com/InstaZDLL/waveflow-server/issues/403)

## Problem

The format and the bit rate of a stream come from the request and nowhere else.
An account cannot say once "Opus at 128 kbit/s on my phone" for a client that
does not ask, and many do not: such a client is served the original, whatever
the link it is on.

## What each surface knows at request time

Read in the code, because the unit depends on it.

| Surface | Account | Device | Client name |
|---|---|---|---|
| `GET /rest/stream` | yes | no such thing | `c`, required by the protocol, free text |
| `GET /api/v2/tracks/{id}/stream` | yes | only if the caller sends `x-waveflow-device-id`, which is optional and which an API token has no reason to send | no |
| `GET /api/v2/stream/{ticket}` | yes, sealed | no: the ticket seals kind, account, track and expiry | no |

A **device** is also shorter-lived than its name suggests. Every sign-in creates
a new `device` row (`AuthService`, both login paths), so a profile keyed on one
would vanish at the next sign-in, and RFC-007's device exists to let a client
recognise its own writes, not to name a phone across sessions.

## Decision 1 — the unit: the account, then the Subsonic client name

A profile belongs to an account. It has one optional **default** and any number
of **overrides by client name**, the `c` of a Subsonic request.

- **The default** applies to every client that does not ask: the Subsonic façade
  and the native bearer route.
- **An override** applies to a Subsonic request naming that client, and replaces
  the default for it. `Symfonium → Opus 128` alongside a `Feishin → original`
  is what makes "on my phone" true on an account with two clients, and an
  account-only profile cannot say it. The name is compared without ASCII case,
  and it is the client's own claim: it can only ever select among preferences
  the account wrote for itself, so there is nothing to forge.
- **Not the device.** No surface but the bearer route could read it, and only
  when told; it does not outlive a sign-in.
- **The native API has no override.** It has no client name, and its clients are
  versioned with this server and can simply ask.

## Decision 2 — precedence: a profile fills a silence, and nothing else

A profile is consulted only when **the request names neither a format nor a
bit rate**. Anything else is answered exactly as if no profile existed.

| Request | With a profile |
|---|---|
| `format=raw` | the original |
| `format=opus` (any format this server produces) | what it asks for, at its own or the format's default bit rate; the profile's bit rate is not borrowed |
| `format=flac` (a format this server does not produce) | RFC-022 decision 6, unchanged: the client named something precise, and the original it gets today is not ours to take away |
| `maxBitRate=320`, no format | the ceiling rule of RFC-002, unchanged |
| native `bitrate=` without a format | `422`, unchanged |
| `maxBitRate=0`, or `format=` empty | silence: `0` is what the protocol calls no limit, what the façade already reads as no parameter, and what a client that does not ask sends |
| nothing | the profile |

`timeOffset` and `offset_ms` name neither, so a seek inside a profile's transcode
stays under the profile. **An account with no profile takes the path it takes
today**: no row, no change, and the response is byte-identical.

**The profile is a ceiling, like `maxBitRate`.** A source whose known bit rate is
at or below the profile's is served as it is, and an unknown one is transcoded,
which is RFC-002's rule for `maxBitRate`, applied to the same numbers. Otherwise
an MP3 at 96 kbit/s would be re-encoded to a larger file of lower quality, at a
cost in CPU, and would lose byte ranges, for nothing. A profile of `Opus 128` thus
means "never more than 128 kbit/s, and Opus when it has to be re-encoded".

### Where a profile never applies

- **Downloads**: Subsonic `download` and `?download=1`. A download is the file.
- **Stream tickets.** A ticket's URL is played by a media element, which seeks by
  byte range, and a live transcode refuses ranges past the first byte: the web
  player sets `currentTime` and nothing else, and has no use for `offset_ms`. A
  profile applied to it would break seeking and the duration on every first
  play, and could hand Safari a format it may not decode. Tickets also keep their
  URL's meaning across the range requests of one play, which a profile changed
  between two of them could not promise. The web player therefore keeps playing
  originals; a per-browser choice of format, and teaching it to seek a transcode,
  is a change of its own (see *What is still open*).
- **Public shares.** Whoever opens one is not the account.

### The limit this leaves

**The argument that exempts tickets also reaches a client that does not ask and
seeks by byte range**, on the façade and on the bearer route, and a profile applies
to it all the same. Until the encode is whole in the cache, a range past the first
byte is answered `416`, so such a client's seek may fail where it worked on the
original. It is kept this way for two reasons: only an account that set a profile
is concerned, and an override of `raw` for that client puts it back on the
original. The section says so in plain words, with that remedy.

No client has been run against this, so this RFC does not say which are concerned.

### How it meets the rest of the server

- **The cache.** A profile produces a format and a bit rate and joins the path
  they already take, so its encode has the key an explicit request for the same
  pair has. Nothing is encoded twice, and a cached encode answers ranges.
- **ReplayGain.** Rides on the encode as it does today:
  `WAVEFLOW_SUBSONIC_REPLAYGAIN` and `replayGain=` on the façade, nothing on the
  native route.
- **The ceilings.** A profile's transcode takes the same slots as an asked-for
  one, and past them is answered `429` with `Retry-After`. It does **not** fall
  back to the original, for the reason the native guide already gives clients:
  it would replace a bounded cost in CPU with an unbounded one over the very link
  that made transcoding necessary.
- **The status screen (#404).** `active` counts these transcodes like any other,
  and `formats` from `GET /api/v2/transcode/status` is what the section offers.
  `available` is `true` on every server that boots, since one without FFmpeg
  refuses to; the day that changes, a profile has to give way to the original
  there too.
- **A format the server does not know.** The table's `CHECK` holds today's
  vocabulary. A stored value the running binary cannot read (a downgrade) is
  logged and read as no profile: a preference must never be why nothing plays.

## Decision 3 — who sets it: the account

The account itself, through the routes below. **No administrator route.** Because
an explicit request always wins, an administrator's profile could only ever be a
default, never a cap; what an administrator actually bounds is CPU, and the
ceilings already do that. A profile lets an account do nothing it could not already
do by asking. An administrator setting a profile for someone else is additive,
if anyone comes to want it.

## Decision 4 — where it shows: Settings › Playback

A **Transcoding** block on `/settings/playback`, marked as the account's where
the rest of that page is the browser's: the default, then one line per client
name, each a format and a bit rate, and a way back to the original. It says in
plain words that it applies to apps that do not ask, and that this web player
keeps playing originals.

It also says what *The limit this leaves* says, and how to put it right: an app
that fails to seek while a track is being converted gets a line of Original.

A client's `c` is not something a listener knows, so the section offers **the
names this account's Subsonic clients have streamed with without asking, since the
server started**: the clients a profile reaches, which is why a request naming a
format or a cap does not count. They are kept in memory (at most sixteen an
account, the name bounded at the length a device's is). Nothing is written on a
stream, and a restart forgets them at no cost, since a name can also be typed.

## The shape

`transcoding_profile`, one dated migration:

```sql
user_id   TEXT    NOT NULL REFERENCES account(id) ON DELETE CASCADE
client    TEXT    NOT NULL DEFAULT '' COLLATE NOCASE  -- '' is the default
format    TEXT    NOT NULL CHECK (format IN ('raw', 'mp3', 'opus', 'aac'))
bitrate   INTEGER            -- NULL exactly when format is raw, else 32..=512
updated_at INTEGER NOT NULL
PRIMARY KEY (user_id, client)
```

`raw` is there for the override that says "not this client", and is refused for
the default, where it would mean the same as deleting it. At most thirty-two rows
an account, so one account cannot grow the table under the writer gate without
bound.

| Route | Access | |
|---|---|---|
| `GET /api/v2/transcoding-profiles` | read | the rows, and the client names seen |
| `PUT /api/v2/transcoding-profiles` | write | `{client?, format, bitrate?}`, a whole row, created or replaced |
| `DELETE /api/v2/transcoding-profiles?client=` | write | the row; without `client`, the default. `204` whether or not it existed |

A bit rate left out takes the format's default, as a stream request's does, and is
stored as a number, so the screen never shows a value nobody set.

Every surface goes through **one** `DomainServices` method, which takes the
account, the client name if any and the source's bit rate, and answers the format
and bit rate to use, or nothing. Each surface decides only whether the request was
silent, which only it can tell. Not journalled: no client replicates this, and
the server reads it on every silent stream.

## What does not change

- Every response for a request that names a format or a bit rate, and every
  response to an account with no profile.
- The stream tickets, public shares, downloads, the cache's keys, the ceilings.
- The Subsonic wire: no new parameter, no new method, no new field.
  `docs/compatibility/subsonic-compatibility.md` records that a silent request
  may now be transcoded for an account that asked for it, and *The limit this
  leaves*.

## The order of the work

One pull request after this RFC, with tests that each fail without their fix:

- the frozen contract: an explicit `format` and an explicit `maxBitRate` each beat
  a profile on both surfaces, `format=raw` and `format=flac` included, and no
  profile changes nothing;
- the resolution: override over default, an override of `raw`, a source at or under
  the ceiling, an unknown source bit rate, a stored format the binary cannot read;
- downloads and stream tickets untouched by a profile, and a profile's transcode
  refused with `429` past the ceilings;
- the routes: validation, the row limit, an account reading only its own rows;
- `migrations`: the new table on a database at the last released schema.

## What is still open

- **The web player.** A per-browser format, sent as an explicit `format=` on the
  ticket URL and seeking through `offset_ms`, would cover a phone listening through
  the installed client. Not done here because it is a change to the player, and a
  local preference rather than an account's.
- **An administrator's route**, as decision 3 says, if asked for.
- **A native client name.** If a native client ever wants an override, it would
  send a name the way Subsonic does; nothing here prevents it.
