# RFC-019 — Exposure hardening: first-run setup, browser headers, upgrade copies, and what leaves the server

- **Status**: Proposed
- **Implemented by**: [#325](https://github.com/InstaZDLL/waveflow-server/pull/325)
  (this RFC, the setup token, the security headers, the pre-migration copy
  and `docs/privacy.md`, in one pull request). The *Status* field above never
  flips in this project; this line is the one that says what is running.
- **Date**: 2026-10-25
- **Authors**: the WaveFlow project
- **Depends on**: [RFC-002](RFC-002-waveflow-server-v2.md) (the credential
  model, the frozen Subsonic contract),
  [RFC-017](RFC-017-sign-in-hardening.md) (which listed the first two items
  below as still open)

## Problem

A comparative audit of this server against Navidrome, Meelo, kima-hub and
Ampache (2026-10-24) found four places where a default installation trusts
more than it should, or says less than it should. Each was confirmed in the
code:

1. **Whoever reaches a fresh instance first becomes its administrator.**
   `POST /api/v2/setup` (`src/api/setup.rs`) creates the first account for any
   request with a plausible `Origin` header. A browser cannot forge that
   header; every other client writes whatever it likes. And
   `docker-compose.yml` publishes port 4533 on every interface, so a fresh
   container is reachable from the whole network for as long as it takes its
   operator to open it.
2. **The web client sends no security headers.** Nothing stops another site
   from framing it, nothing tells the browser not to sniff a content type, and
   nothing restricts where its scripts may come from. Public shares carry
   their bearer token in the path (`/share/{token}`), and no referrer policy
   keeps that path from being handed on.
3. **An upgrade has no way back of its own.** Migrations are embedded and run
   by the boot. One that fails leaves the server refusing to start on a
   database that is already somebody's, and the only copy is whatever backup
   the operator happened to take.
4. **Nothing says what the server sends to other hosts.** Metadata
   enrichment and scrobbling are opt-in, but the facts — which hosts, which
   fields, when, and which variable stops it — are spread over three RFCs and
   the code.

## Decision 1 — a one-time setup token

While no account exists, setup asks for a **setup token** besides the
username, the password and the origin check, which stays.

- **Drawn at boot when the operator chose none.** 256 random bits, written to
  `<data dir>/setup-token` — owner-only on Unix, through the same
  `create_new` and `0600` path the instance key takes. A token already on disk
  is kept across restarts, so a container that restarts before its operator
  has read the file does not change the answer underneath them.
- **The log names the file, never the token.** `CLAUDE.md` forbids logging a
  token, and the rule is right here: a log is read by more people, shipped to
  more places and kept longer than the data directory it describes. Whoever
  can read the data directory can read the database anyway; the token gives
  them nothing new. The line is written by `serve`, not by `initialize`, so a
  CLI command's standard output stays its own.
- **Or chosen by the operator** in `WAVEFLOW_SETUP_TOKEN`, read in
  `src/config.rs` like every other variable, for a deployment whose data
  directory is awkward to reach. It must be at least 16 characters. No file is
  written then, and a file left by an earlier boot is deleted, so there is one
  token, not two.
- **Compared in constant time.** The gate keeps the SHA-256 digest of the
  token and compares digests with `subtle`, so neither the content nor the
  length of a wrong guess changes how long the refusal takes. An absent token
  is compared like a wrong one.
- **Checked before the username and the password.** The order is: the JSON
  body is parsed, then the `Origin` check runs, then the token, and only then
  are the username and password validated. A caller without the token
  therefore learns nothing about the account it would create, not even the
  password rules. A missing or
  wrong token is `403` with its own code, `setup_token_rejected`, so the setup
  screen can say which field to correct; a missing `Origin` stays `403
  forbidden`.
- **Not throttled.** A drawn token is not guessable at any rate, and the floor
  on a chosen one is what keeps it so. RFC-017's limiter is keyed on an
  account, and there is none yet.
- **Forgotten once used.** On success the gate closes in memory and the file
  is deleted; later requests are `409`, as before. A conflict — an account
  created meanwhile from the command line — closes it too. A refused username
  or password leaves it open for the next attempt.
- **Decided at boot.** A boot that finds an account closes setup and deletes a
  stale file. A database that loses its last account while the server runs is
  not reopened until the next start, which is the direction to be wrong in.
- **The command line needs no token.** `account create-admin` already requires
  a shell on the host, which is more than the token proves. Once it has
  created an account, from either `account` command, it closes setup and
  deletes the token file too. A server running beside it keeps its gate open
  in memory until its next setup request, which the database then refuses
  with `409` and which closes it.

**Rejected alternatives.**

- *Binding the compose file to `127.0.0.1` by default.* It closes the window,
  and breaks the ordinary installation: a server on a home network that the
  phone in the next room cannot reach. An operator who wants that can still
  bind it so.
- *Printing the token in the log.* The simplest for the operator, and against
  a rule this repository has no exception for.
- *A new token on every boot.* Safer by nothing — the file is as readable as
  the one before — and a restart would invalidate the token an operator had
  just copied.
- *A header instead of a body field.* The body keeps the token away from
  every layer that might ever be taught to record headers.

The web client's setup screen asks for the token, with a hint naming the file
and the variable, in English and French.

## Decision 2 — security headers on the web client and on shares

**The web client** — every response of the router fallback (`src/webui.rs`),
the shell, its assets and its refusals alike — carries:

- `Content-Security-Policy`:

  ```
  default-src 'self'; script-src 'self' 'sha256-…'; style-src 'self' 'sha256-…';
  img-src 'self' data: blob:; media-src 'self' blob: http: https:;
  font-src 'self' data:; connect-src 'self'; worker-src 'self';
  object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'self'
  ```

  - The built shell has two inline blocks on purpose: the splash screen's
    `<style>` and its ten-second fallback `<script>`. Each is admitted by its
    SHA-256 digest and nothing else is: no `'unsafe-inline'`, no
    `'unsafe-eval'`. The digests are **computed at first use from the shell the
    binary embeds**, so editing either block can never leave a stale digest
    behind, and a build without a client — the placeholder CI embeds — gets a
    policy with none.
  - React sets `style` through the CSSOM, which no `style-src` governs; the
    Vite build has no other inline script or style.
  - `img-src data: blob:`: the stylesheet's inline SVGs, and artwork, which the
    client fetches with its session and shows as a blob.
  - `media-src http: https:`: an internet radio station is played by the
    browser straight from the station's server, whose address a user typed.
    Narrowing it would break radio.
- `X-Frame-Options: SAMEORIGIN`, for browsers that predate `frame-ancestors`.
- `X-Content-Type-Options: nosniff`.
- `Referrer-Policy: same-origin`. Like `no-referrer`, it sends nothing to
  another host — a radio station's server, last.fm while an account is linked
  — which is all a policy is for here. The two differ only on requests to this
  server, where `no-referrer` buys nothing and puts the origin check at risk:
  the Fetch standard serialises `Origin` as `null` for a non-CORS `POST` made
  under it, and the web sign-in and setup refuse every origin but their own.
  Chromium was measured sending the real origin on the client's `fetch` calls
  even so; `same-origin` does not depend on that.

**Public shares** under `/share/` carry `Referrer-Policy: no-referrer`,
`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` and
`Content-Security-Policy: frame-ancestors 'none'`. A share is JSON and audio,
never a page, so its policy names framing only: a `default-src` there would
stop a browser from playing the stream it was opened on.

**Left alone**, on purpose:

- **`/rest`.** The Subsonic contract is frozen; a header a validated client has
  never seen is a change to its wire. It is neither compressed nor given these
  headers.
- **The JSON API** under `/api/v2`. No browser renders it as a document, so
  none of these headers would change what a browser does with it.
- **`/reference`**, see *What is still open*.
- **`Strict-Transport-Security`.** TLS terminates at the operator's proxy; the
  server cannot know whether every name it answers to will keep serving https,
  and a wrong HSTS is a lockout that outlives the mistake. It belongs in the
  proxy's configuration.

The built client was proved to work under this policy by running the built
binary, with the real client embedded, on a spare port and driving it in Chromium: setup with a wrong then the
right token, sign-in, six screens, no policy violation reported, and a frame
from another origin refused by `frame-ancestors`.

## Decision 3 — a copy before every upgrade

When the database has migrations pending, the boot copies it before applying
them (`src/migration_backup.rs`):

- **Only when something will be applied.** A fresh, empty database, and one
  already at this binary's schema, are not copied: a copy on every boot would
  cost the disk for nothing and push out the copies that matter.
- **`VACUUM INTO`**, through the existing `Database::backup_to`, which reads one
  consistent snapshot, write-ahead log included.
- **The key with it.** `waveflow.db` and `instance.key` are one unit; a copy of
  the database without its key restores to a server that refuses to start. The
  copy goes to `<data dir>/pre-migration-<epoch ms>/` in the layout
  `database backup` writes, so `database restore --input <copy>` takes it as it
  is. A database bound to a key is copied only with that key, checked against
  the stored fingerprint the way `database restore` checks it
  (`check_file_instance_key`): a key that is missing or does not match stops
  the boot before anything is applied, since such a pair is no way back and
  the boot would refuse it after migrating anyway. The schema is then left as
  it was for `key adopt` or a restore. Only a database never bound to a key —
  no server has finished a boot on it, so nothing in it was encrypted — is
  copied alone.
- **The newest `WAVEFLOW_MIGRATION_BACKUPS` are kept**, three by default; `0`
  takes none. Older copies made this way are deleted. Pruning recognises only
  `pre-migration-` followed by digits, so a restore's `pre-restore-` copy, a
  key command's `pre-key-` copy, or anything an operator put there is never
  touched. A pruning failure is a warning: the copy this boot needs exists.
- **A copy that cannot be made stops the boot**, before anything is applied,
  and says how to proceed without one. An upgrade that promised a way back
  and went ahead without it would be worse than one that said so.
- **A failed migration names the copy** and the way back: restore it, then
  start the previous release.

The offline key commands are unchanged: they already copy the database as they
found it before migrating (`pre-key-…`).

## Decision 4 — `docs/privacy.md`

One page lists every host the server can contact, what each request carries,
what triggers it and the variable that stops it, written from
`src/outbound.rs`, `src/enrichment.rs`, `src/portrait.rs` and
`src/scrobblers/`. It also lists the three hosts the *browser* may contact
(a radio station, last.fm while linking, the CDN behind `/reference`). It is
linked from `docs/operations.md` and the README.

## What does not change

- The Subsonic façade, byte for byte.
- The native API, except `POST /api/v2/setup`, which now needs `setup_token`
  and can answer `403 setup_token_rejected`.
- `account create-admin`, and every boot of an instance that already has an
  account.

## What is still open

- **`/reference` loads Scalar from `cdn.jsdelivr.net`, unpinned**, in the
  visitor's browser (`utoipa-scalar`'s template). Serving it from the binary,
  or at least pinning the version, would take a third party out of the
  picture; it needs a change to how the page is mounted, and is left for its
  own pull request.
- **The Last.fm and Maloja scrobble adapters read their answers with no size
  cap**, unlike the 2 MiB the metadata clients enforce. Found while writing the
  privacy page; a destination is named by the operator, so this is a
  robustness fix rather than an exposure, and it is left for its own change.
- **FFmpeg is given no `-protocol_whitelist`.** Only local paths reach it, but
  whether a crafted playlist-like media file could make it open a network
  resource rests on FFmpeg's own defaults, which nothing here pins or tests.
- **Second factor (TOTP)** on the web sign-in, still open from RFC-017.
