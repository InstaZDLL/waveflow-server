# AGENTS.md

## Product

This monorepo is being rebuilt as WaveFlow Server v2. The accepted design is `docs/rfcs/RFC-002-waveflow-server-v2.md`; read it before adding a module or public route.

- `/`: one axum binary with SQLite as the only v2 database.
- `web/`: the superseded TanStack/Better Auth v1 front end, kept only until its reusable parts are salvaged. It is not built, served or tested.
- `webapp/`: the embedded React client, built to `webapp/dist` and compiled into the binary by `rust_embed`. Build it before the server, never after.
- The v1 PostgreSQL/JWKS implementation has been removed. Anything predating RFC-002 lives in git history only.

## Commands

```bash
cargo run
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
```

Tests are hermetic and use temporary SQLite databases. No PostgreSQL or Better Auth process is required.

## v2 conventions

- `src/main.rs` only loads config, initializes state, dispatches CLI commands and serves the router. Testable runtime behaviour belongs behind library functions.
- All environment access lives in `src/config.rs`.
- SQL stays in `src/database.rs` or later repository modules. Handlers orchestrate HTTP only.
- SQLite connections enable WAL, foreign keys and `busy_timeout`. All mutations acquire the single process-wide writer gate; libraries may scan concurrently but never create independent SQLite writers.
- Migrations under `migrations-v2/` are immutable once merged. Schema changes always add a new dated migration.
- Tenant isolation is enforced in repository queries through account plus library membership, not only in handlers.
- Public IDs are UUIDs and timestamps are epoch milliseconds.
- A track UUID is stable. Relative path is its current locator; quick hashes are candidates only; a full hash is required to confirm deduplication or relocation.
- Audio files are read-only. Scanner and tag operations never rewrite them.
- Passwords use Argon2id. Opaque tokens are stored hashed. Reversible Subsonic credentials are encrypted with the instance key and never logged.
- HTTP tracing records `uri.path()` only. Never log headers, cookies, query parameters, tokens or passwords.
- `/ready` reports API/database capability. Scan progress and FFmpeg capability have separate status surfaces in later milestones.
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

## Repository hygiene

- Preserve unrelated worktree changes.
- Use `apply_patch` for intentional edits.
- DCO sign-off is mandatory for commits (`git commit -s`).
- Use Conventional Commits with kebab-case scopes and lowercase subjects.
- License is AGPL-3.0-only; the desktop and `waveflow-core` are GPL-3.0-only.
