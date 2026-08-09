# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**WaveFlow Server v2**: a self-hosted music server in Rust — one axum binary, SQLite as the only database, FFmpeg for transcoding, an OpenSubsonic façade, and a React client compiled into the binary.

The accepted design is [RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md). **Read it before adding a module or a public route.** [`AGENTS.md`](AGENTS.md) holds the short form of the same rules plus the milestone sequence; this file is the deeper reference.

Layout:

- `/` — the server crate (`waveflow-server`).
- `webapp/` — the embedded React client, built to `webapp/dist` and compiled in by `rust_embed`.

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) is a separate repository. It consumes this API but is not part of this codebase.

The v1 PostgreSQL/JWKS server and its TanStack/Better Auth front end were removed once M4 landed. If you find a reference to `/api/v1`, `src/db.rs`, `src/apply.rs`, the sync log or the Postgres `migrations/` directory, it is stale documentation — the code lives in git history only.

## Commands

```bash
cargo run                              # migrate, then serve on WAVEFLOW_BIND
cargo fmt --all --check                # CI gate
cargo clippy --all-targets --all-features -- -D warnings   # CI gate; warnings are errors
cargo test --all-features              # hermetic; temporary SQLite databases
cargo test --all-features --test v2_foundations <name>     # one test

bun --cwd=webapp install
bun run build                          # webapp first, then cargo — the order matters
```

Tests need no service container: `test_app()` builds a `Config` over a `TempDir` and the suite drives the same router `main` serves. FFmpeg and ffprobe must be on `PATH`.

`cargo build` works without a client build — `build.rs` embeds a placeholder page when `webapp/dist` is absent. A real `vite build` overwrites it.

## Architecture & conventions

- **Library/binary split.** `src/main.rs` only loads config, dispatches CLI commands and serves. The router is built by `waveflow_server::app(&config, state)` in `src/lib.rs`, so tests spawn the same app in-process. Put logic behind `app()`, not in `main`.
- **`AppState`** (`src/lib.rs`) holds the shared singletons: `db`, `auth`, `secret_box`, `scanner`, `media`, `services`, plus config values copied in (`artwork_dir`, `public_url`, `stream_ticket_ttl`). Add new singletons there.
- **All environment access lives in `src/config.rs`.** Every tunable is a field on `Config` with its env var documented on it.
- **No SQL in handlers.** SQL belongs in `src/database.rs`, `src/catalog.rs` or `src/services.rs`; handlers orchestrate HTTP only.
- **One set of domain services.** `DomainServices` (`src/services.rs`) is the single implementation behind the native API, the Subsonic façade and the web client. A mutation reachable from two surfaces must call the same method — that convergence is the point of M4, and duplicating logic per surface is how the two drift.
- **Tenancy is enforced in the queries**, through `library_member`, not in handlers. The shared projections are `song_select!` / `album_select!` / `artist_select!` macros that `concat!` into literals: sqlx only accepts static SQL, so composing them stays injection-proof by construction. The first bind is always the user id.
- **404 blurs everything.** A resource that is missing, and one that belongs to another account, answer identically. `ServiceError::Forbidden` maps onto 404 for that reason. Never confirm existence to someone not entitled to it.
- **SQLite discipline.** WAL, foreign keys, `busy_timeout`. Every mutation takes the process-wide writer gate (`db.writer_guard()`); libraries may scan concurrently but never open independent writers. `NULL` sorts first in SQLite — order explicitly with `NULLS LAST` wherever a missing tag would otherwise jump the queue.
- **Migrations are immutable once merged.** They are embedded at compile time and checksummed, so editing an applied migration makes the server refuse to start. Schema changes are new dated files under `migrations-v2/`.
- **Identifiers and time.** Public ids are UUIDs; timestamps are Unix epoch milliseconds.
- **Credentials.** Web passwords use Argon2id. Access, refresh, API and OAuth codes are stored as SHA-256 hashes only. The dedicated Subsonic password is encrypted with ChaCha20-Poly1305 under the instance key. Back up `data/waveflow.db` and `data/instance.key` together — encrypted values cannot be recovered with one without the other.
- **Browser playback uses stream tickets** (`src/stream_ticket.rs`). `<audio src>` cannot send an `Authorization` header, so the client exchanges its session for an AEAD-sealed `(user, track, expiry)` and plays from `/api/v2/stream/{ticket}`. Access is re-checked on every redemption, so a ticket cannot outlive the membership that justified it. The TTL is an hour rather than seconds because the browser reuses that URL for every range request while seeking.
- **Native clients use Authorization Code + PKCE** (`src/oauth.rs`). S256 only; redirect targets restricted to loopback, https or a reverse-domain private scheme. A code is spent by its first presentation whatever the outcome.
- **The web client is a router fallback** (`src/webui.rs`). Anything not claimed by an API route is a built asset or a client-side route. Paths under `api/`, `rest/`, `share/` and the probes are excluded, so a mistyped API call stays a JSON 404 instead of returning HTML.
- **Audio files are read-only.** The scanner and tag operations never rewrite them.
- **The Subsonic contract is frozen** for v2.0-beta — see the "Frozen Subsonic v2.0-beta contract" section of RFC-002 and [`docs/subsonic-compatibility.md`](docs/subsonic-compatibility.md). Changing observable wire behaviour there risks the clients already validated against it.
- **Logging.** `tracing` + `RUST_LOG`; `WAVEFLOW_LOG_FORMAT=json` for prod. Traces record `uri.path()` only, with share tokens and stream tickets redacted. Never log headers, query strings, tokens or passwords.

## Contributing rules that affect commits

- **DCO sign-off is mandatory** — `git commit -s`. CI rejects unsigned PRs.
- **Conventional Commits** with kebab-case scopes and lowercase subjects, e.g. `feat(api): add native album endpoints`.
- License is **AGPL-3.0-only** (the server hosts a network service); the desktop app and `waveflow-core` are GPL-3.0-only.
- Do not begin a later milestone until the previous one's tests and release gate pass. `v2.0-beta` additionally requires the Symfonium validation tracked in [`docs/M3-symfonium-validation.md`](docs/M3-symfonium-validation.md).
