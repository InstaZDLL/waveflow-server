# waveflow-server

Self-hosted backend + web client for [WaveFlow](https://github.com/InstaZDLL/WaveFlow). Powers multi-device library sync, browser playback, public shareable playlists, and (later) the mobile app.

This repository is a monorepo:

| Path  | Purpose                                                                                                  |
| ----- | -------------------------------------------------------------------------------------------------------- |
| `/`   | `waveflow-server` — axum (Rust) + PostgreSQL service exposing the auth + sync + streaming + share API.   |
| `web/` | `waveflow-web` — React + TanStack Start frontend + Better Auth instance. Hosts the JWKS the server reads. |

The desktop app at [`InstaZDLL/WaveFlow`](https://github.com/InstaZDLL/WaveFlow) is shipped from its own repository on a different release cadence (local-only software, GPL-3.0) and consumes this monorepo's API.

> **Status:** Server is at Phase 4.d (track + album + artist browse), web is at Sprint 4 (player + playlists). Phase milestone tracking lives on the main repo's [Phase 1 milestone](https://github.com/InstaZDLL/WaveFlow/milestone/1).

## Architecture

The architectural decisions — server stack, web stack, auth boundary, sync protocol, streaming, delivery plan — live in [`docs/rfcs/RFC-001-waveflow-server.md`](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md) on the main repo. Read that before opening a substantive PR here.

TL;DR:

- **Server stack:** axum (Rust) + PostgreSQL + sqlx + utoipa (OpenAPI) + tokio-tungstenite (WebSocket).
- **Web stack:** React 19 + TanStack Start (Vite + Nitro) + Better Auth + Tailwind v4 + `@waveflow/design-tokens`.
- **Reuses `waveflow-core`** from the main `waveflow` repo as a git dependency for the first months, switching to a crates.io release once the public API stabilises.
- **Auth boundary:** Better Auth (hosted by `web/`) issues an ES256 JWT. The server verifies it against the JWKS endpoint and lazy-provisions the `users` row on first request. The server never touches credentials.
- **Sync:** append-only ops log with a server-assigned monotonic sequence + tombstones. WebSocket fan-out via tokio `broadcast`.

## Local development

For visual end-to-end QA there's a sibling [`waveflow-dev-stack`](https://github.com/InstaZDLL/waveflow-dev-stack) repo (Postgres + .env templates + step-by-step) so you don't have to hand-stitch the wiring every time.

For day-to-day work on a single half:

### Server (`/`)

```bash
# Postgres ≥ 15 reachable on DATABASE_URL.
cp .env.example .env
cargo run                                       # listens on WAVEFLOW_BIND (default 127.0.0.1:3000)
cargo test --all-features                       # integration suite — needs a real Postgres
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

The boot sequence connects to Postgres, applies pending migrations, and serves:

- `GET /health` — liveness, always `200 {status, version}`.
- `GET /ready` — readiness, `200` once `SELECT 1` round-trips.
- `GET /openapi.json` + `GET /reference` — Scalar-rendered API reference of every handler with a `routes!()` registration.
- `/api/v1/profiles/*` — full CRUD scoped to the calling user via `Authorization: Bearer <jwt>`. Tenant isolation enforced at the storage layer (`PostgresProfileRepository::*_for_user`).
- `/api/v1/profiles/{profile_id}/libraries/*`, `/.../tracks/*`, `/.../playlists/*` — same auth + tenant-scoping pattern.

> 🔒 **Auth: JWT-only.** Every `/api/v1/*` request must carry an `Authorization: Bearer <jwt>` header signed by the configured Better Auth issuer (`web/`'s instance). Boot requires the full `WAVEFLOW_JWT_JWKS_URL` / `WAVEFLOW_JWT_ISSUER` / `WAVEFLOW_JWT_AUDIENCE` triple.

### Web (`web/`)

```bash
cd web
bun install
cp .env.example .env                             # set BETTER_AUTH_SECRET + DATABASE_URL + WAVEFLOW_SERVER_URL
bun run db:migrate                               # apply Better Auth migrations
bun run dev                                      # Vite dev server on :3000
```

When pairing with a local server, point both at the same Postgres (different databases — `waveflow` vs `waveflow_auth`) and at each other:

```text
web :3000 ──┐
            ├── Better Auth issues JWT ──┐
            │                            ▼
server :4000 ◄── verifies JWT against web's /api/auth/jwks
```

## Repository layout

```text
.
├── Cargo.toml              # single binary crate, name = `waveflow-server`
├── src/                    # server source
│   ├── main.rs             # entrypoint — connect pool, run migrations, serve
│   ├── lib.rs              # router + AppState (PgPool, JwtVerifier, SyncHub, ...)
│   ├── config.rs           # `Config::from_env` — single env-reading entrypoint
│   ├── db.rs / db/         # PgPool wiring + embedded migration runner + per-domain helpers
│   ├── api/                # one file per resource (/health, /ready, sync, share, …)
│   ├── apply.rs            # sync apply pipeline (Phase 1.g.0+)
│   └── storage.rs          # object_store-backed artwork cache
├── migrations/             # Postgres sqlx migrations (immutable once merged)
├── tests/                  # integration tests (real Postgres via sqlx::test)
├── web/                    # TanStack Start app — react routes, server-fns, design tokens
│   ├── src/                # routes, components, server-fns
│   ├── packages/           # @waveflow/design-tokens (workspace)
│   ├── db/migrations/      # Better Auth schema (hand-written, applied by scripts/db-migrate.ts)
│   └── package.json
├── .github/                # CI workflows (rust + web), labeler, dependabot, issue+PR templates
├── CLAUDE.md               # contributor onboarding (this repo)
├── CONTRIBUTING.md         # DCO sign-off + conventional commits
├── LICENSE                 # AGPL-3.0
└── README.md               # this file
```

The `src/` layout intentionally mirrors `waveflow`'s `src-tauri/crates/app/src/` so contributors who know one side can read the other without re-orienting. Sync (`src/sync/`), streaming (`src/stream/`), auth middleware live as siblings of `src/api/`.

## License

[AGPL-3.0-only](LICENSE). The server-side network clause keeps the ecosystem healthy — anyone running a modified version of this server as a network service has to publish their changes under the same terms. The web client lives under the same licence for the same reason.

The desktop app and `waveflow-core` stay [GPL-3.0-only](https://github.com/InstaZDLL/WaveFlow/blob/main/LICENSE) — locally-run software doesn't need the AGPL network clause. GPL-3.0 code can be combined into this AGPL-3.0 work without issue.

Plugins authored against the [plugin SDK](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-002-plugin-sdk.md) can pick any OSI-compatible license they want, since they run inside a WASM sandbox.

## Contributing

The main repo's [CONTRIBUTING.md](https://github.com/InstaZDLL/WaveFlow/blob/main/CONTRIBUTING.md) and [conventional commits](https://www.conventionalcommits.org/) rules apply here. Open issues against the desktop repo's [Phase 1 milestone](https://github.com/InstaZDLL/WaveFlow/milestone/1) for cross-cutting design discussions; bug reports and feature requests scoped to the server or web client land on this repo's issue tracker.

**Contributions are accepted under a [DCO](CONTRIBUTING.md#developer-certificate-of-origin)** — every commit needs a `Signed-off-by:` trailer (`git commit -s`).
