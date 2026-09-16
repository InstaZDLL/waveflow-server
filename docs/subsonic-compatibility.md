# Subsonic compatibility matrix — WaveFlow Server v2

Automated protocol coverage is enforced by the `subsonic_contract`, `subsonic_browse`, `subsonic_fields` and `subsonic_methods` targets under `tests/` for XML, JSON, `.view`, GET, form POST, `u/p`, `u/t/s`, `apiKey`, catalogue isolation, all documented mutations, media and artwork.

> **Three of the five rows were replayed on 2026-09-16 and name the model
> `main` serves; Feishin's and Substreamer's are dated earlier and do not.**
> §4 of [`handoff-2026-08-23.md`](handoff-2026-08-23.md) asked for the campaign
> to be replayed before a stable tag, and it has been — for Symfonium, DSub and
> Juliet, against a façade that had since been **split by method family**
> (`b6888c6`), had **seven defects in the moved code** corrected (`e57559f`),
> had **five output fields added to an album** on 2026-08-24, and had changed
> what a track answers (`efa7af0`, "a song is not a release"). The read path
> moved twice as well: `03f5a06` caches the transcode a seek abandons, and
> `3770966` puts a deadline on filling that cache. **§4 stays open for Feishin
> and Substreamer**, which have not been replayed since 2026-08-23 and
> 2026-08-02.
>
> Nothing the 2026-09-16 campaign found is a regression. Everything it surfaced
> is reachable at `v2.0.0-beta.0` too, and is filed as #224, #225 and #226.

