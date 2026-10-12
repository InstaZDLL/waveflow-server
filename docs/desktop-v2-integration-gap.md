# WaveFlow Desktop ↔ server v2: a historical record of the gap, now closed

> **Final status (2026-08-18): the gap is closed.** The Desktop `/api/v2`
> integration with PKCE was validated end to end, and then the conservative M5
> reconciliation was merged. This document remains a historical record of the
> migration decisions; it is no longer a list of work. See the
> [final handoff](M4-handoff.md) and [RFC-004](rfcs/RFC-004-local-server-reconciliation.md).

> Survey of 2026-08-10, taken read-only against
> [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) at `802f189b`.
> It describes what Desktop expected on that date, what server v2 offered, and
> what had no equivalent. No code had been modified during the survey. The
> target contract is [RFC-003](rfcs/RFC-003-waveflow-sync-v2.md).

## What was found on 2026-08-10

Desktop spoke entirely to server **v1**, which no longer existed. The six routes
it consumed had zero occurrences in the v2 server code, and its sign-in flow
relied on a web front end that PR #84 had removed.

So this was not an adaptation of the synchronization layer: it was the
replacement of a bidirectional protocol with logical clocks by a linear journal
the server is the authority for.

## The surface involved

Roughly 6,000 lines of Rust, spread as follows:

| Module | Lines | Expected fate |
| --- | --- | --- |
| `sync/digest/{mod,client,entity_client}.rs` | 1,178 | Deleted — replaced by `snapshot` |
| `sync/backfill/{mod,pull,lww}.rs` | 1,349 | Deleted — LWW resolution has no purpose left |
| `sync/ws.rs` | 557 | Rewritten — the socket becomes a mere wake-up |
| `sync/queue.rs` | 573 | Kept, re-tooled on `X-WaveFlow-Operation-Id` |
| `sync/drain.rs` | 543 | Rewritten — no more generic ops endpoint |
| `commands/sync.rs` | 624 | Rewritten |
| `server_client.rs` | 357 | Rewritten — base URL, headers, auth |
| `commands/server_auth.rs` | 227 | Rewritten — PKCE |
| `sync_stub.rs` | 368 | To be reassessed |
| `sync/{cursor,hooks,canonical,track_snapshots}.rs` | ~640 | Kept, adapted |
| `commands/share.rs` | 292 | Rewritten onto `/api/v2/shares` |

Plus two TypeScript files: `src/lib/tauri/serverAuth.ts` and
`src/components/views/settings/ServerAccountCard.tsx`.

## Route correspondence

| Desktop on 2026-08-10 | Server v2 | Nature |
| --- | --- | --- |
| `GET /api/v1/sync/ops?since=N` | `GET /api/v2/sync/changes?after=N&limit=` | Renamed + explicit pagination (`next_cursor`, `has_more`) |
| `POST /api/v1/sync/ops` | *(none)* | **Gone.** Mutations go through the domain REST endpoints, which carry `X-WaveFlow-Operation-Id` |
| `POST /api/v1/sync/ack` `{device_id, last_seen_id}` | `PUT /api/v2/sync/ack` `{device_id, cursor}` | Verb and fields changed |
| A WS pushing ops | `GET /api/v2/sync/socket?after=` → `{"cursor": N}` | **Semantics inverted**: a notification, never a transport for state |
| `GET /api/v1/sync/digest` | *(none)* | **Gone** — `GET /api/v2/sync/snapshot` replaces fingerprint reconciliation |
| `GET /api/v1/sync/entity` | *(none)* | **Gone** — the content is carried by `snapshot` / `changes` |
| `GET /api/v1/profiles` | *(none)* | **Gone** — see "the profile model" |
| `POST /api/v1/share/playlists/by-canonical/{p}/{pl}` | `POST /api/v2/shares` | An entirely different sharing model |

## What becomes purposeless

These mechanisms exist because v1 accepted concurrent writes arbitrated on the
client. Server v2 is the authority and its journal is strictly ordered: keeping
them would be carrying a complexity the protocol no longer asks for.

