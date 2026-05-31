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
- **`AppState`** (`src/lib.rs`) holds the shared singletons threaded through every handler. Cheap to clone — every field is `Arc`-backed. Fields today: `db: PgPool`, `jwt_verifier: Arc<JwtVerifier>` (Phase 1.d.1), `stream_ctx: Option<Arc<StreamCtx>>` (Phase 1.e — `None` disables streaming), `sync: SyncHub` (Phase 1.f — broadcast `Sender` + `DashMap<(user_id, device_id), AckEntry>` + flush/compaction tasks; tests construct via `SyncHub::for_tests(pool)` which skips the background loops so `flush_acks` / `compact_once` can be driven by hand for determinism). Add new singletons here.
- **API is one file per resource** under `src/api/`, each exposing a `router()` merged in `src/api/mod.rs`. `/health` (liveness, no DB) and `/ready` (DB-aware readiness) are unversioned infra probes; every real resource mounts under `/api/v1/`.
- **No SQL in handlers.** SQL lives in the DB layer (`src/db.rs`) or in a `waveflow-core::repository::postgres::*` method; handlers stay pure HTTP orchestration. `db::ping` (`/ready`'s `SELECT 1`) and `db::users::create` are the in-tree pattern; everything tenant-scoped goes through `PostgresProfileRepository::*_for_user`. This mirrors the desktop's Tauri-command ↔ `waveflow-core` boundary.
- **Tenancy is enforced at the storage layer.** Server handlers under `/api/v1/*` extract `UserId` from the `middleware::authenticate` middleware and call `*_for_user` methods only — never the single-tenant trait surface. `PostgresProfileRepository` deliberately does NOT implement `ProfileRepository`, so the compiler stops a careless `list_all()` from leaking another tenant's rows. Apply the same pattern when adding library / track / playlist repositories.
- **Auth: JWT-only (Phase 1.d.2).** `middleware::authenticate` requires a Bearer JWT signed by the upstream Better Auth issuer. The middleware verifies the token via [`AppState::jwt_verifier`], then `db::users::find_or_provision_by_external_id(state.db, &sub, now_ms)` lazy-onboards the user on first request — a valid signature is the authoritative onboarding signal, so no separate `POST /api/v1/users` exists. Boot requires the full `WAVEFLOW_JWT_*` triple (`_JWKS_URL` / `_ISSUER` / `_AUDIENCE`); the legacy `X-User-Id` dev shim retired alongside `WAVEFLOW_DEV_AUTH`.
- **Streaming (Phase 1.e).** Two endpoints: `POST /api/v1/profiles/{p}/libraries/{l}/tracks/{t}/stream-url` (JWT-authed) verifies tenant ownership and signs a short-lived (≤ 60 s) URL via [`stream_token::mint`]; `GET /api/v1/stream/{token}` is mounted OUTSIDE the JWT layer because browsers can't attach a Bearer to `<audio src>` — the HMAC in the token IS the auth. The stream handler canonicalises `<music_root>/<file_path>` and refuses anything resolving outside `WAVEFLOW_MUSIC_ROOT` (path-traversal guard via `std::fs::canonicalize` + prefix check). Range requests handled in-process: `Accept-Ranges: bytes`, 206 + `Content-Range` on partial, 416 on unsatisfiable. Both endpoints answer 503 when streaming is disabled at boot (`WAVEFLOW_MUSIC_ROOT` + `WAVEFLOW_STREAM_SECRET` unset). Until 1.f's sync ships, files must be placed manually under the music root.
- **Sync (Phase 1.f).** Append-only `sync_op` log keyed on `BIGSERIAL id`; per-`(user, device)` UNIQUEs on `operation_id` (idempotency) and `lamport_ts` (monotonicity). Three REST routes + one WebSocket under `/api/v1/sync/*`: `POST /ops` push (idempotent replay via `ON CONFLICT operation_id DO NOTHING`, 409 + `stored_max` on lamport regression), `GET /ops?since=N` pull (410 + `compacted_up_to` when `0 < since < watermark`; `since=0` is the bootstrap path and always skips the guard so fresh devices can converge from an empty cursor — read-only, never advances the cursor), `POST /ack` (the **only** path that writes `device_sync_cursor`, buffered in memory + flushed every 5 s), `GET /ws?device_id=…` WebSocket fan-out (one broadcast channel, per-frame `user_id` filter so cross-tenant ops never leak). Live state in `sync::SyncHub`: tokio `broadcast::Sender` + `DashMap<(user_id, device_id), AckEntry>` + daily compaction task that flushes ACKs first then collapses superseded ops in the same Postgres transaction as the watermark UPSERT. Watermark monotonic by UPSERT `WHERE` clause. Stale devices (`last_seen_at < now - 90 d`) skipped from the compaction MIN. Tests drive `SyncHub::for_tests` (no background tasks) and call `flush_acks` / `compact_once` directly for determinism.
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