| Client | Version | Login | Browse/search | Native/transcode | Playlists/user data | Status |
|---|---:|---:|---:|---:|---:|---|
| Symfonium | 15.0.1 | pass | pass | pass | limit | **Re-run 2026-09-16** against the split façade, on an Android 17 emulator, on a 164-file library. **Seeking a lossless track now lands**, which is what moves this column from `limit`: a cold FLAC never played started immediately, a forward seek landed at 120 633 ms and a backward one at 29 090 ms, both continued playing, and the same held on AAC and MP3. **Quitting and replaying behaved identically to the first play** — the point the run existed to test, since two of August's four defects failed the first play and passed the second off the cache the failure built. With the Wi-Fi ceiling set to 128 kbps the badge changed from `Lossless` to `High Efficiency` and the duration stayed right, so the transcode works, and seeks worked inside it as well — **by swallowing the whole transcoded stream and moving locally**, not by asking for a range: six seconds after the track restarted, a seek to 3:46 of 4:06 landed at once. That strategy needs a capped bitrate and a fast network; it is a client's choice, not a protocol feature. 25 range reads were answered 206 in this window and the trace cannot name which client sent them — but DSub froze at the same gesture without sending anything, which leaves this one. Browse passed broadly: 130 albums, 117 artists, `Perfume` showing exactly its `albumCount` of 4, all 8 genres, and non-Latin titles correct throughout. **Its `Composers` category is built directly on `contributors`** and files three entries under the letter `I`, because they begin `INHOUSE)` — #224, present at `v2.0.0-beta.0`. Its `Album artists` category is not affected: it reads `albumArtists`, which is right. `Record labels` answers "Nothing to display", because no file in the library carries the tag — verified with `ffprobe` across all 164, not inferred. The client logs no REST calls of its own, so everything here was read from the server. **Playlists were not retested**, so the 2026-08-23 limit stands: no `createPlaylist` or `updatePlaylist` ever reached the server, while Feishin and DSub both create playlists against it. Carried forward from 2026-08-23, not re-checked: a failed login decoded from its HTTP 200, the separator traps (`AC/DC` one artist, `Bach/Gounod` two composers, a padded slash two artists), that it renders `artist`, `albumArtist` and `composer` and ignores the other ten roles, that bookmarks are exercised incidentally, and that it never calls `getArtists`/`getIndexes`, syncing entirely through the match-all `search3`. |
| Feishin | 1.15.1 | pass | pass | pass | pass | Re-run 2026-08-23 against the aligned model, on Windows desktop, and the row that settles what Symfonium's does not: **it seeks by byte offset into the same seektable-less FLAC files** — Zenith, Padded and Drift, seven ranges, all answered 206 — and **creates a playlist that lands in the database with its four tracks**. Two artists separated correctly. It sends its catalogue calls as POST with the parameters in the body, which the proxy records by method but not by argument. 398 requests, all 200 or 206, covering `getArtists`, `getIndexes`, `getMusicDirectory`, `getAlbumList2`, `search3`, `getSongsByGenre`, `getTopSongs`, `getArtistInfo`, `getRandomSongs`, lyrics, 15 scrobbles, favourites and ratings — read back from server state. `savePlayQueue`/`getPlayQueue` were not called by this version, as in its previous run. |
| Substreamer | 8.0.91 | pass | pass | pass | pass* | Validated 2026-08-02 from the official release source: native playback, Opus/128 cache, playlist add, favorite, rating and scrobble. `*` Its playback queue is local-only and the client never calls `getPlayQueue`/`savePlayQueue`; those endpoints remain covered by fixtures, and by Feishin's 2026-08-02 run. |
| DSub | 5.5.3 (F-Droid 208) | pass | limit | limit | pass | **Re-run 2026-09-16** against the split façade, on an Android 17 emulator. **The full playlist cycle passes again, read from server state**: created with 4 tracks, `songCount=4`; two added through `updatePlaylist&songIdToAdd`, `songCount=6` in order and without duplicates; one removed by index, `songCount=5` with the right track gone; renamed, `songCount=5` intact; re-read, 5 tracks in order; deleted, none left. **August's `createPlaylist` defect — appending where it should replace — does not reproduce**, and the client only ever uses `createPlaylist` to create. User data passes throughout: `star` with each of `id`, `albumId` and `artistId`, all three surviving a full client restart and visible from the other client; `setRating&rating=4` held as `userRating=4`; a track played to the end sent `scrobble?submission=false` then `submission=true`, and the server moved from never-played to `playCount=1`; `savePlayQueue`/`getPlayQueue` round-tripped with `current`, position and `changedBy=DSub`. **`getPlayQueue` names its children `song` where the Subsonic schema says `entry`** — #225, present at `v2.0.0-beta.0`. This client accepts it, which is how three campaigns passed over it. Browse stays `limit`, and Native/transcode **drops to `limit`** on this client's own behaviour, each verified against the server rather than the screen: it named `.complete.flac` a download of 9 216 000 bytes out of 47 835 313 and then froze, while the server announced the right `content-length` and delivered the whole file to the workstation at the same moment; after a backward seek it reported `PLAYING` with the position frozen for over 30 seconds and **sent the server nothing at all**, its log looping on a `.partial.flac` that does not exist; it sends an artist id to `getAlbum`, where error 70 is the correct answer; it doubles its artist list by mixing server results with its local index, showing ten artists for a query the server answers with none; and it promotes a contributor to a starred artist that `getStarred2` does not return. **The flattening parser recorded on 2026-08-23 reproduces**: it lifts the `artist` nested inside a `contributor` to the top level, so an album header shows a composer's name. Pagination is sound — 20 at a time, 10 at `offset=120`, 0 beyond, so it sees all 130 albums. One presentation difference, not a defect: the server puts the non-Latin index group first, this client puts it after Z. Carried forward from 2026-08-23, not re-checked: `getLicense`, `getIndexes`, the legacy `getAlbumList`, and the `maxBitRate` ceiling behaviour. |
| Juliet | 1.5 (iOS, version not re-checked) | pass | limit | pass | pass | **Re-run 2026-09-16** against the split façade, on a physical iPhone over the LAN. **924 requests, 866 answered 200, 53 answered 206, and exactly one 416.** The 53 partial reads are the result that matters: **iOS seeks by byte range and the ranges are served** — the two-byte probe that August read as a seek does not reproduce. **The single 416 is #226 seen in the field**: at 04:37:42 UTC a range was refused in 1 ms, the client re-asked without a range 58 ms later and was served 200, and the next seek two seconds after was answered 206. The user saw nothing, because the client recovered on its own; the refusal is real all the same, and it is what a cold transcode answers until its cache exists. State left in the database, read back rather than believed: the play queue is `changed_by="Juliet"` with 20 tracks at position 1 750 ms, and 37 play events fall inside this window. The artist page cross-checks exactly against the server — `ITZY`, `albumCount: 1`, `BORN TO BE`, 2024, 10 tracks, 1 891 s rendered as "31 mins". Exercised in this run: `getArtist`, `getAlbum`, `getAlbumList2`, `getArtists`, `search3`, `getGenres`, `getSongsByGenre`, `getRandomSongs`, `getSimilarSongs2`, both lyrics methods, `download`, `star`, `scrobble`, `createPlaylist`, `getPlaylist`, `savePlayQueue`/`getPlayQueue`, and **`getNowPlaying`, which names its child `song` where the schema says `entry`** (#225) and was accepted. Browse stays `limit` for the same reason as 2026-08-31: the artist split is offered and not consumed — the server answers `artists[]` as distinct entities while the legacy `artist` field carries the joined display string, and that string is what this client shows. **Two limits of the method**: the trace records path and status, never headers or query strings, so the 53 range reads are countable and their byte offsets are not, and the client version cannot be recovered from it — the version above is carried forward from 2026-08-31 and was not re-checked. Attribution to this client rests on the window boundary: the Android campaign ended at 04:20 UTC by the operator's own clock, and every request after it is this one's. Carried forward from 2026-08-31: the five album fields validated against a catalogue built to carry several values, exactly one, and none. **That check cannot be repeated on this library** — no file in it carries `LABEL`, `RELEASETYPE`, `ORIGINALDATE` or `DISCSUBTITLE`, verified with `ffprobe` across all 164, so all four answer empty here and only `releaseDate` is populated. |

