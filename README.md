# WaveFlow Server

WaveFlow Server v2 is a self-hosted music server built in Rust. SQLite owns the catalogue and user data; FFmpeg streaming, OpenSubsonic compatibility, WaveFlow Desktop sync and the embedded web player land through independently verified milestones.

> **Status:** M0 through M6 pass their release gates. The OpenSubsonic facade has completed its real-client matrix with Feishin, DSub, Symfonium and Juliet; native Desktop integration, conservative reconciliation and the bilingual studio-nocturne web client are validated end to end. No release tag is created without an explicit operator request.

The accepted architecture is documented in [RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md). The v1 PostgreSQL/JWKS implementation has been removed; it remains available in git history.

Integration documentation:

- [Native API v2 guide](docs/api-v2-guide.md): authentication, PKCE, catalogue,
  media tickets, mutations, synchronization, errors and administration;
- [Subsonic/OpenSubsonic guide](docs/subsonic-api-guide.md): client setup,
  authentication, examples, supported methods and advertised extensions;
- [Interactive API reference](http://127.0.0.1:4533/reference) and
  [OpenAPI JSON](http://127.0.0.1:4533/openapi.json) on a running local server.

## Current quick start

Requirements: Rust 1.94 or newer plus `ffmpeg` and `ffprobe` on `PATH`. No external database or authentication service is required.

```powershell
Copy-Item .env.example .env
$env:WAVEFLOW_ACCOUNT_PASSWORD = "replace-with-at-least-12-characters"
cargo run -- account create-admin --username admin

cargo run -- library add --owner admin --name "Music" --path "D:\Music"

# Optional shared-library membership management.
cargo run -- library set-member --actor admin --library-id "LIBRARY_UUID" --username listener --role listener
cargo run -- library remove-member --actor admin --library-id "LIBRARY_UUID" --username listener

$env:WAVEFLOW_SUBSONIC_PASSWORD = "a-different-app-password"
cargo run -- credential set --actor admin --username admin

# Optional long-lived bearer token for a native/API client.
cargo run -- token create --actor admin --username admin --name "Desktop"

cargo run
```

The credential command prints a generated Subsonic API key exactly once. Back up `data/waveflow.db` and `data/instance.key` together; encrypted credentials cannot be recovered with only one of them. The database stores a non-secret key fingerprint so startup and restore reject mismatched pairs before serving or replacing data.

The server listens on `127.0.0.1:4533` by default and exposes:

- `GET /health`: process liveness;
- `GET /ready`: SQLite readiness, independent of scan progress;
- `GET /openapi.json` and `GET /reference`: API contract;
- `POST /api/v2/auth/login`, `/refresh`, `/logout`: rotating native sessions;
- `/api/v2/web/auth/*`: memory-only browser access token plus HttpOnly rotating
  refresh cookie, origin validation and CSRF protection;
- `/api/v2/sync/snapshot`, `/changes`, `/ack`, `/socket`: idempotent user-data
  synchronization defined by `docs/rfcs/RFC-003-waveflow-sync-v2.md`;
- `/api/v2/admin/users`, `/libraries`, `/transcode/status`: native server
  administration and dedicated Subsonic credential rotation;
- `PUT|DELETE /api/v2/admin/users/{username}/subsonic-credential`: rotate or
  revoke the dedicated Subsonic password and API key;
- `POST /api/v2/libraries/{id}/scans`: manual scan trigger;
- `GET /api/v2/scans/{id}` and `/events`: status and SSE progress;
- `GET /api/v2/artwork/{artwork_id}`: authenticated artwork for native and web
  clients;
- `GET /api/v2/tracks/{id}/lyrics`: embedded or sidecar plain/synchronized
  lyrics;
- `GET /api/v2/libraries/{id}/tracks?q=...&offset=...&limit=...`: tenant-scoped catalogue/FTS browsing, paged up to 500 tracks per request.
- `GET /api/v2/tracks/{id}/stream?format=raw|mp3|opus&bitrate=...&offset_ms=...`: authorized playback. Byte ranges apply to originals and completed cache entries; live transcodes use temporal seek and chunked transfer.
- `/rest/<method>` and `/rest/<method>.view`: Subsonic/OpenSubsonic XML or `f=json`, via GET or form POST.
- `/share/{token}`: public metadata plus token-scoped stream URLs for an unexpired share.

For browser-hosted clients such as Feishin, list every trusted origin explicitly, for example `WAVEFLOW_ALLOWED_ORIGINS=http://127.0.0.1:9180,https://music.example.com`. Wildcards are rejected so credential-bearing Subsonic requests cannot be opened to arbitrary sites.

Set `WAVEFLOW_PUBLIC_URL=https://music.example.com` behind the reverse proxy so `createShare` returns an absolute, externally usable URL at creation. An authenticated idempotent retry of that creation returns the same URL, but later share reads and sync snapshots omit it because only its hash is persisted. When the setting is absent, the creation response uses a URL relative to the server origin.

Create or restore a coherent database/key bundle:

```powershell
cargo run -- database backup --output D:\Backups\waveflow-2026-08-02
cargo run -- database restore --input D:\Backups\waveflow-2026-08-02
```

The restore command runs before SQLite is opened and moves the previous database/key into a timestamped recovery directory.

The repository ships a multi-stage `Dockerfile` with FFmpeg and a Compose file. Set `WAVEFLOW_MUSIC_PATH` to the host music directory; it is mounted read-only.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo test --all-features
```

Tests use temporary SQLite databases and need no service container.

The web client lives in `webapp/` and is compiled into the binary, so it must be built first:

```bash
bun --cwd=webapp install
bun run build          # webapp then cargo, in that order
bun --cwd=webapp x playwright install chromium
bun --cwd=webapp run test:e2e
```

`cargo build` alone still works: a placeholder page is embedded when no client build is present.

The embedded client provides the complete functional player and administration
surface with authenticated artwork, persistent queue, Media Session,
preloading, keyboard controls, 14 localized themes, English/French UI and
responsive desktop/mobile navigation. Vitest plus Playwright/axe cover the web
release gate.

## Data and security posture

- SQLite runs with WAL, foreign keys, `busy_timeout` and one process-wide write coordinator.
- Public/domain identifiers are UUIDs; timestamps are Unix epoch milliseconds.
- Web passwords use Argon2id. Access, refresh and API tokens are stored only as SHA-256 hashes.
- The dedicated Subsonic password is encrypted with ChaCha20-Poly1305 under the local 32-byte instance key.
- Library access is represented by explicit owner/manager/listener membership.
- Request traces record sanitized paths, never query strings, authorization headers or public-share bearer tokens.
- Every media lookup verifies membership before resolving the canonical path or cache key; parent components and symlinks are rejected.

## Repository references

- `E:\Workspace\WaveFlow`: desktop client and `waveflow-core` source reference;
- `E:\Workspace\navidrome`: Subsonic and self-hosting behaviour reference;
- `E:\Workspace\waveflow-server-replit-example`: information-architecture reference only.

WaveFlow Server is licensed under [AGPL-3.0-only](LICENSE). Commits require DCO sign-off and Conventional Commit messages.
