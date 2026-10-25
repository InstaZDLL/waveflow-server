# RFC-023 — Hardening: bounded reads, FFmpeg's protocols, the API reference, and disabled accounts

- **Status**: Proposed
- **Implemented by**: the pull request that carries this RFC (RFC and
  implementation in one). The *Status* field above never flips in this
  project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the credential
  model, the frozen Subsonic contract),
  [RFC-019](RFC-019-exposure-hardening.md) (whose *What is still open* lists
  the first three items below)

## Problem

RFC-019 closed what a default installation exposed and left three findings
open. A pass over the server's other external inputs, made for this RFC, found
more of the same kind. Each was confirmed in the code:

1. **Answers from other hosts were read whole.** The Last.fm and Maloja
   scrobbling adapters (`src/scrobblers/`) called `.json()` on whatever came
   back. The metadata clients of `waveflow-core` already capped theirs at
   2 MiB; these did not, and the two calls of the Last.fm authorisation
   journey were not bounded in time either.
2. **FFmpeg was given no `-protocol_whitelist`.** Every input the server hands
   FFmpeg or ffprobe is a local path, but what a file may make either tool open
   from inside it rested on the installed build's defaults, which nothing here
   pinned or tested.
3. **`/reference` loaded Scalar from `cdn.jsdelivr.net`, unversioned.** Every
   visitor's browser ran whatever that CDN called the latest release, on this
   server's origin.
4. **A disabled account's shares still opened**, by their page, their JSON and
   their stream. So did its stream and canvas tickets, for up to an hour: a
   share is opened by nobody and a ticket is sealed rather than looked up, so
   the account was never asked.
5. **Four client inputs were bounded by the request body alone**, which is not
   a bound on what they cost:
   - an **LRC sheet** may put any number of timestamps before a line, and each
     becomes a copy of the line once read: a 1 MiB upload of 100,000 `[0:0]`
     before half a mebibyte of text expanded to some fifty gigabytes, on upload
     and on every read of the stored sheet;
   - a **playlist import** built every entry of the file before refusing more
     than 10,000 of them: a 5 MiB body of one-letter lines is millions;
   - the **sync socket** accepted tungstenite's default 64 MiB message, which it
     buffers whole before dropping it unread;
   - a **search** became one FTS5 phrase per word, three times over, for as many
     words as a 700 KB Subsonic form carries.

## Decision 1 — every answer from another host is read under a ceiling

- **One reader**, `outbound::read_bounded_body(response, limit)`, beside the
  client every outbound request is already built with. It refuses an answer
  whose `Content-Length` is over the limit before reading a byte, and cuts off
  one that announces nothing, or lies, as soon as it crosses the line. It knows
  nothing about who asked, so any later fetcher — an RSS feed, say — adopts it
  as it is.
- **2 MiB for an API answer** (`outbound::MAX_ANSWER_BYTES`), the figure
  `waveflow-core` holds the metadata providers to. The largest answer this
  server really reads is a few hundred bytes. The portrait fetcher keeps its own
  10 MiB, now through the same reader.
- **What an oversized answer means depends on what the body was for.**
  - Where the body only *refines* a verdict the status line already gave — a
    Last.fm `track.scrobble`, a Maloja submission — it refines nothing, and the
    status line decides, exactly as for a body that will not parse.
    `Ambiguous` was not chosen: RFC-010 turns it into a decision a person has
    to make, one per listen, over an answer no real destination sends.
  - Where the body *is* the answer — the Last.fm session key, the request
    token — the call fails as `Unavailable`.
- **Logged by verdict, never by URL or body.** `BodyError` carries one of two
  words and nothing else: a `reqwest::Error` prints its URL, and a Last.fm URL
  carries the operator's key.
- **The Last.fm authorisation calls are bounded in time** by
  `WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS`, the deadline every listen already
  has. Nothing above them had one.
- The metadata clients are unchanged: `waveflow-core` already bounds them, and
  its reader is private to that crate.

