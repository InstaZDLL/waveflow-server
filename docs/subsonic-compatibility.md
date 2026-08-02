# Subsonic compatibility matrix — v2.0-beta

Automated protocol coverage is enforced by `tests/v2_foundations.rs` for XML, JSON, `.view`, GET, form POST, `u/p`, `u/t/s`, `apiKey`, catalogue isolation, all documented mutations, media and artwork.

| Client | Version | Login | Browse/search | Native/transcode | Playlists/user data | Status |
|---|---:|---:|---:|---:|---:|---|
| Symfonium | 15.0.0 | blocked | blocked | blocked | blocked | The official client forces HTTPS and does not trust the emulator's user CA; validate against a temporary publicly trusted HTTPS endpoint. |
| Feishin | 1.15.1 | pass | pass | pass | pass | Validated 2026-08-02: native playback, Opus cache, playlist create/add, favorite, rating, scrobble and queue. |
| Substreamer | 8.0.91 | pass | pass | pass | pass* | Validated 2026-08-02 from the official release source: native playback, Opus/128 cache, playlist add, favorite, rating and scrobble. `*` Its playback queue is local-only and the client never calls `getPlayQueue`/`savePlayQueue`; those endpoints remain covered by fixtures and Feishin. |
| DSub | 5.5.3 (F-Droid 208) | pass | pass | pass | pass* | Validated 2026-08-02 on Android 14: native playback, 64 kbit/s MP3 transcoding, queue, playlist add, album favorite and rating. `*` Scrobbling was disabled in this client run; the endpoint is covered by fixtures, Feishin and Substreamer. Android 17 remains incompatible with this legacy client runtime before it issues a media request. |

Do not tag `v2.0-beta` until every row records the tested client version and every protocol feature the client actually implements has been exercised. A missing client feature must be confirmed from that client's source or traffic and covered by the automated fixtures plus at least one other real client; it must not be recorded as a server pass merely because the UI lacks the feature.

For Substreamer, the Android Media3 session completed the 11-track validation album and the server recorded the corresponding start/submission scrobbles. With the client's streaming profile set to Opus at 128 kbit/s, WaveFlow produced 11 distinct cache entries; `ffprobe` identified the output as an Ogg container with an Opus audio stream. Its playlist mutation, track/album favorites and 4/5 rating were also read back from WaveFlow rather than inferred from local UI state.

The real-client runs added four compatibility requirements to the automated suite: Feishin relies on the independent `artistOffset`, `albumOffset` and `songOffset` values in `search3`; DSub still calls the legacy `getAlbumList` method, treats `maxBitRate` as a ceiling rather than an unconditional transcode request, and sends album/artist favorites through the generic `star?id` form. WaveFlow supports both album-list methods with their protocol-appropriate containers, streams directly when a bitrate ceiling already admits the source, and resolves generic favorite IDs only inside the authenticated user's visible catalogue.

The contract audit additionally covers administrative folder access: `createUser` defaults to all libraries, repeated `musicFolderId` values select a subset, `updateUser` changes that subset, and `getUser`/`getUsers` return `folder[]`. Golden tests authenticate as the created user before and after a folder/password update and verify that `changePassword` leaves the Argon2id web credential untouched. Empty-result mutations emit an empty success envelope as required by the protocol.

Browser-hosted clients need their exact origins in the comma-separated `WAVEFLOW_ALLOWED_ORIGINS` setting. The server permits GET, form POST and OPTIONS from those origins and exposes the byte-range response headers used by web audio players. Wildcard origins are deliberately unsupported.
