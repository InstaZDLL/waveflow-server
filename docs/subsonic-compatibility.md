# Subsonic compatibility matrix — WaveFlow Server v2

Automated protocol coverage is enforced by `tests/v2_foundations.rs` for XML, JSON, `.view`, GET, form POST, `u/p`, `u/t/s`, `apiKey`, catalogue isolation, all documented mutations, media and artwork.

| Client | Version | Login | Browse/search | Native/transcode | Playlists/user data | Status |
|---|---:|---:|---:|---:|---:|---|
| Symfonium | 14.1.0 | pass | pass | pass | pass | Validated 2026-08-09 on Android 17 through an ephemeral `cloudflared` HTTPS tunnel: account and API-key authentication, full catalogue sync, native MP3/FLAC Range playback, Opus at 64 kbit/s, album favorite, scrobbles and playlist create/update. |
| Feishin | 1.15.1 | pass | pass | pass | pass | Validated 2026-08-02: native playback, Opus cache, playlist create/add, favorite, rating, scrobble and queue. |
| Substreamer | 8.0.91 | pass | pass | pass | pass* | Validated 2026-08-02 from the official release source: native playback, Opus/128 cache, playlist add, favorite, rating and scrobble. `*` Its playback queue is local-only and the client never calls `getPlayQueue`/`savePlayQueue`; those endpoints remain covered by fixtures and Feishin. |
| DSub | 5.5.3 (F-Droid 208) | pass | pass | pass | pass* | Validated 2026-08-02 on Android 14, then revalidated 2026-08-15 on the current Android environment: authentication, catalogue, artwork, native playback, seek and playlist. `*` Scrobbling was disabled in the original run; the endpoint remains covered by fixtures and other clients. |
| Juliet | iOS build tested 2026-08-15 | pass | pass | native pass; transcode not run | not run | Current iOS compatibility check: authentication, catalogue, artwork and native playback succeeded. Unsupported/unexercised surfaces are not inferred as passes. |

The M3 real-client gate is closed. This matrix remains the regression record;
missing client features are covered by automated fixtures and another real
client rather than inferred as passes. Creating any tag or release remains a
separate action requiring an explicit operator request.

The Substreamer row records the successful 2026-08-02 run. It could not be
reinstalled on the current Android 17 device during the 2026-08-15 revalidation
because the store marks that legacy build incompatible; Juliet provides the
current iOS sanity check instead. The historical Substreamer evidence is kept,
not silently rewritten as a new run.

For Substreamer, the Android Media3 session completed the 11-track validation album and the server recorded the corresponding start/submission scrobbles. With the client's streaming profile set to Opus at 128 kbit/s, WaveFlow produced 11 distinct cache entries; `ffprobe` identified the output as an Ogg container with an Opus audio stream. Its playlist mutation, track/album favorites and 4/5 rating were also read back from WaveFlow rather than inferred from local UI state.

The real-client runs added four compatibility requirements to the automated suite: Feishin relies on the independent `artistOffset`, `albumOffset` and `songOffset` values in `search3`; DSub still calls the legacy `getAlbumList` method, treats `maxBitRate` as a ceiling rather than an unconditional transcode request, and sends album/artist favorites through the generic `star?id` form. WaveFlow supports both album-list methods with their protocol-appropriate containers, streams directly when a bitrate ceiling already admits the source, and resolves generic favorite IDs only inside the authenticated user's visible catalogue.

The Symfonium run added three narrowly scoped requirements. Version 14.1.0 validates the configured credential and then sends an exact `GET ping` discovery probe with `c=Symfonium` and `test/test`; WaveFlow answers only that public ping shape and never creates a principal. Its catalogue pagination uses the literal `search3` query `""` as match-all. It also requests `getBookmarks` during initial sync, for which WaveFlow returns the standard empty container until audiobook progress exists. Automated tests prevent the discovery probe from reaching any catalogue method or accepting alternate clients, duplicate identities or extra token parameters.

The contract audit additionally covers administrative folder access: `createUser` defaults to all libraries, repeated `musicFolderId` values select a subset, `updateUser` changes that subset, and `getUser`/`getUsers` return `folder[]`. Golden tests authenticate as the created user before and after a folder/password update and verify that `changePassword` leaves the Argon2id web credential untouched. Empty-result mutations emit an empty success envelope as required by the protocol.

Browser-hosted clients need their exact origins in the comma-separated `WAVEFLOW_ALLOWED_ORIGINS` setting. The server permits GET, form POST and OPTIONS from those origins and exposes the byte-range response headers used by web audio players. Wildcard origins are deliberately unsupported.