Every row is read from server state rather than from the client's own display.
`pass` means the server answered the client correctly; **`limit` means it did
too, and the client did not make use of the answer** — a limit is never a
defect on this server's side, and never a pass either. A feature the client
does not have is recorded as absent, never inferred.

**Symfonium, Feishin, DSub and Juliet were all re-run on 2026-08-19 against the
contract of the time**; the first three were replayed again on 2026-08-23
against the aligned artist model, and Juliet on 2026-08-31 against the five
album fields. **Symfonium, DSub and Juliet were replayed once more on
2026-09-16**, against the façade as it stands after the split by method family,
and carry that date. Feishin's row still carries 2026-08-23. Substreamer is out
of the replayed set for the reason given below, and its row remains a
historical result against the previous status behaviour.

The 2026-08-19 replay found **four server defects**, all fixed in the same change
and none of which the automated suite could have produced: it took a browser, a client
that edits a playlist by sending back what remains, a client that browses by
artist index, and an iOS player that probes a resource before reading it.
Missing client features stay covered by automated fixtures and another real
client rather than inferred as passes.

Feishin and Juliet found their defect rather than passing it, so each one
re-played the fixed path itself rather than being credited on a replayed
request sequence: Feishin's MP3 transcode now starts, caches and seeks, and
Juliet's two-byte probe answers 200 cold with no refusal left in the log. Those
rows recorded the behaviour of the server as it stood on that date, which the
alignment has since changed — see the replay section below for what the three
rows dated 2026-08-23 re-establish and what Juliet's still does not. Creating
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

## The replay this alignment required

These rows had all been re-run on 2026-08-19, against a catalogue whose artist
model was replaced the same week. That model was a variant: Navidrome has served
OpenSubsonic to hundreds of clients for years, and where the two disagreed it
was our disagreement to withdraw, not theirs. A matrix green against the variant
said nothing about the model that replaced it, so the whole set was run again.

**All four were replayed on 2026-08-23 against the aligned model**, and every row
above now carries that date. Substreamer stays out of the replayed set for the
reason given above, and is still not counted toward it.

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
wire.

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