## Decision 2 — FFmpeg and ffprobe open local files and nothing else

- **Every input is named through `media::file_input`**, which passes
  `-protocol_whitelist file -i <path>`: the transcode, the quality probe, the
  measurement and the spectrogram, the canvas probe and the canvas remux. No
  call site builds `-i` itself, so none can forget the list.
- **`file` is all any of them needs.** Every input is a path the server chose.
  Outputs (`pipe:1`, a staging file) are not inputs and are not affected.
- **What it changes, measured on FFmpeg 8.0.** A top-level `http://` or
  `concat:` input was opened as asked, and a file opened through `file` could
  still name `crypto` and `data` resources from inside it. With the list,
  FFmpeg refuses `http` and `concat` at the door. An HLS playlist naming an
  `http` segment was already refused by that build's own default for nested
  opens (`file,crypto,data`); the list makes that hold on a build whose default
  is wider.
- **What it does not change.** A playlist-shaped file may still name another
  *local* file. FFmpeg 8 only detects HLS under an `.m3u8` or `.m3u` name, which
  no library file the scanner indexes carries, and the concat demuxer's safe
  mode refuses absolute paths; a demuxer whitelist would close it fully and is
  listed below as open.

## Decision 3 — `/reference` is served from the binary

- **The bundle is embedded.** `dist/browser/standalone.js` of
  `@scalar/api-reference` 1.73.1 — the file jsDelivr resolved the package to —
  stored as `gzip -9n` in `assets/scalar/` (1.3 MB, 4.4 MB decompressed) with
  Scalar's MIT licence beside it. A test holds the embedded bytes to the
  SHA-256 of the published file, and `src/reference.rs` records the npm
  integrity of the tarball and how to move to another release.
- **Served as stored**, at a versioned path (`/reference/scalar-1.73.1.js`)
  with `Cache-Control: immutable`, outside the compression layer: gzip to every
  client that takes it, decompressed for one that does not.
- **No request costs more than a reference count.** The route is open to
  whoever `/reference` is open to, and a first draft copied the 1.3 MB on
  every request and decompressed 4.4 MB on every request that took no gzip —
  CPU and memory for the asking, found by an automated review before merge. The
  gzip body is now the embedded bytes themselves (`Bytes::from_static`); the
  decompressed one is made once per process, on the first request that needs
  it, kept in a `OnceLock` and shared from then on. A test serves it five
  times without `Accept-Encoding` and counts one decompression.
  - *Kept, not dropped.* Always sending gzip whatever the request said would
    have removed the path entirely, and every browser that can run the page
    accepts gzip. It was not chosen because a body coded in a way the request
    did not admit is a wrong answer under HTTP, and the client most likely to
    ask without gzip — a script or a mirror fetching the file — would store
    compressed bytes under a `.js` name. The cost of answering it right is
    4.4 MB resident once such a client has asked, and nothing per request.
  - The page itself (some 200 KB with the document) is built once and shared
    the same way.
- **The page is built once** from the OpenAPI document. The document travels
  in a `<script type="application/json">` block with every `<` escaped, so no
  string in it can close the block.
- **Under a policy of its own**: `default-src 'none'`, scripts and connections
  from this origin only, no `eval`, framed only here. **Styles admit
  `'unsafe-inline'`**, and that was measured, not assumed: Scalar offers to
  stamp its styles with a nonce read from `<meta property="csp-nonce">`, but
  only the first `<style>` it injects carries it, and under a nonce Chromium
  refused six and the page rendered unstyled. A style cannot run code.
- **Everything in Scalar that would reach a host of its own is switched off**
  in its configuration — its fonts, its telemetry (on by default), its AI agent
  and MCP integration, its publishing toolbar — and the policy refuses those
  connections anyway.
- **Verified in a browser**: the page rendered, every resource it loaded came
  from this server, no policy violation was reported, and the "try it" client
  reached `/health` on the same origin.
- `utoipa-scalar` is dropped; `flate2`, already compiled for the compression
  layer, is named for the decompression.

