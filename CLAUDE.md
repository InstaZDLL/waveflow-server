# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**WaveFlow Server v2**: a self-hosted music server in Rust — one axum binary, SQLite as the only database, FFmpeg for transcoding, an OpenSubsonic façade, and a React client compiled into the binary.

The accepted design is [RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md). **Read it before adding a module or a public route.** [`AGENTS.md`](AGENTS.md) holds the short form of the same rules plus the milestone sequence; this file is the deeper reference.

**An RFC's `Status` field never flips.** Every RFC under `docs/rfcs/` says `Proposed` whether it shipped a year ago or not at all, so the header is not evidence about the code — a `Status` line has misled a reader into holding shipped work as pending. What each one carries instead is an **`Implemented by`** line naming the pull requests, which can actually be checked. (Both keys were `Statut` and `Implémentée par` in the five French RFCs until #222 moved the series to English; a document quoting the old spellings is dated, not wrong.) And when it matters, read the code rather than either: the routes registered from `src/app/router.rs` and the dated files under `migrations-v2/` are the only account of what exists.

**Documentation is written in English.** Every RFC, analysis, plan, guide and
handoff under `docs/`, and these instruction files with them. The design
documentation was half French until #222, which is the reason this line exists:
nothing said which language to write in, so each document chose, and the series
of RFCs ended up split down the middle. Two exceptions, both deliberate and both
stating it in a banner at their head: `docs/M3-symfonium-validation.md` and
`docs/M4-handoff.md` stay in French, because translating a dated record rewrites
it. Write a new document in English; if you correct an old one, correct it in
English.

Layout:

- `/` — the server crate (`waveflow-server`). Under `src/`, only `main.rs` and `lib.rs` sit at the root; every module is filed by domain:
  - `app/` — boot, `AppState`, the router and the OpenAPI document; `cli/` — the command line; `config/` — every setting.
  - `api/` — `/api/v2`; `subsonic/` — the OpenSubsonic façade; `services/` — the domain both call (see below).
  - `auth/` — accounts and sessions (`authentication`), the sign-in guard (`sign_in`), native-client OAuth (`oauth`), the setup token, the instance key's cryptography (`security`) and stream tickets.
  - `library/` — reading the music on disk: `scanner/`, `watcher/`, the tag mapper (`tags`), derived identifiers (`pid`), `credit_split`, `link_roots`, `playlist_file` and `lyrics` (the LRC/TTML parsers and the sheets they make).
  - `storage/` — SQLite: `database/` (the pool and the foundation repositories), `catalog/` (the catalogue repository), `sync` (the user-data change journal), `natural_order` (the collation registered on the pool) and `migration_backup`.
  - `remote/` — everything that talks to another host: the outbound client (`outbound`), `enrichment/`, `portrait`, `lrclib`, `podcast` (the feed fetcher) and `scrobblers/`.
  - `http/` — HTTP outside the two API surfaces: `security_headers`, `public_share`, `reference` (the `/reference` page) and `webui`.
  - `media/` — streaming, transcoding, artwork, canvases and `replay_gain`; `quality/` — the audio analysis.
