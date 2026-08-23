# Subsonic compatibility matrix — WaveFlow Server v2

Automated protocol coverage is enforced by `tests/v2_foundations.rs` for XML, JSON, `.view`, GET, form POST, `u/p`, `u/t/s`, `apiKey`, catalogue isolation, all documented mutations, media and artwork.

| Client | Version | Login | Browse/search | Native/transcode | Playlists/user data | Status |
|---|---:|---:|---:|---:|---:|---|
| Symfonium | 14.1.0 | pass | pass | pass | pass | Re-run 2026-08-19 against the current contract, on an Android 17 emulator through an ephemeral `cloudflared` HTTPS tunnel. Every observable change of the six batches was checked against server state rather than against the client's own display: sleeve order in both the album and a synced playlist, two distinct album artists with no composite entity in the artist list, two album genres, a genre spelled four ways answering as one (4 tracks), the extended fields including `explicitStatus`, and a failed login decoded correctly now that it arrives as HTTP 200. User data round-tripped: track and album ratings, track/album/artist favorites, 21 scrobbles, a bookmark resumed at 37 s, three playlists owned by the authenticated user, and 13 Opus cache entries across the 64 and 128 kbit/s ceilings. The earlier 2026-08-09 run on a physical Android 17 device additionally covered API-key authentication, which this one did not re-run. |
| Feishin | 1.15.1 | pass | pass | pass | pass | Re-run 2026-08-19 against the current contract, on Windows desktop. **This run found the two server defects fixed in the same change**: a cold transcode refused the `Range: bytes=0-` that a browser sends to open any resource, so playback failed on every track no earlier client had transcoded; and `createPlaylist` appended its `songId` values instead of replacing the list, so every playlist edit looked lost. Both were then replayed against the fixed server through a logging proxy and confirmed from server state: cold transcode, cache hit and seek all answer, a removal drops to one track, a reorder lands. The rest of the run exercised search (23 `search3`), 145 cover-art reads, genre browsing including 8 `getSongsByGenre`, artists, `getStarred`, `getRandomSongs`, 9 scrobbles, favorites on tracks/album/artists and ratings — all read back from the server rather than from the client. `getAlbumInfo`/`getAlbumInfo2` were never called by this version, so they stay unexercised here rather than counted as a pass, as do the bookmark and play-queue methods. `getTopSongs`, `getArtistInfo` and `getInternetRadioStations` answer empty containers, which is what this server has to say about them. |
| Substreamer | 8.0.91 | pass | pass | pass | pass* | Validated 2026-08-02 from the official release source: native playback, Opus/128 cache, playlist add, favorite, rating and scrobble. `*` Its playback queue is local-only and the client never calls `getPlayQueue`/`savePlayQueue`; those endpoints remain covered by fixtures, and by Feishin's 2026-08-02 run. |
| DSub | 5.5.3 (F-Droid 208) | pass | pass | pass | pass | Re-run 2026-08-19 against the current contract, on an Android 17 emulator, read from `logcat -s RESTMusicService` and DSub's own disk cache rather than from its interface. **This run found the album-artist defect fixed in the same change**: an album credited to two artists hung off a third entity named after the joined string, and browsing to either real artist found no album. The rest passed: sleeve order in both browse modes, two album genres, one canonical `Jazz` answering 4 tracks, artwork, native FLAC and MP3 with seek, a failed login decoded as an error now that it arrives as HTTP 200, playlist create/add/remove/rename/delete — the removal going through `songIndexToRemove`, by index — and a rating. It settles a question Symfonium left open: DSub does submit the scrobble of the last track in a queue. Two of its three unique surfaces needed the brief corrected. The legacy `getAlbumList` is used **only** when "Browse By Tags" is off, `getAlbumList2` otherwise; both were exercised. And `maxBitRate` is never sent above the source — DSub emits `min(setting, track bitrate)` — so "ceiling above the source" is not expressible from this client; the two reachable cases are correct (128 ceiling on a 128 kbit/s MP3 streams natively, 64 transcodes). The generic `star?id` form resolved a track, an album and an artist, all three surviving a resync. Unresolved, and not a server fault: one FLAC (`Drift`) is fetched repeatedly and never completed by the client, though the server answers 200 with the exact `content-length` and the bytes arrive md5-identical to the source file through the same tunnel. Absent from the client, recorded as absent: the explicit marker, and any separate display of two album artists. |
| Juliet | 1.5 (iOS 26.6) | pass | pass | pass | pass | Re-run 2026-08-19 against the current contract, on a physical iPhone — the previous entry was a narrower check that never reached transcoding or user data. **This run found the second half of the cold-transcode defect**: iOS AVFoundation opens a resource with a `bytes=0-1` probe, which the first fix still classified as a seek and refused, so the first play of each track failed and the second succeeded off the cache the failed request had built. Fixed in the same change and verified by replaying the client's exact sequence: `bytes=0-1` cold answers 200, warm answers 206 with two bytes, a real seek answers 206, and a seek into a cold transcode is still refused. The rest of the run: 306 cover-art reads, 115 streams, 64 scrobbles, `getAlbumList2`, `search3`, `getRandomSongs`, `getSongsByGenre`, `getSimilarSongs2`, `getLyrics`, favorites, and playlist create/update/delete. It is also the first client in the replayed set to call `savePlayQueue`/`getPlayQueue` - 48 times - where the matrix otherwise rests on fixtures and on Feishin's historical 2026-08-02 run, its 2026-08-19 re-run having never called them. Absent from the client, recorded as absent: Juliet has no rating feature, so `setRating` was never sent; and it saves its queue with `position=0` every time, so playback restarts at zero — the server stores and returns what it is given, queue, current track and `changedBy` included. |

**Symfonium, Feishin, DSub and Juliet were all re-run on 2026-08-19 against the
current contract**, each on a real device or desktop and each read from server
state rather than from the client's own display. Substreamer is out of the
replayed set for the reason given below, and its row remains a historical
result against the previous status behaviour.

The replay found **four server defects**, all fixed in the same change and none
of which the automated suite could have produced: it took a browser, a client
that edits a playlist by sending back what remains, a client that browses by
artist index, and an iOS player that probes a resource before reading it.
Missing client features stay covered by automated fixtures and another real
client rather than inferred as passes.

Feishin and Juliet found their defect rather than passing it, so each one
re-played the fixed path itself rather than being credited on a replayed
request sequence: Feishin's MP3 transcode now starts, caches and seeks, and
Juliet's two-byte probe answers 200 cold with no refusal left in the log. The
five rows therefore record the behaviour of the server as it stands. Creating
any tag or release remains a separate action requiring an explicit operator
request.

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

## The replay this alignment requires

The four rows above were re-run on 2026-08-19 against a catalogue whose
artist model has since been replaced. That model was a variant: Navidrome has
served OpenSubsonic to hundreds of clients for years, and where the two
disagreed it was our disagreement to withdraw, not theirs.

**The four replayed rows have to be run again before the next tag.** Substreamer stays out of the replayed set for the reason given above, and is still not counted toward it. What changed on the
wire, and why a green matrix from before it says nothing about after it:

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
  search — the full-text index carries every participant's name — but is no
  longer one of the library's artists.
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
