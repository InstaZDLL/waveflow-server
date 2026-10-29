# AGENTS.md

## Product

This repository is WaveFlow Server v2. The accepted design is `docs/rfcs/RFC-002-waveflow-server-v2.md`; read it before adding a module or public route.

- `/`: one axum binary with SQLite as the only v2 database.
- `webapp/`: the embedded React client, built to `webapp/dist` and compiled into the binary by `rust_embed`. Build it before the server, never after.
- The v1 PostgreSQL/JWKS server and the TanStack/Better Auth front end that fronted it have been removed; both live in git history only.

## Commands

```bash
cargo run
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
```

Tests are hermetic and use temporary SQLite databases. No PostgreSQL or Better Auth process is required.

Unit tests live in `tests.rs` beside their module (`foo/mod.rs` declares `#[cfg(test)] mod tests;`), never inline. Integration targets are declared in `Cargo.toml`; a large one is a directory whose `main.rs` declares one file per subject. CI runs `sh scripts/check-rust-layout.sh`: no source file under `src/` past 800 lines, no `tests.rs` or file under `tests/` past 1,000, no inline `mod tests {`. It also runs `cargo doc --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings"`.

## v2 conventions

- `src/main.rs` only loads config, initializes state, dispatches CLI commands and serves the router. Testable runtime behaviour belongs behind library functions.
- `src/` is filed by domain; only `main.rs` and `lib.rs` sit at its root. `app/` (boot, state, router), `cli/`, `config/`; the surfaces `api/` and `subsonic/` over the one domain `services/`; `auth/` (sessions, sign-in guard, OAuth, setup token, instance-key cryptography, stream tickets); `library/` (scanner, watcher, tags, PIDs, credit splitting, playlist files, lyrics parsing); `storage/` (database, catalogue, sync journal, collation, pre-migration copy); `remote/` (outbound client, enrichment, portraits, LRCLIB, podcast feeds, scrobblers); `http/` (security headers, public share, `/reference`, web client); `media/`; `quality/`. A new module goes under the parent whose domain it serves, never at the root.
- All environment access lives in `src/config/`.
- SQL stays in `src/storage/database/`, `src/storage/catalog/` or `src/services/`. Handlers orchestrate HTTP only.
- SQLite connections enable WAL, foreign keys and `busy_timeout`. All mutations acquire the single process-wide writer gate; libraries may scan concurrently but never create independent SQLite writers.
- Migrations under `migrations-v2/` are immutable once merged. Schema changes always add a new dated migration.
- Tenant isolation is enforced in repository queries through account plus library membership, not only in handlers.
- Public IDs are UUIDs and timestamps are epoch milliseconds.
- A track UUID is stable. Relative path is its current locator; quick hashes are candidates only; a full hash is required to confirm deduplication or relocation.
- Audio files are read-only. Scanner and tag operations never rewrite them.
- Passwords use Argon2id. Opaque tokens are stored hashed. Reversible Subsonic credentials are encrypted with the instance key and never logged.
- HTTP tracing records `uri.path()` only. Never log headers, cookies, query parameters, tokens or passwords.
- `/ready` reports API/database capability. Scan progress has its own surface (`/api/v2/scans/{scan_id}/events`), not the probe.
- Web, native and Subsonic mutations must call the same domain services.

## Release sequence

1. M0: SQLite, local auth, CLI, probes.
2. M1: authoritative scanner, catalogue, FTS5, artwork.
3. M2: original streaming, FFmpeg transcode and cache.
4. M3: tested OpenSubsonic façade and v2.0-beta.
5. M4: native API/sync, embedded functional web player and v2.0 stable.
6. M5: conservative local/server reconciliation.
7. M6: premium studio-nocturne web finish for v2.1.

Do not begin a later milestone until the previous milestone's tests and release gate pass.

All seven closed on 2026-08-18 (`docs/history/M4-handoff.md`) and were released as `2.0.0-beta.0` and `2.0.0-beta.1`; `v2.0` stable is not tagged yet. Work since is planned in GitHub issues and in `docs/plans/`, and every RFC from RFC-007 on records what implemented it.

## Repository hygiene

- Preserve unrelated worktree changes.
- Use `apply_patch` for intentional edits.
- DCO sign-off is mandatory for commits (`git commit -s`).
- Use Conventional Commits with kebab-case scopes and lowercase subjects.
- License is AGPL-3.0-only; the desktop and `waveflow-core` are GPL-3.0-only.