- **Logical clocks.** `sync/lamport` and `sync/hlc` stamp every outgoing op for
  a global order between peers. The v2 journal's cursor supplies that order on
  its own.
- **Fingerprint reconciliation.** `sync/digest` compares per-entity digests to
  detect a divergence. `GET /sync/snapshot` returns a writer-side consistent
  representation that the client replaces atomically.
- **LWW backfill.** `sync/backfill/lww.rs` arbitrates conflicts by last writer.
  In v2 a conflict is not arbitrated on the client: reusing an `operation_id`
  with a different fingerprint is **rejected as a conflict**, and a client that
  cannot apply a known event throws away its projection and takes a snapshot
  again.
- **Compaction and `410 Gone`.** `catchup_pull` handles a server that has
  compacted past the cursor. The v2.0 journal is append-only; RFC-003 makes any
  future retention conditional on a snapshot floor. That path can go, but
  **replace it with a snapshot restart** rather than removing it with nothing in
  its place.

## What had to stay in the new architecture

- **`device_id`.** Still central: the server refuses a device belonging to
  another account, and the ACK is per device.
- **The local queue.** Writing offline is still necessary. What changes is its
  content: no longer generic "ops" but replayable domain REST calls, each
  carrying a stable `operation_id`.
- **The local ↔ server identifier mapping.** `sync/canonical.rs` and the
  `sync_id_map` table keep their purpose: v2 public identifiers are UUIDs, and
  Desktop works on local rowids.
- **Cursor, inbound application, reconnection.** Kept, adapted to the new
  shapes.

## Decisions recorded (2026-08-10)

These points were open at the time of the survey. They are settled, and each was
checked against the server code: none of them asks for a server-side addition.

### 0. Desktop is a universal client

**Decision (user).** Desktop must be able to connect to any Subsonic server —
Navidrome, Airsonic, Gonic — and not only to WaveFlow. That choice governs all
the others: it requires a "remote source" abstraction with two implementations
instead of one v2 client.

Two distinct interfaces rather than one, so that synchronization stays a
capability and not the client's mandatory foundation:

```text
   MusicServer (mandatory)              SyncProvider (optional)
   catalogue, search, playback,         snapshot, changes, ack, socket
   user data where supported
          │                                      │
   ┌──────┴────────┐                             │
SubsonicSource   WaveflowSource ─────────────────┘
  /rest/*          /api/v2/*  (native end to end)
(Navidrome,      (never /rest/: WaveFlow's Subsonic
 Airsonic…)       facade is there for third-party clients)
```

**The rule, decided by the user.** Between WaveFlow Desktop and WaveFlow Server
the traffic goes **always** through the native `/api/v2` layer — including for
the catalogue, search and playback, not only for synchronization. The server's
Subsonic facade exists for third-party players (Symfonium, DSub, Substreamer,
Feishin); Desktop only borrows it when facing a server *other* than WaveFlow.

`WaveflowSource` is therefore not a `SubsonicSource` with sync added: it is an
independent implementation. Deriving it from the Subsonic branch would mean
consuming `/rest/` against our own server and losing three things already won:

- **idempotence** — the v2 routes read `X-WaveFlow-Operation-Id`, the Subsonic
  facade does not;
- **relevance** — both surfaces now go through FTS5 (`catalog_search` for
  `search3`), but only `/api/v2` returns paged projections rather than a
  `searchResult3` capped by separate counters;
- **native pagination** and the typed projections of `/api/v2`.

The common denominator is wide: the Subsonic facade covers **every** mutation
Desktop needs — `star`/`unstar`, `setRating`, playlists in full CRUD,
`scrobble`, `savePlayQueue`/`getPlayQueue` and shares.

**Detection.** Every Subsonic response from WaveFlow carries `type="waveflow"`,
`serverVersion` and `openSubsonic="true"` (`src/subsonic.rs`). One `ping` is
therefore enough to decide whether `SyncProvider` can be switched on.