- `webapp/` — the embedded React client, built to `webapp/dist` and compiled in by `rust_embed`. Under `webapp/src/`, only `main.tsx` (the bootstrap: the stylesheets in cascade order, the providers, `createRouter` and the render) sits at the root; everything else is filed by what it is:
  - `app/` — the shell, the rail and its More sheet, and the routes: one file per area under `app/routes/` (`root`, `library`, `listen`, `settings`, `maintenance`), assembled in matching order by `app/route-tree.ts`.
  - `pages/` — one screen per file (a large one is a directory with an `index.tsx`), filed under the area of the route file that mounts it: `pages/root/` (sign-in, `/authorize`, a share link, offline, not found), `pages/library/`, `pages/listen/`, `pages/settings/` (administration included, since it is a section of Settings) and `pages/maintenance/`. `app/routes/<area>.tsx` imports its screens from `pages/<area>/` and nowhere else.
  - `components/` — pieces shared between screens, filed by what they serve: `layout/` (the page frame, its header controls, the icons, the theme picker), `catalog/` (the album grid, the song table, the mix shelves, the folder picker, the play-a-set actions), `menus/` (the popup menu and the action and track menus built on it), `track/` (Properties and its sections), `artist/` (the portrait controls and the credit split), `artwork/` (covers, canvases and the cover's light) and `playlists/`.
  - `api/` — the HTTP client, one file per resource over the session and transport in `client.ts`, re-exported by `index.ts` so the rest of the app imports the directory; then the query keys (`api/queries/`, one file per domain) and the query client.
  - `player/`, `lib/` (logic with no React in it) and `state/` (contexts: preferences, library scope, and i18n, whose strings are one file per domain and locale under `state/i18n/en/` and `state/i18n/fr/`, merged by each locale's `index.ts`).
  - `styles/` — the design tokens (`design-tokens/`) and three stylesheets, `styles.css`, `fluid.css` and `refresh.css`, each an ordered list of `@import`s of its parts in the directory of the same name. The order of the imports is the cascade, so it is kept; a new rule goes into the part it belongs to.

  A unit test sits beside the file it tests. The Playwright suite under `webapp/e2e/` is one spec per subject over a shared mock server in `e2e/fixtures/` (its state, its catalogue and its `/api/v2`), type-checked by `e2e/tsconfig.json` and linted by biome like `src/`. CI holds every file to a size (`scripts/check-webapp-layout.sh`): 600 lines for a `.ts`/`.tsx` file under `src/`, 800 for a stylesheet, 1,000 for a unit test and for anything under `e2e/`. Past that, split along the file's seams; an exception is listed in the script with its reason.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) is a separate repository. It consumes this API but is not part of this codebase.

The v1 PostgreSQL/JWKS server and its TanStack/Better Auth front end were removed once M4 landed. If you find a reference to `/api/v1`, `src/db.rs`, `src/apply.rs`, the sync log or the Postgres `migrations/` directory, it is stale documentation — the code lives in git history only.

## Commands

```bash
cargo run                              # migrate, then serve on WAVEFLOW_BIND
cargo fmt --all --check                # CI gate
cargo clippy --all-targets --all-features -- -D warnings   # CI gate; warnings are errors
cargo test --all-features              # hermetic; temporary SQLite databases
cargo test --all-features --test subsonic_contract <name>   # one test, in one target
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features   # CI gate; a link that resolves nowhere fails
sh scripts/check-rust-layout.sh        # CI gate; file sizes and no inline unit tests

bun --cwd=webapp install
bun run build                          # webapp first, then cargo — the order matters
```

Tests need no service container: `test_app()` builds a `Config` over a `TempDir` and the suite drives the same router `main` serves. FFmpeg and ffprobe must be on `PATH`.

The integration suite is twenty-three targets under `tests/`, one per subject — `auth`, `scanner`, `identity`, `catalog`, `subsonic_contract`, `subsonic_browse`, `subsonic_fields`, `subsonic_methods`, `native_api`, `sync`, `media`, `service`, `cli`, `scrobbling`, `enrichment`, `credits`, `quality`, `uploads`, `canvas`, `discovery`, `smart_playlists`, `podcasts`, `migrations` — nearly all of them sharing `tests/support/mod.rs`. `migrations` is the exception and deliberately so: every other target starts from `test_app()`, which builds its database from nothing, and that is exactly the path an upgrade does not take. It seeds a database at each **released** schema, boots the server on it, and reads the rows back through the services — and it is where anything else that needs a database *behind* this binary belongs, such as proving that a key change reads without migrating and copies the pair aside before it does. `autotests = false`, so each is a target only because `Cargo.toml` declares it: a new file under `tests/` is compiled by nothing until a `[[test]]` entry names it, and the count above is worth checking against that list rather than trusted. `support/mod.rs` is the exception that proves it — shared module code, declared nowhere, a target never. A target too large for one file becomes a directory and stays one target: its `[[test]]` entry points at `tests/<name>/main.rs`, which reaches the shared module through `#[path = "../support/mod.rs"]`, keeps the imports and helpers its files share, and declares one file per surface with `mod`; each of those starts with `use super::*;`. The target's name does not change, so `cargo test --test native_api <name>` still finds a test, now under its module (`playlists::…`) — and a new file in such a directory is compiled by nothing until `main.rs` declares it, the same trap one level down. File a new test by the surface it exercises, and check that by what it calls rather than by what its name says.

Unit tests never sit inline in the code. A module with unit tests is a directory: the code in `foo/mod.rs`, which declares `#[cfg(test)] mod tests;`, and the tests in `foo/tests.rs` beside it, reaching private items through `use super::*;` — never a `foo.rs` with a `foo/` beside it. CI holds the sizes too (`scripts/check-rust-layout.sh`): a source file under `src/` stays within 800 lines, a `tests.rs` under `src/` or any file under `tests/` within 1,000, and no file under `src/` opens `mod tests {`. Past a limit, split along the file's seams into a directory, as the restructuring did, rather than raising it; the script's allowlist holds one file, a single test that cannot be cut without changing it, and says why.

`cargo build` works without a client build — `build.rs` embeds a placeholder page when `webapp/dist` is absent. A real `vite build` overwrites it.

## Architecture & conventions

- **Library/binary split.** `src/main.rs` only loads config, dispatches CLI commands and serves. The router is built by `waveflow_server::app(&config, state)` in `src/app/router.rs`, re-exported by `src/lib.rs`, so tests spawn the same app in-process. Put logic behind `app()`, not in `main`.
- **`AppState`** (`src/app/state.rs`) holds the shared singletons: `db`, `auth`, `secret_box`, `scanner`, `media`, `services`, plus config values copied in (`artwork_dir`, `public_url`, `stream_ticket_ttl`). Add new singletons there.
- **Three directories carry the surfaces.** `src/api/` is `/api/v2`, one module per resource plus `error`, `access` and `web_session`; `src/subsonic/` is the façade, one module per method family; `src/services/` is the domain, several `impl DomainServices` blocks split by domain. Each `mod.rs` keeps only what its children share — the router or dispatch, the cross-cutting types, and for `services` the SQL projection macros, which sit ahead of the `mod` declarations so their textual scope reaches every child. File a new handler under the resource it serves, not in `mod.rs`. Everything else in `src/` is filed by domain (see Layout): a new module goes under the parent whose domain it serves — `auth/`, `library/`, `storage/`, `remote/`, `http/`, `media/` — never at the root, and each parent's `mod.rs` holds only its `//!` and the `pub mod` lines.
- **All environment access lives in `src/config/`.** Every tunable is a field on `Config` with its env var documented on it.
- **No SQL in handlers.** SQL belongs in `src/storage/database/`, `src/storage/catalog/` or `src/services/`; handlers orchestrate HTTP only.
- **One set of domain services.** `DomainServices` (`src/services/`) is the single implementation behind the native API, the Subsonic façade and the web client. A mutation reachable from two surfaces must call the same method — that convergence is the point of M4, and duplicating logic per surface is how the two drift.
- **Tenancy is enforced in the queries**, through `library_member`, not in handlers. The shared projections are `song_select!` / `album_select!` / `artist_select!` macros that `concat!` into literals: sqlx only accepts static SQL, so composing them stays injection-proof by construction. The first bind is always the user id.
- **404 blurs everything.** A resource that is missing, and one that belongs to another account, answer identically. `ServiceError::Forbidden` maps onto 404 for that reason. Never confirm existence to someone not entitled to it.
- **SQLite discipline.** WAL, foreign keys, `busy_timeout`. Every mutation takes the process-wide writer gate (`db.writer_guard()`); libraries may scan concurrently but never open independent writers. `NULL` sorts first in SQLite — order explicitly with `NULLS LAST` wherever a missing tag would otherwise jump the queue.
- **Migrations are immutable once merged.** They are embedded at compile time and checksummed, so editing an applied migration makes the server refuse to start. Schema changes are new dated files under `migrations-v2/`.
- **Identifiers and time.** Public ids are UUIDs; timestamps are Unix epoch milliseconds. Album and artist ids are *derived* UUIDs (version 8, RFC 9562 §5.8): the identity engine in `src/library/pid/` hashes the tags that name the entity, under the configurable spec `WAVEFLOW_PID_ALBUM`/`WAVEFLOW_PID_ARTIST`, so the same files answer the same ids on a fresh install. Track ids stay drawn at random — six tables cascade off them, and a scan matches a track by path, then by content hash, then by the relocation hint in `track.pid`. That hint is the track spec evaluated over the file's tags, and it stays a hint: it is asked only when the first two have nothing to say — a file both moved and re-encoded — and it moves nothing unless it names exactly one track whose file has genuinely disappeared. Changing `WAVEFLOW_PID_TRACK` drops every stored hint on the next boot: a hint written under one spec must never be compared against a lookup computed under another, because two specs can evaluate to the same string and the hint would then hand a new file an unrelated track's identity. The next ordinary scan writes them back from its skip path, so clearing them costs no rescan — and a track spec change still re-identifies nothing. Changing a spec re-identifies everything it governs: the next boot notices, schedules a full rescan of every library, and album favourites do not survive it. A credit decision (RFC-015) is narrower: splitting a joined credit such as `Zedd, Alessia Cara` re-identifies only the artists it names. The album display string and the track hint read the tag mapper's names, before any decision, so no album or track id moves, and favourites on the joined artist are carried onto its parts through the journal.
- **Credentials.** Web passwords use Argon2id. Access, refresh, API and OAuth codes are stored as SHA-256 hashes only. The dedicated Subsonic password is encrypted with ChaCha20-Poly1305 under the instance key. Back up `data/waveflow.db` and `data/instance.key` together — encrypted values cannot be recovered with one without the other.
- **Browser playback uses stream tickets** (`src/auth/stream_ticket/`). `<audio src>` cannot send an `Authorization` header, so the client exchanges its session for an AEAD-sealed `(user, track, expiry)` and plays from `/api/v2/stream/{ticket}`. Access is re-checked on every redemption, so a ticket cannot outlive the membership that justified it. The TTL is an hour rather than seconds because the browser reuses that URL for every range request while seeking.
- **Native clients use Authorization Code + PKCE** (`src/auth/oauth/`). S256 only; redirect targets restricted to loopback, https or a reverse-domain private scheme. A code is spent by its first presentation whatever the outcome.
- **The web client is a router fallback** (`src/http/webui.rs`). Anything not claimed by an API route is a built asset or a client-side route. Paths under `api/`, `rest/`, `share/` and the probes are excluded, so a mistyped API call stays a JSON 404 instead of returning HTML.
- **Audio files are read-only.** The scanner and tag operations never rewrite them.
- **The real quality of a file is measured, never read** ([RFC-016](docs/rfcs/RFC-016-audio-quality-analysis.md)). `src/quality/` decodes a lossless file through FFmpeg and runs the desktop's wall search on it, ported from `waveflow-core` and held to it by a test; `src/services/quality/` decides which files are measured and what a measurement means. The analysis runs in the background, `WAVEFLOW_QUALITY_PARALLELISM` files at a time (one by default, zero turns it off), and starts no file while a scan runs. A row in `track_quality` is current only for the content, the analyser version and the FFmpeg build that made it; boot deletes the others, so a read can call a row current on its `full_hash` alone. The verdict is SQL computed where it is read, never a column.
- **The Subsonic contract is frozen** for v2.0-beta — see the "Frozen Subsonic v2.0-beta contract" section of RFC-002 and [`docs/subsonic-compatibility.md`](docs/subsonic-compatibility.md). Changing observable wire behaviour there risks the clients already validated against it.
- **Logging.** `tracing` + `RUST_LOG`; `WAVEFLOW_LOG_FORMAT=json` for prod. Traces record `uri.path()` only, with share tokens and stream tickets redacted. Never log headers, query strings, tokens or passwords.

## Contributing rules that affect commits

- **DCO sign-off is mandatory** — `git commit -s`. CI rejects unsigned PRs.
- **Conventional Commits** with kebab-case scopes and lowercase subjects, e.g. `feat(api): add native album endpoints`.
- License is **AGPL-3.0-only** (the server hosts a network service); the desktop app and `waveflow-core` are GPL-3.0-only.
- Do not begin a later milestone until the previous one's tests and release gate pass. `v2.0-beta` additionally requires the Symfonium validation tracked in [`docs/M3-symfonium-validation.md`](docs/M3-symfonium-validation.md).
