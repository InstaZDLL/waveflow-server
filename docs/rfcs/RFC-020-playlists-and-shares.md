# RFC-020 — Playlists and shares: repeats, public playlists, and the share page

- **Status**: Proposed
- **Implemented by**: [#327](https://github.com/InstaZDLL/waveflow-server/pull/327)
  (this RFC and its implementation, together), and [#335](https://github.com/InstaZDLL/waveflow-server/pull/335) (the move of a
  playlist entry, removal in the web client, `getPlaylists?username=` and the
  share page's covers, closing *What is still open* but one). The *Status* field above never
  flips in this project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (tenancy in the
  queries, the frozen Subsonic contract, stream tickets)

## Problem

A comparative audit against Navidrome, Ampache and the Subsonic clients
(2026-10-24) reported six defects in playlists and public shares. Each was
reproduced by a failing test before anything was changed, and one of them
was worse than reported:

1. **A playlist cannot hold a track twice.** `playlist_track` is keyed on
   `(playlist_id, track_id)`. Adding a track already present violates the key,
   which surfaces as `ServiceError::Database` and answers **503** to an
   ordinary edit. The audit said that one request naming a track twice
   answers 404; measured, it answers 503 too, because `songs_by_ids_lenient_on`
   returns one song per id *asked*, duplicates included, so the length check
   passes and the insert is what fails. An M3U import reports the second copy
   as `duplicate` and drops it, so a playlist with a repeat exports and
   re-imports shorter.
2. **The `public` flag does nothing.** Every read filters on
   `owner_user_id=?`, so no other account ever sees a playlist marked public,
   although Subsonic's `updatePlaylist` accepts and stores the flag.
3. **The share link opens a JSON document.** `/share/{token}` answers JSON to
   everyone, and no web route renders it, so the person the link was sent to
   sees identifiers where a player should be.
4. **A visit is counted per stream request, under the writer gate.** Every
   stream request on a share — each range request a seek makes included —
   called `public_share`, which incremented `visit_count` under the
   process-wide writer gate. One listen with three seeks counted four visits,
   and every one of those anonymous requests queued behind any scan.
5. **An anonymous visitor chooses the transcode.** The share stream route
   took the whole `StreamQuery`: any format, any bitrate, a live encode from
   any offset, each one an FFmpeg process run on the owner's account.
6. **A share cannot be limited to a number of visits.** Ampache has
   `max_counter`; this server had nothing.

## Decision 1 — a list holds what it is given, keyed on the position

`playlist_track` and `share_track` are rebuilt, keyed on `(list, position)`,
the shape `play_queue_track` was given in `20260809150000` for the same
reason. Shares are included although the audit named only playlists: the web
client shares the play queue, which may repeat a track, and creating that
share failed with the same 503.

- **The rebuild copies the rows as they are.** Positions were already unique
  per list, so the new key cannot refuse one, and the order is the stored
  order. `tests/migrations.rs` seeds each released schema with a playlist and
  a share whose rows are stored against their insertion order, boots, reads
  them back through the services, then appends a repeat.
- **Every surface follows**, because they all write through the one service:
  native creation and `add`, Subsonic `createPlaylist` (`songId` repeated) and
  `updatePlaylist` (`songIdToAdd` repeated), and the M3U import, which now
  places every copy. The `duplicate` reason is no longer produced; it stays in
  the import report's schema so that a client generated against it still reads
  every value it was told to expect.
- **A position is the position the caller was shown.** With repeats, a
  position is the only way to say which copy goes, and the list a reader is
  shown is the stored list without the tracks it cannot read (a file gone from
  disk, a library it left). `remove_indexes` and `songIndexToRemove` used to be
  applied to the stored list, so an index past a hidden entry removed its
  neighbour. They are now mapped onto the shown list; the hidden entries are
  kept, for when the track comes back.

## Decision 2 — a public playlist is readable by every account, through its own libraries

- **Who sees it.** Every account on this server, as Navidrome and the
  Subsonic specification have it ("all playlists a user is allowed to play").
  A playlist whose owner is disabled is not listed: a disabled account
  publishes nothing.
- **What they see of it.** Its name, comment, owner, dates, and the tracks
  *they* can read. The tracks are resolved through the viewer's
  `library_member` rows by the same projection every other read uses, so a
  track in a library the viewer is not a member of is not shown, not counted
  in `songCount` or `duration`, and not named anywhere in the answer. An
  account with no library in common with the owner sees the playlist and none
  of its tracks. What remains observable is that the owner changed *something*:
  `changed` moves on any edit. That is the playlist's own metadata, which the
  owner chose to publish.
- **Read-only to everyone else.** Every write reaches the owner's playlists
  only, in SQL; a non-owner's update, deletion, or `createPlaylist` with a
  `playlistId` answers exactly as a missing playlist does — 404 natively,
  error 70 in Subsonic.
- **Native.** `GET /api/v2/playlists` stays the caller's own: the desktop
  client lists playlists to mirror and edit them, and handing it someone
  else's would offer an edit that answers 404. `?include_public=true` adds the
  others after the caller's own. `GET /api/v2/playlists/{id}` and the `.m3u8`
  export answer a public playlist to anyone, read through the reader's
  libraries. Every playlist now carries `owner_id` and `owner_username`. The
  synchronization snapshot holds the caller's own playlists only.
- **Subsonic.** `getPlaylists` answers the caller's playlists, then everyone
  else's public ones, newest first; `getPlaylist` answers a public one. `owner`
  is the owner's username — which is what Feishin reads to decide whether a
  playlist is editable — and `public` is the stored flag. This is a change to
  frozen wire behaviour, made because the flag the façade accepted had no
  effect anywhere, and recorded in `docs/subsonic-compatibility.md`. The
  `username` parameter of `getPlaylists` stays ignored.

## Decision 3 — the share link opens a player, by content negotiation

`/share/{token}` is the address Subsonic's `createShare` has been handing out,
so it is the address a recipient opens; a different route would leave every
link already sent showing JSON. The same address therefore answers two
representations:

- **A request whose `Accept` names `text/html`** — what a browser sends when
  someone clicks a link — is handed the embedded web client, which renders a
  public player at `/share/$token`, outside the signed-in shell. `*/*`,
  `application/json` and no `Accept` at all keep the JSON, byte for byte the
  shape they had, so no existing consumer changes. A share that cannot be
  opened answers the same page with **404**.
- **Not a server-rendered page.** It would be a second player, a second set
  of strings to translate and a second stylesheet, outside the client that
  already has all three.
- **Handing out the page counts nothing.** The client then asks the same
  address for the JSON, and *that* is the visit. A link preview fetching the
  page to draw a card has listened to nothing, and must not spend a capped
  share's visits. A `HEAD` — what uptime monitors and previewers send, usually
  with no `Accept` — is answered as the page is, status included, and counts
  nothing either.
- **`share/` stays reserved** in the client fallback (`src/webui.rs`).
  Nothing under it falls through to the client by accident; the one address
  that answers the client is chosen by its own route.
- Both answers carry `Vary: Accept`, `Cache-Control: no-store` and
  `Referrer-Policy: no-referrer`: the address is the credential, and the page
  must not hand it to whatever the browser fetches next.
- The routes move out of the Subsonic façade into `src/public_share.rs`. They
  were never Subsonic methods; their paths are unchanged.

## Decision 4 — a visit is one opening, and streaming writes nothing

- **The opening counts.** `GET /share/{token}` for JSON counts the visit in
  one conditional `UPDATE … RETURNING` under the writer gate. An expired,
  deleted or capped share matches no row, so the count and the check cannot
  disagree, and two visitors racing for a share's last visit are serialised by
  the gate: the second finds the cap reached. Because it waits for the gate,
  the opening sits under the ordinary request timeout and answers 408 rather
  than holding its connection for the length of a scan; the stream route stays
  outside it, as every media route does.
- **The opening hands out one pass per track.** Each `streamUrl` carries
  `?visit=`, a stream ticket of a third kind, `ShareVisit`, sealing
  `(share, track, expiry)` under the instance key. Its first identifier is the
  share, not an account: a ticket naming the owner would open the owner's
  catalogue on the routes that redeem the other kinds, and every route asserts
  the kind it serves.
- **The stream route redeems the pass and writes nothing.** It verifies the
  pass, then re-reads the share — still present, not expired, holding that
  track — in one read outside the writer gate, then checks the owner may still
  read the track. Deleting or expiring the share therefore stops the music at
  once, whatever the pass says. The cap is not asked again: the visit that
  reached it was counted, and keeps its music.
- **A pass lasts twelve hours.** A stream ticket lasts an hour because it
  covers one track while seeking; a pass covers a sitting, and a share holds up
  to four hundred tracks. It is not what keeps a share closed — the share is
  read on every request — so all it bounds is how long one counted visit plays.
- **A stream URL built by hand, without a pass, answers 404.** This changes
  the stream addresses a JSON consumer receives, and the consumers that follow
  `streamUrl` from the document — every one this project knows of, its own
  web client included — keep working unchanged. A consumer that assembled the
  address itself was relying on what made the count meaningless.
- A unit test holds the writer gate the way a scan does, asserts that opening
  the share waits for it — which proves the test can see the gate — and that
  the stream lookup does not.

## Decision 5 — an anonymous visitor plays the original or one fixed MP3

The share stream route takes two parameters and refuses any other with 400:

- **`visit`**, the pass;
- **`format`**: absent or `raw` for the original file, `mp3` for MP3 at
  192 kbit/s — the server's default MP3 profile, so the cache entry it fills is
  the one every other default MP3 request of the file already uses, and a
  second visitor costs no encode.

There is no bitrate, no Opus and no offset: a live encode from an arbitrary
offset is a new FFmpeg process per seek, and the visitor is anonymous. Seeking
works by byte range on the original, and on the MP3 once its cache entry is
complete. The encode still runs on the owner's per-account transcode slots, as
before, and is refused with 429 when they are full. The share page plays the
original when the browser says it can (`canPlayType` on the track's content
type), and the MP3 otherwise.

## Decision 6 — an optional visit cap

- `share.max_visits`, nullable, at least 1, `NULL` for no cap. Existing shares
  have none.
- **Enforced where the visit is counted**, in the same statement (decision 4),
  so a capped share answers 404 to its next opening, exactly as an expired one
  does.
- **Set natively** on `POST /api/v2/shares` and `PATCH /api/v2/shares/{id}`
  (`max_visits`), lifted by `clear: ["max_visits"]`, answered on every share
  read, and carried in the synchronization journal's share payload. Zero or a
  negative number answers 422. Lowering the cap to the count already reached
  closes the share at once.
- **Not in Subsonic.** `createShare` and `updateShare` carry no such parameter
  in the specification, and adding one to a frozen contract for a single
  server is not worth the drift; `getShares` is unchanged.
- **Set in the web client** when a share is created, and shown beside its
  visit count.

## What does not change

- The Subsonic share methods, and the shape of the share JSON document.
- How a share is created, its token, and the hash-only storage of it.
- The sync contract of RFC-003: playlist and share payloads gain fields, and
  lose none.

> **Amended 2026-10-28, after the third client campaign: two Subsonic share
> methods changed, and the share page.** The first item above no longer holds
> as written.
>
> - **`getShares` and `updateShare` hand the owner the share's `url`.** It was
>   given at creation and on an idempotent replay only, on the reasoning that
>   only the token's hash is stored. That reasoning never applied to the
>   token's derivation: `derive_share_token(id)` is a keyed hash of the
>   instance key and the share's id, so the server can derive it again on any
>   read. The schema makes `url` required on a `share`, Navidrome always sends
>   it, and DSub 5.5.3 crashes on its share screen without it (a
>   `NullPointerException` in `Share.getName`). The owner already reads every
>   track the link plays, so handing them the link grants nothing new; nobody
>   else is ever handed it, since no Subsonic method lists another account's
>   shares. The native `GET /api/v2/shares` is unchanged and still omits it.
>   After a key rotation, the link derived is the share's new address, because
>   the rotation re-addresses every share under the new key.
> - **`createShare` takes an album's id.** The specification takes the id "of
>   a song, album or video"; an album answered error 70. An album now expands
>   into its available tracks, in the album's order (disc, then track), beside
>   any track named in the same request, in the order the ids were given. An
>   id that names neither, or an album of a library the caller cannot read,
>   is still 70; more than 400 tracks once expanded is error 10, as too many
>   tracks were. The expansion sits in `DomainServices`, ahead of the one
>   share creation every surface calls; the native route still takes track
>   ids only.
> - **The share page's own play button plays.** The page's `<audio>` had no
>   source until a row was touched, so its play button answered "Unable to
>   play media" at 0:00 / 0:00 in Chrome on Android. It now holds the first
>   row's address before anything is chosen, with `preload="none"` and no
>   `autoplay`, so opening the page still fetches and plays nothing, and
>   pressing it plays what touching the first row plays.
>
> Recorded in `docs/subsonic-compatibility.md`.

## Logging

Nothing new is logged per visit or per stream. The traces keep recording
`/share/{redacted}` for every path under `/share/`, so neither the token nor
the pass — which sits in the query string, never recorded — reaches a sink.

## The order of the work

One pull request, with this RFC: the migration, the services, the native and
Subsonic surfaces, the share page in the web client, the visit cap in the
shares page, and a test for each decision, each shown to fail without its fix.

## What is still open

Four of the items this section listed were closed by the follow-up pull
request named in *Implemented by*; what was decided is recorded here, beside
the one item left.

- **Reordering a playlist — done.** `POST /api/v2/playlists/{id}/move` takes
  `from` and `to`, positions in the list the caller is shown, as removal
  does (decision 1), and optionally `track_id`, the track the caller believes
  sits at `from`: when it does not, the move answers 409 rather than moving
  another entry of a list edited elsewhere. A hidden entry keeps its place
  among the others, one copy of a repeated track moves and the other stays,
  and the move journals the playlist's whole new order, as an update does.
  Owners only: anyone else is answered 404. **Subsonic has no move verb**, and
  none is added to a frozen contract: a client reorders by sending the whole
  list again through `createPlaylist?playlistId=`, which replaces the track
  list in the order given, as it always has — a test now holds that.
- **Removing one entry from the web client — done.** A playlist's owner has a
  control per row to move it up, move it down, or remove it, each sending the
  position shown and the track the page believes is there; another account's
  public playlist shows none. The removal goes through
  `POST /api/v2/playlists/{id}/remove` (`position`, optional `track_id`),
  guarded as a move is — 409 for a stale view — where an update's
  `remove_indexes` carry no such guard. The page holds every entry control
  until the edited list has been read again, so a second click never names a
  position in a list the first one changed.
- **`getPlaylists?username=` — done.** For an active administrator naming
  another account: that account's own playlists, private ones included, read
  through the **administrator's** libraries, so a track in a library the
  administrator is not a member of is neither listed nor counted; and
  `getPlaylist` opens what that listing named. Anyone else naming another
  account, and anyone naming an account that does not exist, is answered
  error 70 — a 50 would confirm the name belongs to someone. Naming oneself is
  the ordinary listing, and every write stays the owner's. Recorded in
  `docs/subsonic-compatibility.md`.
- **Artwork on the share page — done.** The share's JSON gives each track that
  has a cover a `coverUrl`, `/share/{token}/tracks/{id}/cover?visit=<pass>`,
  authorised exactly as the stream is: by the pass the counted opening handed
  out for that track, the share read again on every request, so a deleted or
  expired share stops answering at once and a capped one keeps the covers of
  the visit that was counted. It answers that one cover — the track's own, or
  its album's — read through the owner's libraries, `private, no-cache` with
  the hash as validator, counts nothing and writes nothing. The artwork routes
  stay session-only.
- **Shares of a disabled owner** still open. Disabling an account should
  arguably close its shares as it now hides its public playlists; that is a
  decision about accounts rather than about shares, and it is the hardening
  pull request's.
