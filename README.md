# waveflow-server

Self-hosted backend for [WaveFlow](https://github.com/InstaZDLL/WaveFlow). Powers multi-device library sync, browser playback, public shareable playlists, and (later) the mobile app.

> **Status:** bootstrap. The `1.b` skeleton (axum + Postgres CRUD) is not implemented yet. Track progress against the Phase 1 milestone on the main repo.

## Architecture

The full architectural decisions — server stack, web app stack, auth boundary, sync protocol, streaming, delivery plan — live in [`waveflow/docs/rfcs/RFC-001-waveflow-server.md`](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md). Read that before opening a PR here.

TL;DR:

- **Stack:** axum (Rust) + PostgreSQL + sqlx + utoipa (OpenAPI) + tokio-tungstenite (WebSocket).
- **Reuses `waveflow-core`** from the main `waveflow` repo as a git dependency for the first months, switching to a crates.io release once the public API stabilizes.
- **Auth boundary:** JWT signed by `waveflow-web` (Better Auth), verified here against the JWKS endpoint. Server never touches credentials.
- **Sync:** append-only ops log with a server-assigned monotonic sequence + tombstones. WebSocket fan-out via Postgres `LISTEN / NOTIFY`.

## Local development

> Pre-requirement: Rust stable (1.84+) and a Postgres ≥ 15 instance reachable on `DATABASE_URL`.

```bash
git clone https://github.com/InstaZDLL/waveflow-server
cd waveflow-server
cargo run
```

Once the `1.b` skeleton lands the binary will boot a `/health` endpoint on `:3000`. Until then `cargo run` only prints the placeholder banner.

## Repository layout

```text
.
├── Cargo.toml              # single binary crate, name = `waveflow-server`
├── src/
│   └── main.rs             # entry point
├── .github/workflows/
│   └── ci.yml              # cargo check / test / clippy / fmt on push + PR
├── CONTRIBUTING.md         # DCO sign-off + commit conventions
├── LICENSE                 # AGPL-3.0 (server hosts a network service)
└── README.md               # this file
```

Once Phase 1.b starts, this grows into `src/{api,db,sync}` modules and a `migrations/` directory. The structure mirrors `waveflow`'s `src-tauri/crates/app/src/` so contributors who know one repo can read the other without re-orienting.

## License

[AGPL-3.0-only](LICENSE). The server-side network clause keeps the ecosystem healthy — anyone running a modified version of this server as a network service has to publish their changes under the same terms.

The desktop app and `waveflow-core` stay [GPL-3.0-only](https://github.com/InstaZDLL/WaveFlow/blob/main/LICENSE) — locally-run software doesn't need the AGPL network clause. GPL-3.0 code can be combined into this AGPL-3.0 work without issue (AGPL is a strict superset for combined works).

Plugins authored against the [plugin SDK](https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-002-plugin-sdk.md) can pick any OSI-compatible license they want, since they run inside a WASM sandbox and aren't statically linked into the server binary.

## Contributing

The main repo's [CONTRIBUTING.md](https://github.com/InstaZDLL/WaveFlow/blob/main/CONTRIBUTING.md) and [conventional commits](https://www.conventionalcommits.org/) rules apply here too. Open issues against the desktop repo's [Phase 1 milestone](https://github.com/InstaZDLL/WaveFlow/milestone/1) for cross-cutting design discussions; bug reports and feature requests scoped to the server itself land on this repo's issue tracker.

**Contributions are accepted under a [DCO](CONTRIBUTING.md#developer-certificate-of-origin)** — every commit needs a `Signed-off-by:` trailer (`git commit -s`). See [CONTRIBUTING.md](CONTRIBUTING.md) for the one-paragraph version.
