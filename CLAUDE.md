# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`waveflow-server` is the self-hosted backend for [WaveFlow](https://github.com/InstaZDLL/WaveFlow): an axum (Rust) + PostgreSQL service for multi-device library sync, browser playback, and public shareable playlists. It is a single binary crate (`name = "waveflow-server"`), a Linux/macOS daemon by design — *not* the desktop binary.

The authoritative design lives in [RFC-001](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md) in the main repo. **Read it before adding a new module.** Work is phased against the main repo's Phase 1 milestone; the README's status line tracks the current sub-phase.

## Commands

```bash
cargo run                              # connect pool, run migrations, serve on WAVEFLOW_BIND
cargo fmt --all --check                # CI gate
cargo clippy --all-targets --all-features -- -D warnings   # CI gate; warnings are errors
cargo check --all-targets --all-features
cargo test --all-features              # needs a reachable Postgres on DATABASE_URL
cargo test ready                       # run a single test file / filter by name
```

`cp .env.example .env` first for local dev (`dotenvy` loads `.env` best-effort at boot; release deploys use real env vars). `Config::from_env` (`src/config.rs`) is the single source of truth for the env surface — every tunable is a field there with its env var documented in the doc comment.

### Tests need real Postgres

The integration suite uses `#[sqlx::test]`, which creates a fresh per-test database from `DATABASE_URL`, runs the migrations, and drops it on exit — no manual fixtures. Spin one up:

```bash
docker run --name waveflow-pg -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=postgres -p 5432:5432 -d postgres:17
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test
```

CI runs the full suite only on Linux (service container); the Windows leg is a compile/clippy guard only (Actions service containers are Linux-only).

## Architecture & conventions

- **Library/binary split.** `src/main.rs` is only runtime plumbing (load `.env`, init tracing, connect pool, run migrations, bind, serve with graceful shutdown). The router is built by `waveflow_server::app(config, state)` in `src/lib.rs` so integration tests spawn the *same* app in-process (`tests/support.rs::spawn_app`). Put logic behind `app()`, not in `main`.
- **`AppState`** (`src/lib.rs`) holds the shared singletons threaded through every handler — currently just the `PgPool` (cheap to clone, `Arc`-backed). Add new singletons here.
- **API is one file per resource** under `src/api/`, each exposing a `router()` merged in `src/api/mod.rs`. `/health` (liveness, no DB) and `/ready` (DB-aware readiness) are unversioned infra probes; every real resource mounts under `/api/v1/`.
- **No SQL in handlers.** SQL lives in the DB layer (`src/db.rs`) or in a `waveflow-core::repository::postgres::*` method; handlers stay pure HTTP orchestration. `db::ping` (`/ready`'s `SELECT 1`) and `db::users::create` are the in-tree pattern; everything tenant-scoped goes through `PostgresProfileRepository::*_for_user`. This mirrors the desktop's Tauri-command ↔ `waveflow-core` boundary.
- **Tenancy is enforced at the storage layer.** Server handlers under `/api/v1/*` extract `UserId` from the `require_user_id` middleware and call `*_for_user` methods only — never the single-tenant trait surface. `PostgresProfileRepository` deliberately does NOT implement `ProfileRepository`, so the compiler stops a careless `list_all()` from leaking another tenant's rows. Apply the same pattern when adding library / track / playlist repositories.
- **Auth: JWT + dev shim, transitioning.** `middleware::authenticate` runs JWT-first when [`AppState::jwt_verifier`] is configured: verify the Bearer, then `db::users::find_or_provision_by_external_id(state.db, &sub, now_ms)` to lazy-onboard the user on first request (Phase 1.c.3a — a valid signature is the authoritative onboarding signal, so no separate `POST /api/v1/users` is needed after Better Auth signup). Falls back to the legacy `X-User-Id` shim when `dev_auth_enabled`; returns 503 when neither path is configured (production gate). Phase 1.d.2 deletes the shim branch entirely.
- **Don't leak DB errors to unauthenticated probes.** `/ready` logs the sqlx error via `tracing::warn!` but returns a fixed sentinel body (`{status, db}`) so a load balancer never sees the connection-URL host or credentials. Apply the same discipline to any other unauthenticated endpoint.
- **Migrations are immutable once merged.** They're embedded at compile time via `sqlx::migrate!("./migrations")` (`db::MIGRATOR`); the `_sqlx_migrations` table stores each file's checksum, so editing an applied migration makes the server refuse to start. Schema changes = a new dated migration file (`YYYYMMDDHHMMSS_name.sql`). Boot applies pending migrations *before* opening the listener, which is what makes `/ready` trustworthy.
- **Schema parity with the desktop SQLite migrations.** Postgres tables mirror the shapes in the desktop repo's `src-tauri/migrations/app/` so `PostgresProfileRepository` and `SqliteProfileRepository` (in `waveflow-core`) satisfy the same trait against identical rows. Keep types compatible (e.g. `BIGSERIAL` ↔ SQLite `INTEGER PK`, epoch-millis `BIGINT` for timestamps).
- **`waveflow-core` is a git dependency pinned by `rev`** (not branch) in `Cargo.toml` for reproducible builds — bump the rev in-tree to pick up a new core release. It provides the repository traits + Postgres impls.
- **Error handling:** `anyhow` at the binary edges (`main`, `Config::from_env`, tests); prefer `thiserror`-typed errors inside modules once the domain is richer than "boot failed."
- **Logging:** `tracing` + `RUST_LOG` filter; `WAVEFLOW_LOG_FORMAT=json` switches to JSON (CI/prod), pretty otherwise (dev). Every request carries an `x-request-id` (generated if absent) as a structured span field — never log full headers (they'd leak Authorization/Cookie).

## Contributing rules that affect commits

- **DCO sign-off is mandatory** — every commit needs a `Signed-off-by:` trailer (`git commit -s`). CI rejects unsigned PRs. The git `user.email` must match a verified GitHub identity. Fix a miss with `git commit --amend -s --no-edit`.
- **Conventional Commits** with kebab-case scopes, lowercase subject — e.g. `feat(api): add /api/v1/playlists endpoint`, `refactor(db): factor pool wiring out of main`.
- License is **AGPL-3.0-only** (the server hosts a network service); `waveflow-core` and the desktop app are GPL-3.0-only.