> **A trap.** `getOpenSubsonicExtensions` now announces `formPost`,
> `apiKeyAuthentication` and `transcodeOffset` — but it will **never** say that
> the native v2 API exists: that is not an OpenSubsonic extension. And it
> requires an authenticated call, whereas the decision to switch on
> `SyncProvider` is taken before there is any credential. The discriminator
> stays `type="waveflow"`.

**What only WaveFlow offers**, and which must therefore be treated as an
optional capability and not as a prerequisite: the journal (`/sync/snapshot`,
`/sync/changes`), the per-device ACK, the wake-up WebSocket, and above all the
idempotence of mutations.

**The identifier model.** Remote identifiers are **opaque strings**, never typed
UUIDs. WaveFlow serialises its UUIDs (`id.to_string()`), which makes its two
surfaces interchangeable with no mapping table — but Navidrome and the rest emit
textual identifiers of another shape. Desktop must therefore:

- treat every remote identifier as a `String`, without parsing it into a `Uuid`;
- index locally on the composite key `(profile_id, remote_id)`, since two
  servers can emit the same identifier with nothing in common;
- keep the catalogue cache distinct from synchronized state, one being
  rebuildable and the other not;
- never assume a capability is present without having observed it.

**The consequence for idempotence.** `src/subsonic.rs` never reads
`X-WaveFlow-Operation-Id`: those headers exist only on the v2 routes. A Subsonic
mutation is therefore **not safely replayable**. If the response is lost, the
client does not know whether the server applied it: replaying duplicates the
scrobble or the playlist. The two implementations do not offer the same offline
guarantee, and the queue has to know it:

- `WaveflowSource` — safe replay by `operation_id`; the queue may re-emit
  freely. That is the direct reason for the rule above: going through `/rest/`
  against our own server would mean deliberately giving up that guarantee.
- `SubsonicSource` — no blind replay. After a lost response, re-read the state
  before deciding, or accept the duplicate for entities that are idempotent by
  nature (`star`, `setRating`) and abstain for those that are not (`scrobble`,
  `createPlaylist`).

### 1. One Desktop profile = one server account

The profile stays the **local** unit that separates accounts, tokens, mappings
and cursors. Its synchronization identity becomes the `account_id` the server
supplies; `profile_canonical_id` ceases to exist in the protocol.

```text
Desktop                          Server
└── Profile                      └── Account
    ├── server_url                   ├── Device A
    ├── remote_identity ────────────▶├── Device B
    ├── auth (tokens | u/p)          ├── Library 1
    ├── sync state (if WaveFlow)     └── Library 2
    └── active_library_id (optional)
```

The remote identity is **polymorphic**, since not every server has the same
account model:

```rust
enum RemoteIdentity {
    Waveflow { account_id: Uuid, device_id: Uuid, cursor: u64 },
    Subsonic { username: String },
}
```

A third-party Subsonic server has no UUID account identifier, no notion of a
device and no cursor: those three fields exist only in the WaveFlow branch.
Wiring them into the common structure would force them to be optional everywhere
and spread `unwrap`s over cases that cannot happen.

One profile per library was set aside: a library is a content resource, a profile
is an identity and a session. An account giving access to several libraries would
otherwise produce artificial profiles for one and the same user.

**Nothing to add on the server.** `POST /api/v2/auth/login` already returns the
account's `id` and the `device_id` (`WebAuthResponse`), and the server infers the
user from the Bearer alone. `active_library_id` corresponds to the
`library_id: Option<Uuid>` parameter the catalogue projections already accept
(`list_albums`, `list_artists`): an existing filter, not a notion to introduce.

### 2. Authorization Code + PKCE over loopback

The current flow opens `<web-url>/desktop-login?cb=…` and waits for a Better
Auth JWT in the query string; the target is PKCE (S256) on
`/api/v2/oauth/authorize` and `/api/v2/oauth/token`. Desktop already has a
loopback listener and a randomness generator (the Spotify path): the machinery
exists, the protocol changes.