**Rejected:** pinning the CDN URL with Subresource Integrity. It closes the
version drift, and still has every visitor's browser contact a third party.

## Decision 4 — a disabled account shares and plays nothing

- **Shares.** `share_is_open` — the one spelling of "open" that the page and
  the counted visit share — and the stream's own lookup now require an owner
  that is not disabled. The page, the JSON and the stream answer **404, exactly
  like a missing share**: a stranger holding the link learns nothing about the
  account. A refused opening counts no visit. Re-enabling the account reopens
  the share as it was.
- **Stream tickets** redeem through `stream_track_for_user`, and **canvas
  tickets** through `canvas_for_track`; both now require an account that is not
  disabled, so a ticket stops at the next request rather than at its expiry.
  The share stream reads through the same function as its owner, so it is
  closed twice.
- **Already refused, now tested.** API tokens, sessions and refresh tokens are
  looked up with `a.disabled = 0`, and an OAuth code granted before the account
  was disabled is refused by `issue_session_for_account`. None had a test; each
  now does, and each test re-enables the account to show the refusal was the
  account's.

## Decision 5 — the other inputs of the same kind

Fixed here, each small and each tested:

- **LRC expansion has a budget**: 8 MiB of expanded lines, counted as each
  line's text plus 32 bytes. A sheet over it is not a synced sheet: it is read
  as the text it is, one line per line of the file, and stored as unsynced, so a
  later read never expands it either. A real sheet repeats a chorus a handful
  of times and stays far below.
- **A playlist import stops at the first entry past 10,000**, before building
  it (`playlist_file::parse` takes the ceiling).
- **The sync socket closes on a message or frame over 4 KiB.** A client only
  ever needs to send a ping, a pong or a close, each at most 125 bytes.
- **A search reads its first 256 characters, and of those its first 16
  words.** Narrowed, never refused, so no client receives an error it never had;
  recorded in `docs/subsonic-compatibility.md` because `search3` and `search2`
  share the builder.

Checked and found bounded, so unchanged: upload offers and chunks (batch,
hash, size, extension, quota), every route's body limit, lyrics uploads
(1 MiB; TTML parsed with DTDs refused), radio station URLs (http and https
only, 2,048 characters, never fetched by the server), the Subsonic form body
(705,536 bytes), and the native string fields with a declared length.

## What does not change

- The Subsonic façade, except a search query past 256 characters or 16 words.
- The native API's routes and shapes. `/reference` is the same page, served
  differently.
- Any request a validated client makes today.

## What is still open

- **A demuxer whitelist for FFmpeg**, which would stop a playlist-shaped file
  from naming another local file. It needs the full list of formats a library
  may hold, and a file probed as anything else would stop playing.
- **The Subsonic parameter count.** The form body is bounded, but every
  `key=value` pair in it is kept: some 350,000 pairs, tens of megabytes per
  request, each lookup a linear scan. A ceiling of about 25,000 pairs would
  still admit the largest legitimate playlist write.
- **Free-text fields with no length of their own**: a playlist's comment, a
  share's description, a bookmark's comment, a play queue's `changed_by`, a
  library's name at creation, a track correction's fields, an OAuth grant's
  `client_id`, `redirect_uri` and `state`. Each is bounded only by its route's
  body, 16 KiB natively and 705,536 bytes through Subsonic. One helper and a
  length per field would close them; some of them reach the frozen façade.
- **Passwords have a floor and no ceiling.** A Subsonic password is
  MD5-hashed with the salt on every token request.
- **API token scopes are not checked against the known set** at creation.
- **No read timeout on a request body.** `/rest` is merged outside the request
  timeout, and `axum::serve` sets no header or body read deadline, so a slow
  sender holds a task. It belongs with the listener's configuration.
- **`sync_event` is never pruned**, and a playlist update journals its whole
  track list. It needs a retention policy, not a bound.
- **Second factor (TOTP)** on the web sign-in, still open from RFC-017.
