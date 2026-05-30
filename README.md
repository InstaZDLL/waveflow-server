# waveflow-server

Self-hosted backend for [WaveFlow](https://github.com/InstaZDLL/WaveFlow). Powers multi-device library sync, browser playback, public shareable playlists, and (later) the mobile app.

> **Status:** Phase 1.b.4 — tenant-scoped profile CRUD landed (`POST /api/v1/users`, full `/api/v1/profiles/*` with the dev `X-User-Id` header shim). Phase 1.d will swap the shim for JWT verification against Better Auth's JWKS. Track progress against the Phase 1 milestone on the main repo.

## Architecture

The full architectural decisions — server stack, web app stack, auth boundary, sync protocol, streaming, delivery plan — live in [`waveflow/docs/rfcs/RFC-001-waveflow-server.md`](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md). Read that before opening a PR here.

TL;DR:

- **Stack:** axum (Rust) + PostgreSQL + sqlx + utoipa (OpenAPI) + tokio-tungstenite (WebSocket).
- **Reuses `waveflow-core`** from the main `waveflow` repo as a git dependency for the first months, switching to a crates.io release once the public API stabilizes.
- **Auth boundary:** JWT signed by `waveflow-web` (Better Auth), verified here against the JWKS endpoint. Server never touches credentials.
- **Sync:** append-only ops log with a server-assigned monotonic sequence + tombstones. WebSocket fan-out via Postgres `LISTEN / NOTIFY`.

## Local development

> Pre-requirement: Rust stable (1.94+, the MSRV inherited from sqlx 0.9) and a Postgres ≥ 15 instance reachable on `DATABASE_URL`. Both plain TCP and TLS (`sslmode=require` / `verify-ca` / `verify-full`) connection strings are supported — managed providers like Neon, Supabase, Prisma Accelerate and RDS work out of the box.

```bash
git clone https://github.com/InstaZDLL/waveflow-server
cd waveflow-server
cp .env.example .env
# Edit DATABASE_URL if needed, then:
cargo run
```

`cargo run` connects to Postgres, applies pending migrations, opens the listener on `WAVEFLOW_BIND`, and serves two probes today:

- `GET /health` — liveness, always returns `200 {status, version}`. Doesn't touch the DB.
- `GET /ready` — readiness, `200 {status: "ready", db: "ok"}` when `SELECT 1` round-trips, `503 {status: "not_ready", db: "unavailable"}` otherwise. The sqlx error detail stays in the `tracing::warn!` log so an unauthenticated probe (e.g. a load balancer) doesn't see the connection-URL host or credentials.
- `GET /openapi.json` — OpenAPI 3.1 spec built from the handlers that carry both a `#[utoipa::path(...)]` annotation and a `routes!()` registration on the per-module `OpenApiRouter`. A plain `Router::route()` would mount the handler but leave it absent from the spec, so make sure new endpoints follow the same `routes!()` pattern as `/health` and `/ready`.
- `GET /reference` — [Scalar](https://github.com/scalar/scalar) API reference UI. Modern, dark-mode-native, integrated search. The OpenAPI spec it renders is the same one served at `/openapi.json`.
- `POST /api/v1/users` — mint a user row, returns `{id}`. Gated by the dev-auth shim (see below).
- `/api/v1/profiles/*` — full CRUD scoped to the calling user via the `X-User-Id` header. Tenant isolation enforced at the storage layer (`PostgresProfileRepository::*_for_user`), not just at the handler. `DELETE` refuses 409 if it would leave the user with zero profiles — same invariant the desktop's selector enforces client-side.

> ⚠️ **Dev auth shim — production-off by default.** `/api/v1/*` returns `503 Service Unavailable` until `WAVEFLOW_DEV_AUTH=1` is set explicitly. With the gate on, every data route reads its tenant id from a forgeable `X-User-Id` request header — fine for local dev against a private Postgres, **never safe to expose on the public internet**. Phase 1.d retires both the flag and the shim by replacing the middleware with JWT verification against Better Auth's JWKS endpoint.

### Running the tests

The integration suite uses `#[sqlx::test]`, which spins up a per-test database from `DATABASE_URL` and drops it on exit. Point it at any reachable Postgres instance:

```bash
docker run --name waveflow-pg \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=postgres -p 5432:5432 -d postgres:17
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test
```

CI provisions the same shape via a GitHub Actions service container (see `.github/workflows/ci.yml`).

## Repository layout

```text
.
├── Cargo.toml              # single binary crate, name = `waveflow-server`
├── src/
│   ├── main.rs             # entrypoint — connect pool, run migrations, serve
│   ├── lib.rs              # router + AppState (PgPool)
│   ├── config.rs           # `Config::from_env` — single env-reading entrypoint
│   ├── db.rs               # PgPool wiring + embedded migration runner
│   └── api/                # one file per resource (/health, /ready, …)
├── migrations/             # sqlx migrations (immutable once merged)
├── tests/                  # integration tests (real Postgres via sqlx::test)
├── .env.example            # template — see `Config` for the full env surface
├── .github/workflows/
│   ├── ci.yml              # cargo check / test / clippy / fmt (+ pg service)
│   ├── codeql.yml          # security scanning (rust + actions)
│   └── dco.yml             # DCO sign-off check on PRs
├── CONTRIBUTING.md         # DCO sign-off + commit conventions
├── LICENSE                 # AGPL-3.0 (server hosts a network service)
└── README.md               # this file
```

The structure mirrors `waveflow`'s `src-tauri/crates/app/src/` so contributors who know one repo can read the other without re-orienting. Sync (`src/sync/`), streaming (`src/stream/`) and auth middleware land as new modules in later sub-phases.

## License

[AGPL-3.0-only](LICENSE). The server-side network clause keeps the ecosystem healthy — anyone running a modified version of this server as a network service has to publish their changes under the same terms.

The desktop app and `waveflow-core` stay [GPL-3.0-only](https://github.com/InstaZDLL/WaveFlow/blob/main/LICENSE) — locally-run software doesn't need the AGPL network clause. GPL-3.0 code can be combined into this AGPL-3.0 work without issue (AGPL is a strict superset for combined works).

Plugins authored against the [plugin SDK](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-002-plugin-sdk.md) can pick any OSI-compatible license they want, since they run inside a WASM sandbox and aren't statically linked into the server binary.

## Contributing

The main repo's [CONTRIBUTING.md](https://github.com/InstaZDLL/WaveFlow/blob/main/CONTRIBUTING.md) and [conventional commits](https://www.conventionalcommits.org/) rules apply here too. Open issues against the desktop repo's [Phase 1 milestone](https://github.com/InstaZDLL/WaveFlow/milestone/1) for cross-cutting design discussions; bug reports and feature requests scoped to the server itself land on this repo's issue tracker.

**Contributions are accepted under a [DCO](CONTRIBUTING.md#developer-certificate-of-origin)** — every commit needs a `Signed-off-by:` trailer (`git commit -s`). See [CONTRIBUTING.md](CONTRIBUTING.md) for the one-paragraph version.