Careful: a v2 code is **spent on its first presentation**, whatever the outcome.
A wrong verifier burns the code and forces the flow to be restarted; that is not
a bug to work around by trying again with the same code.

PKCE applies only to the WaveFlow branch. A third-party Subsonic server
authenticates by `u/p`, `u/t/s` or `apiKey` depending on what it accepts: that is
a second form of authentication to carry, not a degradation of the first.

### 3. A direct Bearer for playback, no ticket

Desktop reads through `GET /api/v2/tracks/{track_id}/stream` with
`Authorization: Bearer`. Verified in `src/media.rs`: that handler requires the
Bearer and accepts `format`, `bitrate` and `offset_ms`, with 206/416 on
`Range`. Sealed tickets stay reserved for consumers that cannot carry a
header — `<audio src>` in the browser.

Against a third-party server, playback goes through `/rest/stream` with the
Subsonic credentials. `MusicServer` therefore exposes a stream URL (and the
header to attach, if any), not a hard-coded route.

## Design constraints on the queue

The local queue no longer holds abstract ops but replayable domain mutations. A
typed shape is preferable to serialised HTTP requests:

```rust
enum Mutation {
    AddFavorite { operation_id: Uuid, track_id: Uuid },
    SetRating   { operation_id: Uuid, track_id: Uuid, rating: u8 },
    UpdatePlaylist { operation_id: Uuid, playlist_id: Uuid, /* … */ },
}
```

Three properties of the server constrain that queue, and ignoring them would
produce failures in perfectly ordinary cases:

- **A queued entry is immutable.** The server stores a canonical fingerprint of
  the action, the target resource and the normalised payload. Reusing an
  `operation_id` with a different fingerprint is **rejected as a conflict**, not
  treated as a replay:

  ```rust
  if record.intent_hash.as_deref() != Some(intent.as_bytes()) {
      return Err(SyncError::Conflict);
  }
  ```

  So if the user corrects their action before the queue drains — renaming a
  playlist that is already queued, say — a second mutation has to be emitted
  with a **new** `operation_id`, or the two merged before queueing by
  regenerating the identifier. Mutating a pending entry while keeping its
  `operation_id` breaks.

- **A replayed share returns the same URL.** `derive_share_token(id)` is
  deterministic (keyed BLAKE3 over the instance key and the UUID). Share
  creation therefore replays like any other mutation, with no special handling
  and without forging a second URL.

- **The ACK is not a prerequisite.** RFC-003 makes it a signal for
  observability and future retention, not a condition for reading the next
  pages. A failed ACK must not block synchronization.

## The target sequence

```text
LOGIN ──▶ account_id + device_id
   │
BOOTSTRAP ──▶ GET /sync/snapshot ──▶ atomic replacement ──▶ cursor ──▶ ACK
   │
SYNC ──▶ GET /sync/changes?after=cursor ──▶ apply ──▶ cursor ──▶ ACK
   │
WS {"cursor": N} ──▶ if N > the local cursor ──▶ GET /sync/changes?after=cursor
```

Upstream: an optimistic local write, queueing, then the call to the domain
endpoint with `X-WaveFlow-Operation-Id` (and `X-WaveFlow-Device-Id`). The server
applies it or recognises the replay; the change then comes back through
`/changes`.

## A warning

**RFC name collision.** Desktop has its own
`docs/rfcs/RFC-003-sync-architecture.md` (hybrid logical clocks, status *Draft*,
2026-06-12), unrelated to the server's `RFC-003` (sync v2, *accepted*,
2026-08-09). The comments in `sync/mod.rs` refer to the **desktop** RFC-003. Any
instruction mentioning "RFC-003" must name the repository, or it will be read the
wrong way.

## A reminder of scope

RFC-003 holds that the server catalogue is a **separate remote source**: the
protocol synchronizes only the state the account owns, never imports a server
track into the local catalogue, and never guesses a local/server correspondence.
Reconciliation is **M5** and requires its own RFC — automatic linking on a
unique full hash only, MBID as a suggestion to confirm, and no fuzzy matching by
title, artist or duration.
