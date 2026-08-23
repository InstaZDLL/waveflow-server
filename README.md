<h1 align="center">WaveFlow Server</h1>

<p align="center">
  <strong>Self-hosted music server — one Rust binary, SQLite, and an OpenSubsonic API your clients already speak</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/static/v1?label=version&message=2.0.0-beta.0&color=emerald&style=flat-square" alt="Version" />
  <img src="https://img.shields.io/badge/rust-1.94%2B-orange?style=flat-square&logo=rust" alt="Rust 1.94+" />
  <img src="https://img.shields.io/badge/database-SQLite%20only-003b57?style=flat-square&logo=sqlite&logoColor=white" alt="SQLite only" />
  <img src="https://img.shields.io/badge/API-OpenSubsonic-6a4bd6?style=flat-square" alt="OpenSubsonic" />
  <img src="https://img.shields.io/badge/docker-ghcr.io-2496ed?style=flat-square&logo=docker&logoColor=white" alt="Docker image on GHCR" />
  <img src="https://img.shields.io/badge/license-AGPL--3.0-green?style=flat-square" alt="License AGPL-3.0" />
</p>

---

WaveFlow Server streams the music you already own to the clients you already use. It scans your folders, owns the catalogue in a single SQLite file, transcodes on demand with FFmpeg, and answers on three surfaces at once: the **OpenSubsonic API** that dozens of existing players speak, a **native API** for WaveFlow Desktop, and an **embedded web player** compiled into the binary.

No PostgreSQL. No Redis. No identity provider. No container orchestration. One binary, one database file, one key file.

> **Status — `2.0.0-beta.0`.** All six milestones pass their release gates. The OpenSubsonic façade has been replayed against four real clients on real devices — Symfonium, Feishin, DSub and Juliet — with every result read back from server state rather than from the client's display. See [the compatibility matrix](docs/subsonic-compatibility.md) for what each client actually exercises, and what it does not.

## Why another one

**Your identifiers stop moving.** Album and artist IDs are *derived* from the tags that name them (UUID v8 over a configurable spec, the same grammar Navidrome uses). Rebuild your database from scratch and the same files answer with the same IDs — so cached artwork, starred albums and deep links survive a reinstall. Track IDs are drawn at random on purpose: six tables cascade off them, and a scan matches a file by path and then by content hash, which is a better identity than any tag.

**The Subsonic surface is the reference's, not a variant.** Where WaveFlow and Navidrome disagreed on the artist model, we withdrew — thirteen credit roles, `contributors[]`, `displayComposer`, `roles[]`, an album that hangs off *every* artist it is credited to, and separator rules that split `Rue Delacour / Ivy Trench` in two while leaving `AC/DC` alone.

**Multi-user is a query, not a filter.** Tenancy is enforced inside the SQL through library membership, never in a handler. A resource that is missing and one that belongs to somebody else answer identically — a 404 never confirms existence to someone not entitled to it.

**Your files are never written.** The scanner and every tag operation are strictly read-only.

## Features

| Area | Highlights |
| --- | --- |
| **Catalogue** | Authoritative scanner with content hashing and relocation detection, FTS5 full-text search that folds case and diacritics, deduplicated artwork, embedded and sidecar lyrics, extended tags (ISRC, BPM, moods, ReplayGain, explicit status) |
| **Credits** | Thirteen roles from the reference model — artist, album artist, composer, lyricist, conductor, arranger, producer, director, engineer, mixer, remixer, DJ mixer, performer with its instrument — one person can hold several on one track |
| **Streaming** | Original byte-range playback, on-demand FFmpeg transcode to MP3 or Opus with a disk cache, per-user and global concurrency limits, temporal seek into a live transcode |
| **OpenSubsonic** | The full browse, search, playlist, favourite, rating, scrobble, bookmark, play-queue and share surface, in XML or JSON, over GET or form POST, with the extensions it advertises |
| **Native API** | `/api/v2` with rotating sessions, Authorization Code + PKCE for desktop clients, user-data synchronization over REST and WebSocket ([RFC-003](docs/rfcs/RFC-003-waveflow-sync-v2.md)), SSE scan progress, OpenAPI document and an interactive reference |
| **Web player** | React 19 client compiled into the binary — complete player and administration surface, authenticated artwork, Media Session, preloading, keyboard controls, 14 localized themes, English and French, responsive |
| **Security** | Argon2id passwords, tokens stored only as SHA-256 hashes, the dedicated Subsonic password encrypted with ChaCha20-Poly1305 under a local instance key, stream tickets so `<audio src>` needs no header, origin validation and CSRF protection for browsers |
| **Operations** | Single-file SQLite in WAL with one process-wide writer, immutable checksummed migrations, coherent backup and restore of the database/key pair, `/health` and `/ready`, JSON logging that never records a query string or a token |

## Quick start

### Docker

```bash
docker run -d --name waveflow \
  -p 4533:4533 \
  -v waveflow-data:/data \
  -v /path/to/music:/music:ro \
  ghcr.io/instazdll/waveflow-server:2.0.0-beta.0
```

Or with Compose — set `WAVEFLOW_MUSIC_PATH` to your music directory, which is mounted read-only:

