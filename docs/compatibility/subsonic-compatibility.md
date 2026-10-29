# Subsonic compatibility matrix — WaveFlow Server v2

Automated protocol coverage is enforced by the `subsonic_contract`, `subsonic_browse`, `subsonic_fields` and `subsonic_methods` targets under `tests/` for XML, JSON, `.view`, GET, form POST, `u/p`, `u/t/s`, `apiKey`, catalogue isolation, all documented mutations, media and artwork.

> **The third client campaign ran on 2026-10-27 and 2026-10-28**, the first
> since 2026-09-16, with Juliet 8, Feishin 1.17.0, Symfonium 15.1.0 and DSub
> 5.5.3. It ran against `main` from `0814958` (#367) to `52e3114` (#368), on a
> 22-track catalogue kept from August and extended with traps for what had
> landed since: RFC-017 to RFC-025, with podcasts, internet radio, smart and
> public playlists, AAC, server-side ReplayGain and lyrics, and the
> restructuring of the code (#351–#366). None of that had met a real client
> before. Every server defect the campaign found is now fixed:
> - the shape of an unsynced lyrics line in JSON (#367);
> - a seek probed by iOS (#368);
> - writes on a removed track (#370);
> - an album still found by a removed track's title (#371);
> - album-less tracks missing from `getIndexes` (#372);
> - three share defects (#373);
> - OpenSubsonic fields sent to DSub (#374), and the media type of its errors
>   (#375).
>
> #377 fixed one more, found while fixing #371 and reached by no client: an
> album whose every file went. DSub, which surfaced most of them, was replayed
> against the fixes the same day and passed every point. The sections below
> record each defect, and the rows record the runs. Substreamer is still not
> part of the set: that build no longer installs on the current Android device.
>
> **What the earlier notes asked this campaign to check, and what it found.**
>
> - **`getTopSongs` answers from local play history** (#215). Feishin called it
>   six times in JSON, and DSub in XML; both rendered the list. A single child
>   decodes as a list, so `topSongs.song` is an array at any length.
> - **`getSimilarSongs` and `getSimilarSongs2` answer without a metadata
>   provider** ([RFC-024](../rfcs/RFC-024-lyrics-stats-and-mixes.md)). Feishin,
>   DSub and Juliet (`getSimilarSongs2`) rendered what they were given.
>   `getArtistInfo` stays empty by default; Feishin's `getArtistInfo2` calls
>   were answered without error.
> - **Internet radio is per account** (#217). Symfonium, Feishin and DSub
>   listed and played `wave`'s station. DSub signed in as `listener` showed
>   "No data", without an error, as a per-account list should.
> - **`getPlayQueue` answers nothing before a queue is saved** (#229). DSub
>   opened the never-used `fresh` account without an error and without
>   crashing. Juliet was not taken through a fresh account, so its half of that
>   check is still open.
> - **Public playlists** ([RFC-020](../rfcs/RFC-020-playlists-and-shares.md)).
>   `wave` sees `listener`'s public playlist with `owner=listener`, and the
>   server refuses `wave`'s rename and removal there with error 70, checked
>   directly; the playlist was left intact. Symfonium keeps such edits, and its
>   own renames and deletions, on the device and never sends them. Whether
>   Feishin offers no edit on another account's playlist was not recorded.
> - **Podcasts** ([RFC-025](../rfcs/RFC-025-smart-playlists-and-podcasts.md)).
>   DSub listed the channel and its four episodes as both `wave` and
>   `listener`, played the downloaded episode through `streamId`, and kept the
>   episode's position. Symfonium 15.1.0 and Feishin 1.17.0 show no podcast
>   screen over Subsonic, which is the client's absence, not a refusal.
> - **An unsynced lyrics sheet** (#367). Symfonium shows *Bridge*'s plain
>   sheet, held still, not scrolled; Feishin and Juliet render synced LRC.
>
> **Three deliberate differences in internet radio** that a client may notice:
>
> 1. **The list is per account**, where the reference server keeps one global
>    list only an administrator may change. A client built for that model sees
>    its own stations and can edit them, which is the shape it expects; it
>    simply never sees another account's. A station id belonging to somebody
>    else answers error 70, like every other foreign id here.
> 2. **`homePageUrl` out, `homepageUrl` in.** The specification spells the
>    response attribute with a capital `P` and the request parameter without
>    one. WaveFlow copies the asymmetry rather than correcting it, so a client
>    that follows the document reads the field it set.
> 3. **The server never connects to a station.** It validates the address as
>    absolute `http` or `https`, stores it as typed and hands it over; the
>    player is what reaches the station. A client on an HTTPS page therefore
>    cannot play a clear-text station — that is the browser's mixed-content
>    rule, not a server refusal, and the web client says so in advance.
>
> **How the runs were read.** A logging proxy stood in front of the server and
> recorded, for every request, the time, the path and the query string, with
> the credentials masked, plus the status and the client's address. It did not
> record request bodies, so a POST in form encoding shows its path only: 199 of
> Feishin's 322 requests were such POSTs, and they were attributed by address
> rather than by `c=`. Every result in a row was read back from the server's
> state or from that log, never from the client's own screen alone. One finding
> was the proxy's own: it dropped `Content-Length` while re-chunking a body,
> and without a length a FLAC with no SEEKTABLE cannot be seeked. It was fixed
> on 2026-10-28 at 02:17 UTC, so any FLAC seek observed through it before then,
> August's included, says nothing about the server.

| Client | Version | Login | Browse/search | Native/transcode | Playlists/user data | Status |
|---|---:|---:|---:|---:|---:|---|
| Symfonium | 15.1.0 | pass | pass | pass | limit | **Run 2026-10-28**, on an Android 17 emulator over the LAN, in three sessions. Signed in by password and by API key; a wrong password is answered with a clean error. Browse matches the server: 6 albums, *Night Shift* filed under compilations, 22 tracks, the trap file *Outtake* absent, `Lior Sand` and the genre counts right. Opus plays and seeks; MP3 seeks; **FLAC seeks once the length is known**, inside what it has already downloaded and without a new request — the August note that it could not seek in FLAC was the logging proxy dropping `Content-Length`, not the server. Lyrics show in Opus and native, *Bridge*'s unsynced sheet held still. Internet radio plays. A removed track disappears cleanly. **Playlists and user data stay `limit`**: stars, a rating, plays, a pushed playlist and a duplicate removed by its right occurrence were all read back from the server, but its renames and deletions are never sent, nor any edit of another account's public playlist, so the server keeps the original names. It also uses bookmarks rather than `savePlayQueue`, and restores a position from the device rather than from the server. Top songs, the radio mix and sharing are computed locally: no `getTopSongs`, `getSimilarSongs` or `createShare` was sent. Podcasts, AAC and now-playing are absent from this version. During its run, ~197 000 wrong passwords sent from another address throttled that address only; the right password from the device kept working, and an API key is never throttled (RFC-017). |
| Feishin | 1.17.0 | pass | pass | pass | pass | **Run 2026-10-28**, desktop on Windows over the LAN, 00:32–00:48 UTC. 322 requests, all answered 200 or 206, 199 of them POSTs carrying their parameters in the body. Signed in with `u` and a token, never an API key. Browse covers artists, albums, `getIndexes` and folders, genres, search and `getArtistInfo2`. Transcoding to Opus 128 played through 25 `stream` calls with 19 range reads answered 206, and no `timeOffset`. User data was read back from the server: a star, two ratings, plays, and a playlist rewritten in place to three tracks. Lyrics were requested nine times through `getLyricsBySongId` and rendered as synced LRC; `getTopSongs` six times and `getSimilarSongs` were rendered; internet radio played. Once, removing a track from a playlist showed "Cannot read properties of undefined (reading 'filter')" in the client; the same form POST replayed by hand was served correctly, the following `getPlaylist` and `getPlaylists` were well formed, and it did not reproduce. Podcasts are absent from this version over Subsonic, and it called neither `savePlayQueue`/`getPlayQueue` nor `getNowPlaying`. It browsed folders before #372, so *Untitled Sketch* at the root of `getIndexes` has not been seen in it. |
| Substreamer | 8.0.91 | pass | pass | pass | pass* | Validated 2026-08-02 from the official release source: native playback, Opus/128 cache, playlist add, favorite, rating and scrobble. `*` Its playback queue is local-only and the client never calls `getPlayQueue`/`savePlayQueue`; those endpoints remain covered by fixtures, and by Feishin's 2026-08-02 run. |
| DSub | 5.5.3 (F-Droid 208) | pass | pass | limit | pass | **Run 2026-10-28** on an Android 17 emulator over the LAN, "Browse By Tags" off, then **replayed the same day against #370–#375** (see the corrections below). The first run passed `getIndexes`, `getMusicDirectory`, `search2` and every sort of the legacy `getAlbumList`, `byGenre` and a reversed `byYear` included; the `maxBitRate` ceiling (128 plays the original, 64 transcodes); a generic `star?id` resolved as artist, album and track; the full playlist cycle; a queue saved, the app killed, and resumed from the server at the same position; podcasts through `streamId` with the episode's position kept; `getTopSongs`, `getSimilarSongs`, internet radio, `getLyrics`, sharing a track, and the per-password sign-in limit. It found five server defects, every one fixed and replayed: the shares screen crashed on a `getShares` with no `url` (#373); an album could not be shared (#373); the composers and producer nested in `contributors` were shown as starred artists and in an album header (#374); *Untitled Sketch*, which has no album, could not be reached by folders (#372); and an error answered in place of a stream was cached as audio and requested again every second (#375). The replay confirmed each fix. Native/transcode stays `limit` on its own behaviour: asked to play a removed track, it recognises the error and stops requesting, but shows no message and stays loading until stopped. Carried forward from 2026-09-16, not re-checked: its freeze after a backward seek in a long FLAC, and an artist list doubled by its local index. |
| Juliet | 8 (iOS) | limit | pass | limit | pass | **Run 2026-10-27**, then four further sessions to 2026-10-28, on a physical iPhone over an HTTPS tunnel, on Wi-Fi and mobile data. 576 requests in the first session, all 200 or 206, and nothing failed in it. Signed in with a token and salt; browse covers artists, albums, genres, random and search. 134 `stream` calls transcoded to Opus at 64 kbps, with range reads answered 206. User data was read back from the server: `createPlaylist`, an artist star, `savePlayQueue`/`getPlayQueue` and scrobbles. `getSimilarSongs2` was rendered, and synced lyrics show. **Login and Native/transcode are `limit`, for one client behaviour**: once `getOpenSubsonicExtensions` advertises `apiKeyAuthentication`, it sends part of its requests with the account password as `apiKey`, seeks among them, and crashes when they are refused — tapping a lyrics line is such a seek (see "Juliet sends the account password as `apiKey`" below). Before that call, every seek works by byte range. The server's fix for the iOS seek probe (#368) has therefore not been exercised by this client. Not called: podcasts, internet radio, smart and public playlists, and the lyrics of *Late Additions*, so the #367 fix was confirmed by Symfonium rather than by this client. |

Every row is read from server state rather than from the client's own display.
`pass` means the server answered the client correctly; **`limit` means it did
too, and the client did not make use of the answer** — a limit is never a
defect on this server's side, and never a pass either. A feature the client
does not have is recorded as absent, never inferred.

**The four rows carry the third campaign's date, 2026-10-27/28.** Each client
has been run four times before against an earlier server: on 2026-08-19
against the contract of the time; Symfonium, Feishin and DSub again on
2026-08-23 against the aligned artist model, and Juliet on 2026-08-31 against
the five album fields; and all four on 2026-09-16, against the façade split by
method family. Those runs live in this file's history. Where a row says
*carried forward*, the observation is from one of them and was not re-checked.
Substreamer is out of the replayed set for the reason given below, and its row
remains a historical result against the previous status behaviour.

The 2026-08-19 replay found **four server defects**, all fixed in the same
change and none of which the automated suite could have produced: it took a
browser, a client that edits a playlist by sending back what remains, a client
that browses by artist index, and an iOS player that probes a resource before
reading it. The third campaign found more, listed at the head of this file, for
the same reason: each needed a client doing something no test had thought to
do. Missing client features stay covered by automated fixtures and another real
client rather than inferred as passes. Creating any tag or release remains a
separate action requiring an explicit operator request.

The Substreamer row records the successful 2026-08-02 run. It could not be
reinstalled on the current Android 17 device during the 2026-08-15 revalidation
because the store marks that legacy build incompatible, and the 2026-08-19
re-run confirmed it: that build no longer launches on the current emulator. It
is therefore **out of the replayed set** — its row stays as historical evidence
of a real run against the old contract, and is not counted toward the next tag.
Juliet on a physical iPhone takes its place as the fifth client, which also
moves the iOS check off an emulator. The historical Substreamer evidence is
kept, not silently rewritten as a new run.

For Substreamer, the Android Media3 session completed the 11-track validation album and the server recorded the corresponding start/submission scrobbles. With the client's streaming profile set to Opus at 128 kbit/s, WaveFlow produced 11 distinct cache entries; `ffprobe` identified the output as an Ogg container with an Opus audio stream. Its playlist mutation, track/album favorites and 4/5 rating were also read back from WaveFlow rather than inferred from local UI state.

The real-client runs added four compatibility requirements to the automated suite: Feishin relies on the independent `artistOffset`, `albumOffset` and `songOffset` values in `search3`; DSub still calls the legacy `getAlbumList` method, treats `maxBitRate` as a ceiling rather than an unconditional transcode request, and sends album/artist favorites through the generic `star?id` form. WaveFlow supports both album-list methods with their protocol-appropriate containers, streams directly when a bitrate ceiling already admits the source, and resolves generic favorite IDs only inside the authenticated user's visible catalogue.

The Symfonium run added three narrowly scoped requirements. Version 14.1.0 validates the configured credential and then sends an exact `GET ping` discovery probe with `c=Symfonium` and `test/test`; WaveFlow answers only that public ping shape and never creates a principal. Its catalogue pagination uses the literal `search3` query `""` as match-all. It also requests `getBookmarks` during initial sync, which WaveFlow now answers from real per-account playback positions rather than with an empty container. Automated tests prevent the discovery probe from reaching any catalogue method or accepting alternate clients, duplicate identities or extra token parameters.

The contract audit additionally covers administrative folder access: `createUser` defaults to all libraries, repeated `musicFolderId` values select a subset, `updateUser` changes that subset, and `getUser`/`getUsers` return `folder[]`. Golden tests authenticate as the created user before and after a folder/password update and verify that `changePassword` leaves the Argon2id web credential untouched. Empty-result mutations emit an empty success envelope as required by the protocol.

## Deviation accepted for the next tag: HTTP 200 on every `/rest` answer

Protocol failures now answer HTTP 200 and report the outcome in the body
(`status="failed"` plus an `error` code), as the Subsonic contract requires.
Previously WaveFlow also set 401, 403, 404, 409 or 429. That this would not
break the clients was an expectation until the 2026-08-19 replay made it a
result: **each re-run row above was given a deliberate wrong password and
decoded it as an authentication error**. Range responses keep 206/416, and
`/share` and `/api/v2` are unchanged.

The same release adds `tokenInfo` — the half of `apiKeyAuthentication` that was
advertised but never served — and `getAlbumInfo`/`getAlbumInfo2`, so Feishin
and Symfonium no longer receive an unimplemented-method error when an album page
opens. `playlist.owner` and
`share.username` now carry the authenticated username instead of an empty
string, which is what Feishin reads to decide whether a playlist is editable.

Five further batches of wire changes landed while the matrix was still
unvalidated, and the 2026-08-19 replay covers them all: media items gained the remaining
OpenSubsonic fields under the presence rule (`moods`, `explicitStatus`, `isrc`,
`replayGain`, `bpm` and the rest); `startScan`, `getScanStatus`, `search2`,
`getStarred` and the bookmark methods were added or backed by real state;
`album` and `artist` gained `musicBrainzId`, which `getMusicDirectory` children
deliberately do not carry; and rescanning became restricted to the `owner` and
`manager` roles, so a client signed in as a listener now sees `startScan`
succeed while queuing nothing. Finally, genre matching was unified on the
canonical name, so `getSongsByGenre` and `getRandomSongs?genre=` now return a
genre in full where they previously returned the fraction whose spelling matched
the request; media items gained `albumArtists[]` and `displayAlbumArtist`, and
albums gained `artists[]` and `genres[]`; and `getAlbum` returns an album in
sleeve order rather than alphabetically.

Two wire changes land after that replay, both additive. `album` and `artist`
gained `sortName`, emitted with its default like every other supported
OpenSubsonic addition, so an album whose files carry no `ALBUMSORT` reports it
empty rather than omitting it — the difference between unknown and
unsupported. And the folder level of `getMusicDirectory` now lists the tracks
of that library which belong to no album, alongside its artists: those tracks
already named the library as their `parent`, and browsing there used to find
none of them. No existing attribute changed value, so a client validated
against the 2026-08-19 replay sees the same catalogue with two more fields on
it and one directory that is no longer a dead end.

One deviation is worth stating rather than leaving to be discovered. The
`artists[]` and `albumArtists[]` arrays on media items and albums are typed as
`ArtistID3` but carry only `id` and `name`. They are references: no
`musicBrainzId`, no `albumCount`, no `starred`, and — since sort names landed —
no `sortName` either. A client reading an artist's own fields out of one of
those entries finds them missing and should fetch the artist by the identifier
the entry carries. The artist and album entries `getMusicDirectory` returns as
`child` are a different shape and do carry those fields, `musicBrainzId`
excepted; the two projections are pinned against each other in the test suite,
in JSON and in XML.

Browser-hosted clients need their exact origins in the comma-separated `WAVEFLOW_ALLOWED_ORIGINS` setting. The server permits GET, form POST and OPTIONS from those origins and exposes the byte-range response headers used by web audio players. Wildcard origins are deliberately unsupported.

## Deviation for RFC-022: a format the server cannot produce falls back, and AAC

Two changes to `stream` land with
[RFC-022](../rfcs/RFC-022-playback-and-web-client.md), after the 2026-09-16
replay, and no client has been replayed against them yet.

- **`format=aac` is produced** — also spelled `m4a` or `adts`, in any case — as
  AAC-LC in an ADTS stream, `Content-Type: audio/aac`, 192 kbit/s unless
  `maxBitRate` names a rate. ADTS rather than an MP4 because a transcode is
  written down a pipe as it is made: ADTS streams from its first byte and a
  cached file seeks by byte range, while a regular MP4 cannot be written to a
  pipe at all and a fragmented one has no index to seek by.
- **A format the server cannot produce is read as no format at all**, where it
  used to be answered error 10 (`Unsupported format`): `flac`, `wav`, `alac`
  or a typo now get the original while `maxBitRate` admits it, and MP3 at the
  cap when it does not — the same answers a request naming no format gets. The
  `Content-Type` names what was sent. A client asking for FLAC over a FLAC
  library gets the file it wanted; one asking for FLAC over an MP3 gets an MP3
  that plays, which is what Navidrome does too.

This changes observable behaviour under a frozen contract, and the reason it is
taken anyway is that the old answer was a refusal to play: no validated client
depends on error 10 here, because a client receiving it plays nothing. A request
that worked before is answered exactly as it was.

The same RFC raises the `formPost` body bound from 64 KiB to
`MAX_PLAYLIST_TRACKS × 64 + 64 KiB` (705,536 bytes), so a playlist of the ten
thousand tracks the domain allows can be written in one `updatePlaylist` or
`createPlaylist`. Only requests that used to be refused with
`Invalid form body` are affected.

## Deviation for RFC-020 to RFC-022 follow-ups: `username`, `replayGain` and a longer queue

Three changes to `/rest` land with the follow-ups to RFC-020 and RFC-022, and
no client has been replayed against them yet.

- **`getPlaylists?username=`** is honoured as the specification says: for an
  administrator naming another account, that account's own playlists, private
  ones included, with their tracks read through the administrator's own
  libraries; and `getPlaylist` opens what that listing named. To anyone else
  naming another account, and to anyone naming an account that does not exist,
  it answers error 70 — the not-found every refusal here gives, so the
  parameter cannot be used to learn which accounts exist. Naming oneself is the
  ordinary listing. Writes stay the owner's. A request without the parameter
  is answered exactly as before. That account's smart playlists (RFC-025)
  follow its ordinary ones only when they are public, as any reader sees them:
  a private rule set is its owner's, and `getPlaylist` answers it error 70 to
  anyone else, administrators included; a public one is evaluated by its
  owner's rules and read through the administrator's libraries.
- **`stream` takes `replayGain=off|track|album`**, a parameter of this
  server's that every other server ignores, and the operator may set a default
  with `WAVEFLOW_SUBSONIC_REPLAYGAIN` (`off` unless set). It levels a
  transcode only; the original is never re-encoded for it, and an unknown mode
  is error 10. With the default left off, no answer changes.
- **`savePlayQueue` holds as many tracks as a playlist**, ten thousand, where
  it refused more than four hundred with error 10; such a request fits the
  `formPost` bound. `star`, `unstar` and `scrobble` keep their four hundred
  identifiers a request.

Reordering a playlist needs no new method: a client sends the whole list again
through `createPlaylist?playlistId=`, which replaces the track list in the
order given, as it always has. A move that keeps hidden entries in place is the
native API's (`POST /api/v2/playlists/{id}/move`).

## Deviation for RFC-023: a search reads its first 256 characters and 16 terms

[RFC-023](../rfcs/RFC-023-hardening.md) bounds the full-text query that
`search3`, `search2` and the native search build from what a client typed.
Each word was one more phrase FTS5 intersects, three times over, and nothing
bounded them but the form body — some seven hundred kilobytes. A query is now
read to its first 256 characters, and of those its first 16 words; the last
word kept still matches as a prefix.

The query is **narrowed, never refused**, so no client receives an error it
never had. Only a query longer than either bound answers differently — it may
match more than it did — and no validated client sends one: a search typed by a
person is a few words. Two queries never reach the bound at all, and answer as
they did:

- **The literal `""`** — two double quotes, `query=%22%22` — is the match-all
  Symfonium pages its catalogue with. It is recognised before any full-text
  query is built and answers the paged catalogue.
- **An empty or blank query** (`query=`, `query=%20`) names no term, and
  answers an empty `searchResult3`, not the catalogue. Nothing else under `/rest` changes; a disabled account's shares
closing (the same RFC) affects `/share/`, which is not part of the façade.

## Additions for RFC-025: smart playlists and podcasts

Both land after the 2026-09-16 replay, and no client had been run against them
before the third campaign of 2026-10-27/28, whose podcast results close this
section. Nothing that existed changes shape, with the one exception named.

**Smart playlists** ([RFC-025](../rfcs/RFC-025-smart-playlists-and-podcasts.md)
decision 6). `getPlaylists` lists them after the ordinary playlists, and
`getPlaylist` answers one with its entries, evaluated at that moment. Each
carries the OpenSubsonic `readonly="true"`. **`getPlaylists` evaluates none of
them** (decision 4): their `songCount` and `duration` are what the caller's
last `getPlaylist` of that playlist found, over what the caller can play, with
`validUntil` saying when; a smart playlist the caller has not opened since the
server started, or whose rules changed since, answers `songCount="0"`,
`duration="0"` and no `validUntil`. `getPlaylist` always evaluates, under a
budget, and answers error 0 when it is passed. `updatePlaylist`,
`deletePlaylist` and `createPlaylist` with a `playlistId` naming one answer
error 70, as for a missing playlist. **Ordinary playlists carry no `readonly`
attribute, as before**, so no existing answer changes. To check in the next
campaign: that Feishin and Symfonium offer no edit on a playlist marked
`readonly`, and that a client caching playlists by id refetches one whose
`validUntil` has passed.

**Podcasts** (decisions 7 to 13). `getPodcasts`, `getNewestPodcasts`,
`getPodcastEpisode`, `refreshPodcasts`, `createPodcastChannel`,
`deletePodcastChannel`, `deletePodcastEpisode` and `downloadPodcastEpisode`
answered "not implemented" (error 0) and are now served, in the schema's
`PodcastChannel` and `PodcastEpisode` shapes, with the five writes answering
the bare envelope. `podcasts.channel`, `channel.episode` and
`newestPodcasts.episode` are arrays in JSON at any length. A stored episode
carries `streamId`, its own id, and `stream` and `download` take it.
`createBookmark` and `deleteBookmark` take an episode id, and `getBookmarks`
lists episode bookmarks after the track ones, their `entry` the episode
(`type="podcast"`).

- **The one changed value: `podcastRole`.** It was `false` for every account,
  which was true. It is now `true` for an enabled administrator of a server
  with `WAVEFLOW_PODCASTS` on, and `false` otherwise — the default server still
  answers `false` for everyone.
- **One list per instance**, as in the reference server; an account without
  `podcastRole` reads and plays, and its writes answer error 50.
- **`channel.url` is the feed address to an account holding `podcastRole`,
  and only its scheme and host to anyone else** — `https://feeds.example.org/`.
  The schema requires the attribute, and a private feed carries its
  subscriber's token in its path as often as in its query. Episodes carry no
  enclosure address at all: they are played from the server.
- **Cover art, fetched by the server** (RFC-025, amended). A channel carries
  `coverArt`, the hash of the copy of the feed's cover the server fetched into
  the artwork store, and every episode carries its channel's — an episode's own
  `itunes:image` is not fetched. `getCoverArt` serves it to every account, as
  every account reads the channels. Both are absent until a fetch succeeds,
  and when `WAVEFLOW_REMOTE_ARTWORK` is off. **Changed values**: there was no
  `coverArt` on a channel or an episode before, and `originalImageUrl`, the
  feed's own address for the cover, is now given only to an account holding
  `podcastRole`, as `channel.url` is — a private feed's cover can carry its
  subscriber's token. Both are optional in the schema.
- **Internet radio stations carry no `coverArt`**, though a station may now
  have an image set through the native API: `InternetRadioStation` defines no
  such field, and the frozen contract is left as it was.
  `createInternetRadioStation` takes no image, and `updateInternetRadioStation`
  leaves a station's image as it is rather than clearing it.
- **A feed that cannot be read** answers `createPodcastChannel` with error 0
  and a generic message; nothing is stored.

Checked in the third campaign (2026-10-28): DSub lists the channel and its
four episodes, plays the downloaded one through `streamId` and keeps the
episode's position, signed in as `wave` and as `listener` alike, since every
account reads podcasts (decision 8). Symfonium 15.1.0 and Feishin 1.17.0 have
no podcast screen over Subsonic, so neither exercised the methods. Not
recorded: whether a channel's and an episode's `coverArt` show through
`getCoverArt`.

## Correction: an unsynced lyrics line is an object in JSON

Found on 2026-10-27, while preparing the third client campaign, before any
client connected. In JSON, `getLyricsBySongId` sent the lines of an
**unsynced** sheet as bare strings — `"line": ["…", "…"]` — where OpenSubsonic
documents `line` as `{start?, value}` objects. Synced lines were right, because
`start` made them objects; XML was right, because a line's text is its element
text there. The cause was the JSON renderer's general rule that an element
holding only text is a string (the documented shape of `moods`, `isrc` and
`roles`), which an unsynced line, having no `start`, fell under.

An unsynced line now renders as `{"value": "…"}`. No campaign before this one
could have seen it: the earlier catalogue held no unsynced lyrics. The third
campaign's did, in *Bridge*, and it ran on the fix: Symfonium shows that sheet
held still, without inventing a scroll.

## Correction: a seek probed by iOS is answered with a range

Found on 2026-10-27, in the third client campaign, while looking into a crash
in Juliet 8 on iOS. **Juliet's crash turned out to have another cause**,
described in the next section. This one is a server defect found while
replaying Juliet's requests with valid credentials.

A `timeOffset` seek into a transcode, probed with `Range: bytes=0-1`, got the
whole live transcode back. That is the probe AVFoundation sends before it
plays anything. The answer was a 200 with no `Content-Length` and
`Accept-Ranges: none`, so it told the player nothing about how long the stream
was. A play from the start is not affected, because it fills the cache and
every later request gets ranges. A seek had no cache entry of its own, so each
probe started a new live transcode. Navidrome caches a seek under a key that
includes the offset.

Now a seek requested with a bounded range from the first byte, such as
`bytes=0-1`, is encoded whole into the cache first, under its own key. The
probe then gets its two bytes and the total length as a 206. Every later range
of that seek is served from the same file. A seek with no range, or with
`bytes=0-` (browsers, Feishin), still streams live, so a listener hears it at
once. Replayed through the campaign proxy after the fix, on a 45-second FLAC
transcoded to Opus at 128 kbit/s:
- the first probe of a seek got its 206 in 150 to 300 ms;
- the probes after it, and a range further in, were served from the cache in
  2 ms;
- the total length shrank with the offset (724 614, 519 922 and 313 714 bytes
  at 10, 20 and 30 s).

`timeOffset` on the original, with no transcode, still returns error 10.
Ignoring the offset instead, as Navidrome does, would play the start of the
track at a position the client believes is later. No validated client has yet
received either answer, the 206 or the error 10.

## Juliet sends the account password as `apiKey`

Established on 2026-10-28, over four further sessions through the campaign
proxy. Before each of the last two, the transcode cache was emptied. The proxy
compared each `apiKey` with the campaign's credentials and logged only which
one it matched, never the value.

**What Juliet does.** Juliet 8 on iOS was configured with a username and a
password only, and no API key was ever entered. Once it has called
`getOpenSubsonicExtensions`, which advertises `apiKeyAuthentication` and
`transcodeOffset`, it changes how it authenticates:
- **it sends the account password as `apiKey`**, without `u` (204 requests in
  one session, against 72 still sent with `u` and a token);
- **it seeks through `timeOffset` on that path**, including into a track
  already cached (`timeOffset=1` on a cached file went out that way).

Before that call, Juliet seeks by byte range into the cached file, and every
seek works. That is the session in which nothing failed.

**What the server answers.** The server refuses each of those requests with
error **40** ("Wrong username or password"), inside an HTTP 200, as it does for
every authentication failure since the façade's statuses were aligned. Its
sign-in counter recorded `wrong_credentials` equal to the number of `apiKey`
requests in the same minute: 28 and 237 in two sessions. **Juliet crashes on
that refusal** when a lyrics line is tapped, which is the crash the
investigation started from. Once on that path, every later stream request is
refused too, until the app restarts. Juliet writes no log of its own.

**What the server should not do.** Accepting a password as an API key would
make the account password a bearer token sent in the clear in every request's
URL, where token authentication sends only a salted digest of it. The refusal is
correct, and Navidrome and Ampache refuse the same key.

**The error code is not the cause.** The OpenSubsonic error table defines code
**44**, "Invalid API key". Navidrome (`ec2e0a9e`) and Ampache (`aa31bde65d`)
use it for a refused key, and Navidrome uses 43 for a key sent together with
`u`, `p`, `t` or `s`. This façade reports both as 40, like any other credential.
To rule the difference out, the campaign proxy rewrote to 44 every refusal of
an `apiKey` request, and the test was repeated with an empty cache. All 37 of
Juliet's `apiKey` requests received 44, and Juliet crashed in the same way.
Returning 44 would still align the façade with the specification and both
servers, but it would change the frozen v2.0-beta contract, and it would not fix
Juliet.

**What the row records.** Juliet's row now records Juliet 8: it passes
everything except a seek made after it has read the extensions, and that limit
is the client's, so Login and Native/transcode read `limit`. The defect is to
be reported to Juliet's developer. The server's fix for the iOS seek probe
(the previous section) has not been exercised by Juliet, since its seeks never
get past authentication.

## Correction: a write on a removed track is refused like a read

Found on 2026-10-28, in the third client campaign, after a file was removed and
the library rescanned. The track became unavailable. `getSong` and `stream` on
it answered 70, but `createBookmark`, `deleteBookmark`, `scrobble`, `star`,
`unstar` and `setRating` answered `ok`. Symfonium and DSub both sent such writes
from their caches. The check every one of them goes through confirmed that the
account can see the track's library, and never that the track was still there.

Now those writes on an unavailable track answer **70**, through the façade and
the native API alike (404 there), and **write nothing, not even in part**. What
was written before the file went is not deleted: the star, the rating and the
bookmark stay, and come back when the file does, under the same identity.

**`savePlayQueue` is the one exception.** A client keeps a removed track in the
queue it holds. Refusing the whole queue left DSub unable to save any queue at
all, and it showed three `not found` errors. The queue is now saved **without**
the tracks that went. If the current track went, the current track and the
position are dropped with it. An identifier that was never a track is still
refused with 70, and the stored queue is left as it was.

Checked in the DSub replay of 2026-10-28, read back from the database. With
*Coda* removed, DSub's `unstar` and `setRating` on it were refused with 70, and
DSub showed the error in a toast. The star set that morning survived, and no
rating was written. A `savePlayQueue` still holding *Coda* was stored as the
other three tracks, with its position. When the file came back, *Coda* came
back under the same identifier, still starred.

## Correction: a search does not find an album by a removed track's title

Found on 2026-10-28, in the same campaign, after the same removal. *Coda* was
removed from *Late Additions*, which kept its three other tracks, none of them
named Coda. `search2 query=Coda` still answered the album *Late Additions*, and
no song. The song half of a search already left an unavailable track out. The
album half found its albums through the tracks the full-text index matched, and
that index keeps the row of a removed track, because the track itself is kept
for the file's return. Nothing asked whether the matching track was still there.
`search3` and the native `/api/v2/search` used the same query.

Now an album is found only through a track that is available, on `search2`,
`search3` and the native search alike. An album whose every track went is found
by none of them, its own title included. When the file comes back, the ordinary
rescan makes the title findable again; nothing is reindexed. No client is
affected beyond the result itself: the response's shape is unchanged.
Checked in the DSub replay of 2026-10-28: with *Coda* removed, its search for
"Coda" found neither the track nor the album. Its search for "Verse" still
found both the album and that track.

## Correction: an album whose files all went is not found

Reported on 2026-10-28, while the search above was being fixed; no client
reached it, because the campaign removed one track of a four-track album. An
album row outlives its files: the scan deletes an album only once no track row
points at it, and the row of a removed track is kept for the file's return.
Every album read counted the available tracks without asking whether there
were any. So an album whose every file had gone stayed in `getAlbumList`,
`getAlbumList2`, an artist's albums and the starred albums, with `songCount="0"`
and `duration="0"`; `getAlbum` opened it on no song; and a star or a rating on
it was accepted.

Now such an album is not found anywhere the catalogue is read, through the
façade and the native API alike: it is out of every album list, artist page,
search and starred list, `getAlbum` and `getMusicDirectory` on it answer
**70** (404 natively), and a star, an unstar or a rating on it answers 70 and
writes nothing. An album that keeps one track is unchanged. The two native
lists of a user's own data, `GET /api/v2/favorites` and `GET
/api/v2/ratings`, still carry its identifier, as they carry a removed
track's: they list what the account holds, not what the catalogue shows. Nothing is deleted: its star and its rating stay, and when the
files come back it returns under the same identifier with both. Covered by
`tests/subsonic_methods/removed_albums.rs`.

## Correction: a track with no album is a root file of `getIndexes`

Found on 2026-10-28, in the third client campaign, with DSub 5.5.3 browsing by
folders ("Browse By Tags" off). A track tagged with no album — *Untitled
Sketch* by Ivy Trench, filed under `Loose/` in the trap catalogue — could be
reached by search and by nothing else. It was in no artist's directory, since
an artist's directory lists albums, and `getIndexes` answered artists only.
This was not a regression: the folder level of `getMusicDirectory` has listed
such tracks since #123, because they name their library as their `parent`.
DSub never asks for that level. It enters at `getIndexes` and walks down from
the artists it lists.

`getIndexes` now carries those tracks as `child` elements after its `index`
elements, which is where the Subsonic schema puts the files at the root of the
folders. The projection, order and ceiling are the ones the folder level of
`getMusicDirectory` uses: the same media item, sorted by title, at most 2 000
tracks, with a warning in the log when a listing reaches the ceiling.
`musicFolderId` restricts them as it restricts the artists, and only libraries
the account belongs to contribute any. In JSON, `child` is an array even when
it holds one track. A response with no album-less track to list is unchanged,
with no `child` key at all, and `getArtists` is unchanged in every case, since
`ArtistsID3` has no `child`.

Checked in the DSub replay of 2026-10-28: browsing by folders, DSub lists
*Untitled Sketch* as a file at the root, after the six artist folders, and
plays it. Not yet seen: Feishin's folder view, the other
client in this table that calls `getIndexes`; it browsed folders in the
third campaign, but before this fix.

## Correction: DSub is answered without the OpenSubsonic fields

Found again on 2026-10-28, in the third client campaign, with DSub 5.5.3. In
XML a credit goes out as
`<contributors role="producer"><artist id="…" name="Hal Brennan"/></contributors>`,
and DSub's parser collects **every** `<artist>` element of a document, wherever
it sits. Two of its screens showed what it collected:
- in `getStarred`, the composers of a starred track (Bach and Gounod) were
  listed as starred artists;
- in `getMusicDirectory`, the header of the album Session Work showed
  "Hal Brennan", the producer of one of its tracks.

The 2026-08-23 replay recorded the same parser as a client limitation, and
chose not to narrow the wire by the declared `v=` (see the replay section
below). That reasoning stands for `v=`. This correction keys on the client's
**name** instead, which is how Navidrome meets the same client: its
`subsonic.legacyclients` setting is `DSub` by default, and a client named there
is given no OpenSubsonic field at all.

**The change.** `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS` lists client names, separated
by commas and compared exactly, case included, with the `c` parameter. It is
`DSub` by default; set it empty to answer every client in full. A request from
a listed client is answered without the OpenSubsonic fields the table below
names: one pass over the finished response removes them from the elements
listed there, before it is rendered in either encoding, and its JSON is not
given back, as empty arrays, the ones it removed; the arrays a method is made
of, such as the lyrics' `structuredLyrics`, keep their presence rule. What is
removed:

| Element | Attributes removed | Child elements removed |
|---|---|---|
| `song`, `entry`, `child`, `album`, `artist`, `similarArtist` | `bitDepth`, `bpm`, `channelCount`, `comment`, `displayAlbumArtist`, `displayArtist`, `displayComposer`, `explicitStatus`, `isCompilation`, `mediaType`, `musicBrainzId`, `played`, `samplingRate`, `sortName` | `albumArtists`, `artists`, `contributors`, `discTitles`, `genres`, `isrc`, `moods`, `originalReleaseDate`, `recordLabels`, `releaseDate`, `releaseTypes`, `replayGain`, `roles` |
| `playlist` | `readonly`, `validUntil` (smart playlists) | — |

**What stays.** Every attribute the 1.16.1 schema gives `Child`, `AlbumID3`,
`ArtistID3` and `Artist`. One element name here stands for several of those
types (`album` is an `AlbumID3` in `getAlbumList2` and a `Child` in
`getAlbumList`), so `title` and `userRating`, which `Child` carries, stay on an
album. The classic `genre` attribute stays; only the OpenSubsonic `genres`
array goes. A playlist's `comment` stays, because it is a Subsonic field there.
`getGenres`' `genres` container and `getAlbumInfo`'s `musicBrainzId` element
stay as well, because the lists are per element.

**The envelope keeps `openSubsonic="true"`**, as Navidrome's does. It describes
the server rather than the answer, an old client ignores an unknown attribute
on the root as it does elsewhere, and the methods only OpenSubsonic defines
(`getOpenSubsonicExtensions`, `tokenInfo`, `getLyricsBySongId`) still answer a
listed client that asks for them.

**An XML answer to a listed client is `text/xml`**, errors included, where
every other client keeps `application/xml`; JSON is `application/json` on both.
Found in the DSub replay of 2026-10-28: DSub 5.5.3 tells an error in place of a
stream from audio only by a type beginning `text/xml` or `text/html`
(`RESTMusicService`, in its download and its byte reads). Asked to play a
track removed from disk, it was answered error 70 in `application/xml`, wrote
those 234 bytes to its cache as a FLAC, and requested the stream again once a
second, 62 times in a minute, until it was stopped. Navidrome sends
`application/xml` too, so the same loop is DSub's on that server as well; the
type DSub tests for is the one the Subsonic server it was written against
sent. Its other calls parse the body whatever the type, so nothing else
changes for it. Checked in a second DSub replay, the same evening, with the
logging proxy counting requests. Asked to play the removed track from its
queue, DSub requested the stream 6 times within one second, then nothing more
until it was stopped a minute later. It recognised each answer as error 70,
cached nothing, and did not crash. It shows no message and does not move on
to the next track; it stays loading until stopped. That is the client's
choice once the error is received, and nothing on the wire changes it.

This changes the frozen v2.0-beta contract for DSub alone. Every other client,
every other row of the table above included, receives exactly what it received
before. Covered by `tests/subsonic_fields/legacy_clients.rs`, in XML
and JSON, over `getStarred`, `getMusicDirectory`, `getSong` and `getAlbum`, and
for the media type over `ping` and a `stream` error. Checked in the DSub
replay of 2026-10-28: its starred screen lists the three starred artists and
neither Bach nor Gounod, the Session Work header no longer shows a producer,
and each track keeps its own artist line. The rest of the replay, browsing and
playback included, went through on the same build.

## Correction: every share carries its `url`, and `createShare` takes an album

Found on 2026-10-28, in the third client campaign.

**`getShares` sent no `url`.** DSub 5.5.3's share screen crashed on opening,
with a `NullPointerException` in `Share.getName`, which rewrites the share's
`url` without checking it is there. The schema makes `url` required on a
`share`, and Navidrome always sends it. This façade sent it in the answer to
`createShare` only. [RFC-020](../rfcs/RFC-020-playlists-and-shares.md) had kept
it out of later reads because only the token's hash is stored. But the token
is derived from the instance key and the share's id, so the server can derive
it again on any read without storing it.

Now `getShares` and `updateShare` carry the `url` of every share, the same one
`createShare` gave. Only the owner is ever handed it: no Subsonic method lists
another account's shares, and the owner already reads every track the link
plays. The native `GET /api/v2/shares` still omits it.

**`createShare` refused an album ID with error 70.** The specification takes
the ID "of a song, album or video", and a share holds tracks only. Now an
album ID expands into the album's available tracks, in the album's order
(disc, then track). Albums and tracks can be mixed in one request, and the
share keeps the order the IDs were given in. An ID that names neither, or an
album of a library the caller cannot read, is still refused with 70. More than
400 tracks once expanded is refused with 10, as too many tracks always were.

The share page that the link opens is not a `/rest` answer, but it changed the
same day: its own play button now plays the first track. In Chrome on Android
it answered "Unable to play media" until a row was touched, because the player
had no source before one was. Checked in the DSub replay of 2026-10-28: the
share screen opens without crashing and shows each share's link; sharing *Late
Additions* returned a link whose page lists its four tracks; and in Chrome on
the same Android emulator, the player's own button started the first track.

## The replay this alignment required

These rows had all been re-run on 2026-08-19, against a catalogue whose artist
model was replaced the same week. That model was a variant: Navidrome has served
OpenSubsonic to hundreds of clients for years, and where the two disagreed it
was our disagreement to withdraw, not theirs. A matrix green against the variant
said nothing about the model that replaced it, so the whole set was run again.

**All four were replayed on 2026-08-23 against the aligned model**, and again
on 2026-09-16 against the façade as it stands after the split by method family;
and once more in the third campaign of 2026-10-27/28, whose date the rows above
carry. Substreamer stays out of the replayed
set for the reason given above, and is still not counted toward it.

That campaign found no defect in what it exercised, but it found two before it
reached a client at all, both invisible to a suite that only ever migrates an
empty database or searches a catalogue with no credits: the participants
migration failed to commit on any database holding a single track, and the
artist half of a search returned everyone credited on the matching tracks
rather than the artists whose name matched. Both are fixed.

It also settled a question the matrix could not have asked before. **The server
emits its OpenSubsonic fields whatever protocol version a client declares**, and
that is deliberate: `v=` is not a capability negotiation — Symfonium announces
1.13.0 while supporting far more — so trimming the wire to a declared number
would mean maintaining two shapes of every response and diverging from the
reference. DSub 5.5.3 pays for it: its parser lifts a nested `artist` out of a
`contributor`, so two of its screens show names the server never offered at
that level. Recorded as a client limitation, not repaired by narrowing the
wire. (Since 2026-10-28, DSub is answered without the OpenSubsonic fields,
by its client name rather than by `v=`; see "Correction: DSub is answered
without the OpenSubsonic fields" above.)

What changed on the wire, and why a green matrix from before it says nothing
about after it:

Additive — a client that ignores them sees the catalogue it saw:

- `contributors[]` and `displayComposer` on a media item, and `roles[]` on an
  artist. Absent before, which under the presence rule said the server did not
  read them; emitted with their default now, on a track that credits nobody.
- `albumArtists[]` may hold more than one entry. It was built from a single
  identifier, so an album credited to two artists named one and dropped the
  other on every one of its tracks.

Breaking — a client validated against the previous run can see these change
under it:

- **Every album and artist identifier changed.** They are derived from the tags
  now rather than drawn at random, which makes this the last time they move:
  the same files answer the same ids on a fresh install, where a rebuilt
  database used to re-mint every one of them. Track identifiers did not change.
  Favourites and ratings that name an album or an artist do not survive the
  transition: `user_star` and `user_rating` hold an untyped identifier with no
  foreign key, so their rows are left pointing at identifiers nothing answers
  for any more. Nothing reads them — every projection resolves through an
  `EXISTS` — so they are invisible rather than wrong, and a rescan does not
  recreate them. Track favourites, ratings, playlists, play history, bookmarks
  and queues are untouched, because track identifiers are.
- **An album's `artists[]` is its own credit**, the artists it is credited to,
  rather than the union of its tracks' credits. An album with a guest on one
  track was reporting the guest as one of its own artists.
- **`getArtists` and `getIndexes` list only artists an album is credited to.**
  A composer with no album of their own is reachable by identifier and by
  search — every artist has a full-text row of their own, matched on their
  name — but is no longer one of the library’s artists.
- **`search3` and `search2` match an artist on their own name.** The artist
  half of a search was derived rather than matched: it returned everyone
  credited on the tracks the full-text index had found, so searching a track
  title answered with that track’s whole session crew. Artists now carry a
  full-text row of their own, as they do in the reference.
- **`artist.albumCount` counts credits**, so both artists of a two-artist album
  now count it. The old count could only ever see the first.
- **A track's `artists[]` follows the reference's separators**: a padded slash,
  `feat.`, `ft.` and `"; "`, with the plural `ARTISTS` tag winning outright and
  never being cut. `AC/DC` survives, which an unpadded slash would not have.
- **`getArtist` and `getMusicDirectory` list an artist's albums by credit**, and
  through the same query — they used to filter a single column, which for an
  album with two album artists would have answered two different lists on one
  identifier.

Upgrading requires a full rescan. The server schedules it itself: the identity
rules a scan ran under are recorded when it completes, and a boot that finds
them different from its configuration asks every library to read every file
again. An interrupted migration resumes as a migration rather than skipping the
files it had already rewritten.
