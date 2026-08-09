# WaveFlow Server

WaveFlow Server v2 is a self-hosted music server built in Rust. SQLite owns the catalogue and user data; FFmpeg streaming, OpenSubsonic compatibility, WaveFlow Desktop sync and the embedded web player land through independently verified milestones.

> **Status:** M0 through M3 pass their release gates. The OpenSubsonic facade has completed its real-client matrix with Feishin 1.15.1, Substreamer 8.0.91, DSub 5.5.3 and Symfonium 14.1.0. M4 is implemented; no release tag is created without an explicit operator request.

The accepted architecture is documented in [RFC-002](docs/rfcs/RFC-002-waveflow-server-v2.md). The v1 PostgreSQL/JWKS implementation has been removed; it remains available in git history.

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
- `POST /api/v2/auth/login`, `/refresh`, `/logout`: rotating local sessions.
- `POST /api/v2/libraries/{id}/scans`: manual scan trigger;
- `GET /api/v2/scans/{id}` and `/events`: status and SSE progress;
- `GET /api/v2/libraries/{id}/tracks?q=...&offset=...&limit=...`: tenant-scoped catalogue/FTS browsing, paged up to 500 tracks per request.
- `GET /api/v2/tracks/{id}/stream?format=raw|mp3|opus&bitrate=...&offsetMs=...`: authorized playback. Byte ranges apply to originals and completed cache entries; live transcodes use temporal seek and chunked transfer.
- `/rest/<method>` and `/rest/<method>.view`: Subsonic/OpenSubsonic XML or `f=json`, via GET or form POST.
- `/share/{token}`: public metadata plus token-scoped stream URLs for an unexpired share.

For browser-hosted clients such as Feishin, list every trusted origin explicitly, for example `WAVEFLOW_ALLOWED_ORIGINS=http://127.0.0.1:9180,https://music.example.com`. Wildcards are rejected so credential-bearing Subsonic requests cannot be opened to arbitrary sites.

Set `WAVEFLOW_PUBLIC_URL=https://music.example.com` behind the reverse proxy so `createShare` returns absolute, externally usable URLs. When it is omitted, share URLs remain relative to the server origin.

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
```

`cargo build` alone still works: a placeholder page is embedded when no client build is present.

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