```bash
WAVEFLOW_MUSIC_PATH=/path/to/music docker compose up -d
```

Then create the admin account and register a library:

```bash
docker exec -e WAVEFLOW_ACCOUNT_PASSWORD='at-least-twelve-characters' \
  waveflow waveflow-server account create-admin --username admin
docker exec waveflow waveflow-server library add --owner admin --name Music --path /music
```

### From source

Requirements: **Rust 1.94+**, plus `ffmpeg` and `ffprobe` on `PATH`. Nothing else.

```bash
bun --cwd=webapp install
bun run build              # webapp first, then cargo — the order matters

export WAVEFLOW_ACCOUNT_PASSWORD='at-least-twelve-characters'
cargo run -- account create-admin --username admin
cargo run -- library add --owner admin --name Music --path /path/to/music
cargo run
```

`cargo build` works without a client build too: a placeholder page is embedded when `webapp/dist` is absent.

### Connect a client

```bash
export WAVEFLOW_SUBSONIC_PASSWORD='a-different-app-password'
cargo run -- credential set --actor admin --username admin
```

That prints an API key **once**. Point any Subsonic client at `http://your-host:4533` with the username and that password. For browser-hosted clients such as Feishin, list the trusted origins explicitly — wildcards are rejected, so credential-bearing requests can never be opened to arbitrary sites:

```bash
WAVEFLOW_ALLOWED_ORIGINS=http://127.0.0.1:9180,https://music.example.com
```

Behind a reverse proxy, set `WAVEFLOW_PUBLIC_URL=https://music.example.com` so a created share returns an absolute, externally usable URL.

## Back up two files, together

```bash
cargo run -- database backup  --output /backups/waveflow-2026-08-23
cargo run -- database restore --input  /backups/waveflow-2026-08-23
```

`data/waveflow.db` and `data/instance.key` are one unit: the encrypted Subsonic credentials cannot be recovered with one without the other. The database stores a non-secret fingerprint of the key, so a mismatched pair is rejected at startup rather than after it has replaced your data. Restore runs before SQLite is opened and moves the previous pair into a timestamped recovery directory.

## Tech stack

| Layer | Technologies |
| --- | --- |
| **HTTP** | axum 0.8 (with WebSocket), tower-http, utoipa 5 for the OpenAPI document |
| **Storage** | SQLite through sqlx 0.9 — WAL, foreign keys, `busy_timeout`, FTS5, one process-wide write coordinator |
| **Media** | FFmpeg and ffprobe as external processes, lofty 0.25 for tags and embedded art, BLAKE3 for content hashing |
| **Identity** | UUID v4 for tracks, UUID v8 derived from tags for albums and artists, MD5 as the deduplication digest of the identity spec |
| **Security** | Argon2id, SHA-256 token digests, ChaCha20-Poly1305 for reversible credentials, AEAD-sealed stream tickets |
| **Web client** | React 19, TypeScript, Vite 8, compiled into the binary by rust-embed 8 |
| **Runtime** | tokio, `tracing` with optional JSON output |

## Documentation

- **Integration** — [Native API v2 guide](docs/api-v2-guide.md) · [Subsonic/OpenSubsonic guide](docs/subsonic-api-guide.md) · [compatibility matrix](docs/subsonic-compatibility.md) · [gap analysis](docs/opensubsonic-gap-analysis.md)
- **Architecture** — [RFC-002, the accepted design](docs/rfcs/RFC-002-waveflow-server-v2.md) · [RFC-003, synchronization](docs/rfcs/RFC-003-waveflow-sync-v2.md) · [RFC-004, local/server reconciliation](docs/rfcs/RFC-004-local-server-reconciliation.md)
- **Live on a running server** — [`/reference`](http://127.0.0.1:4533/reference) for the interactive API, [`/openapi.json`](http://127.0.0.1:4533/openapi.json) for the contract
- **Contributing** — [CONTRIBUTING.md](CONTRIBUTING.md); commits need DCO sign-off (`git commit -s`) and Conventional Commit messages

The v1 PostgreSQL/JWKS implementation was removed once the native API landed. It remains in git history; any reference to `/api/v1` in older documents is stale.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features          # hermetic — temporary SQLite databases, no service container

bun --cwd=webapp x playwright install chromium
bun --cwd=webapp run test:e2e
```

FFmpeg and ffprobe must be on `PATH`: the suite boots a real media service.

## Community

- 🐛 **Bug?** → [Bug report](https://github.com/InstaZDLL/waveflow-server/issues/new?template=bug_report.yml)
- ✨ **Feature idea?** → [Feature request](https://github.com/InstaZDLL/waveflow-server/issues/new?template=feature_request.yml)
- 🔒 **Security?** → [Private disclosure](.github/SECURITY.md) — never post a vulnerability publicly.

English and French both welcome.

## Related

- [**WaveFlow**](https://github.com/InstaZDLL/WaveFlow) — the desktop player, a separate project that consumes this API for multi-device sync.

## License

WaveFlow Server is licensed under [**AGPL-3.0-only**](LICENSE): it hosts a network service, so anyone you serve it to is entitled to its source. The desktop application and `waveflow-core` are GPL-3.0-only.
