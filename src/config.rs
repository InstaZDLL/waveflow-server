//! Server configuration loaded once at boot from environment variables
//! (with a `.env` fallback in dev, see `main.rs`).
//!
//! Each field carries the env var name in its doc comment so a reader
//! browsing this struct sees the full configuration surface in one
//! place. Add new entries here when introducing a tunable; don't read
//! from `std::env` in module code.

use std::net::SocketAddr;

/// Process-wide configuration. Construct once at boot via [`Config::from_env`]
/// and pass into [`crate::app`].
///
/// `Debug` is hand-written so the HMAC `stream_secret` is rendered as
/// `<redacted>` — a derived `Debug` would print the raw bytes any
/// time the config landed in a `tracing` field or a `Config::from_env`
/// failure message.
#[derive(Clone)]
pub struct Config {
    /// `WAVEFLOW_BIND` — `host:port` the server binds to.
    /// Default: `127.0.0.1:3000`.
    ///
    /// Bind to `0.0.0.0:3000` in container / systemd deploys; the
    /// loopback default avoids exposing a dev instance to the LAN by
    /// accident.
    pub bind_addr: SocketAddr,

    /// `WAVEFLOW_REQUEST_TIMEOUT_SECS` — per-request timeout enforced
    /// by the tower-http TimeoutLayer. Default: 30 seconds.
    ///
    /// 30 s is comfortable for the CRUD endpoints planned in 1.b.2.
    /// The streaming endpoint (1.e) will live behind a separate router
    /// without this layer so range requests can run for the full
    /// duration of a track.
    pub request_timeout_secs: u64,

    /// `DATABASE_URL` — Postgres connection string consumed by sqlx
    /// (`postgres://user:pass@host:port/dbname`). Required — there's
    /// no sensible default for a server's main database, and silently
    /// falling back would let a misconfigured deploy boot and then
    /// 5xx every request.
    pub database_url: String,

    /// `WAVEFLOW_DB_MAX_CONNECTIONS` — upper bound on the sqlx pool.
    /// Default: 20. Postgres can easily serve that many active
    /// connections per `pgbouncer`-less deploy; bump if you front the
    /// server behind a pooler that demands a smaller pool here.
    pub db_max_connections: u32,

    /// `WAVEFLOW_JWT_JWKS_URL` — URL of the upstream JWKS document
    /// (e.g. `https://auth.waveflow.app/api/auth/jwks`). Required at
    /// boot — the legacy `X-User-Id` dev shim retired in Phase 1.d.2,
    /// so JWT verification is the only auth path the server offers.
    pub jwt_jwks_url: String,

    /// `WAVEFLOW_JWT_ISSUER` — expected `iss` claim on verified
    /// tokens. Must match Better Auth's `BETTER_AUTH_URL`. Required.
    pub jwt_issuer: String,

    /// `WAVEFLOW_JWT_AUDIENCE` — expected `aud` claim on verified
    /// tokens. Must match `WAVEFLOW_JWT_AUDIENCE` on the auth server
    /// side (defaults there to `"waveflow-server"`). Required.
    pub jwt_audience: String,

    /// `WAVEFLOW_MUSIC_ROOT` — filesystem root the streaming endpoint
    /// resolves `track.file_path` against. Every file the server can
    /// stream lives under this directory; the handler canonicalises
    /// the joined path and refuses anything outside it (path-traversal
    /// guard). `None` disables the streaming endpoints — the mint
    /// route returns 503 instead of issuing tokens that would just
    /// 404 on the stream side.
    pub music_root: Option<std::path::PathBuf>,

    /// `WAVEFLOW_STREAM_SECRET` — HMAC key the mint endpoint signs
    /// stream URLs with. Browsers can't attach a Bearer to
    /// `<audio src>`, so the short-lived signed URL replaces the JWT
    /// for that one route. `None` disables streaming (the mint
    /// endpoint returns 503). 32 random bytes (`openssl rand -base64
    /// 32`) is the recommended size.
    pub stream_secret: Option<Vec<u8>>,

    /// Artwork storage configuration. `Some` when
    /// `WAVEFLOW_ARTWORK_LOCAL_DIR` is set at boot; `None` disables
    /// the artwork endpoints (they answer 503). Same opt-in
    /// philosophy as streaming — a deploy that doesn't want to ship
    /// the feature just leaves the env unset.
    pub artwork: Option<crate::storage::ArtworkConfig>,

    /// Background scanner that periodically repairs partial variant
    /// caches (Phase 1.i.1). `Some` when the artwork backend is set
    /// AND `WAVEFLOW_ARTWORK_SCANNER_DISABLED` is unset/empty;
    /// `None` skips the spawn at boot. Defaults: 5-minute interval,
    /// 50 parents per cycle.
    pub artwork_scanner: Option<crate::artwork_jobs::ArtworkScannerConfig>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("request_timeout_secs", &self.request_timeout_secs)
            // `DATABASE_URL` typically embeds the Postgres credentials
            // (`postgres://user:password@host/db`), so a derived
            // `Debug` would land them in any `tracing` field or
            // anyhow context that prints the config. Redact the
            // whole value rather than try to parse-and-mask the
            // password segment — opaque is the safer default.
            .field("database_url", &"<redacted>")
            .field("db_max_connections", &self.db_max_connections)
            .field("jwt_jwks_url", &self.jwt_jwks_url)
            .field("jwt_issuer", &self.jwt_issuer)
            .field("jwt_audience", &self.jwt_audience)
            .field("music_root", &self.music_root)
            .field(
                "stream_secret",
                &self.stream_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("artwork", &self.artwork)
            .field("artwork_scanner", &self.artwork_scanner)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("WAVEFLOW_BIND")
            .as_deref()
            .unwrap_or("127.0.0.1:3000")
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid WAVEFLOW_BIND: {e}"))?;

        let request_timeout_secs = std::env::var("WAVEFLOW_REQUEST_TIMEOUT_SECS")
            .ok()
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|e| anyhow::anyhow!("invalid WAVEFLOW_REQUEST_TIMEOUT_SECS: {e}"))?
            .unwrap_or(30);

        // Zero would make every request time out before the handler
        // runs. Fail fast at boot rather than silently 408 every call.
        if request_timeout_secs == 0 {
            anyhow::bail!("invalid WAVEFLOW_REQUEST_TIMEOUT_SECS: must be > 0");
        }

        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required"))?;

        let db_max_connections = std::env::var("WAVEFLOW_DB_MAX_CONNECTIONS")
            .ok()
            .map(|s| s.parse::<u32>())
            .transpose()
            .map_err(|e| anyhow::anyhow!("invalid WAVEFLOW_DB_MAX_CONNECTIONS: {e}"))?
            .unwrap_or(20);

        if db_max_connections == 0 {
            anyhow::bail!("invalid WAVEFLOW_DB_MAX_CONNECTIONS: must be > 0");
        }

        // JWT triple — all three are required for the server to
        // boot. The dev `X-User-Id` shim retired in Phase 1.d.2, so
        // there's no longer a "boot without JWT" mode to fall back
        // to. Failing at boot (rather than silently 503-ing every
        // request) tells the operator immediately that the
        // deployment is misconfigured.
        let jwt_jwks_url = std::env::var("WAVEFLOW_JWT_JWKS_URL")
            .map_err(|_| anyhow::anyhow!("WAVEFLOW_JWT_JWKS_URL is required"))?;
        let jwt_issuer = std::env::var("WAVEFLOW_JWT_ISSUER")
            .map_err(|_| anyhow::anyhow!("WAVEFLOW_JWT_ISSUER is required"))?;
        let jwt_audience = std::env::var("WAVEFLOW_JWT_AUDIENCE")
            .map_err(|_| anyhow::anyhow!("WAVEFLOW_JWT_AUDIENCE is required"))?;

        // Streaming knobs — both required together, both optional
        // (unset disables `/api/v1/stream/*` cleanly with 503). A
        // half-set config (one without the other) is a footgun, so
        // we bail at boot instead. `std::env::var` returns `Ok("")`
        // when a variable is exported but empty, which would slip
        // a zero-byte HMAC key past the structural check — treat
        // empties as if the var were unset (and then enforce the
        // mutual-presence + minimum-length rules).
        let music_root = std::env::var("WAVEFLOW_MUSIC_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);
        let stream_secret = std::env::var("WAVEFLOW_STREAM_SECRET")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.into_bytes());
        if music_root.is_some() != stream_secret.is_some() {
            anyhow::bail!(
                "streaming requires both WAVEFLOW_MUSIC_ROOT and \
                 WAVEFLOW_STREAM_SECRET to be set together (non-empty) \
                 or both unset to disable"
            );
        }
        // Sanity-check the HMAC key length. `openssl rand -base64 32`
        // (the doc-recommended generator) emits 44 bytes, so 32 is a
        // comfortably-low floor; rejecting anything shorter keeps a
        // trivially-guessable secret (think "x" exported on a quick
        // local boot) from masquerading as a real key.
        const MIN_STREAM_SECRET_BYTES: usize = 32;
        if let Some(secret) = stream_secret.as_ref() {
            if secret.len() < MIN_STREAM_SECRET_BYTES {
                anyhow::bail!(
                    "WAVEFLOW_STREAM_SECRET is too short ({} bytes); minimum is {} bytes. \
                     Generate with `openssl rand -base64 32`.",
                    secret.len(),
                    MIN_STREAM_SECRET_BYTES,
                );
            }
        }

        // Artwork storage — opt-in (same shape as streaming). The
        // `from_env` helper returns `Ok(None)` when the feature is
        // unconfigured so a fresh deploy doesn't have to set the
        // var until it wants to ship the cache.
        let artwork = crate::storage::ArtworkConfig::from_env()?;

        // Background self-heal scanner. Only resolved when the
        // artwork backend itself is enabled (a scanner with no
        // storage would have nothing to repair) AND the operator
        // hasn't explicitly disabled it. Defaults are tuned for a
        // healthy server — see the constants in `artwork_jobs`.
        let artwork_scanner = if artwork.is_some() {
            let disabled = std::env::var("WAVEFLOW_ARTWORK_SCANNER_DISABLED")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some();
            if disabled {
                None
            } else {
                let interval_secs = std::env::var("WAVEFLOW_ARTWORK_SCANNER_INTERVAL_SECS")
                    .ok()
                    .map(|s| s.parse::<u64>())
                    .transpose()
                    .map_err(|e| {
                        anyhow::anyhow!("invalid WAVEFLOW_ARTWORK_SCANNER_INTERVAL_SECS: {e}")
                    })?
                    .unwrap_or_else(|| crate::artwork_jobs::DEFAULT_SCAN_INTERVAL.as_secs());
                // Floor at 1 s — zero would busy-loop the worker.
                let interval = std::time::Duration::from_secs(interval_secs.max(1));
                let batch_size = std::env::var("WAVEFLOW_ARTWORK_SCANNER_BATCH_SIZE")
                    .ok()
                    .map(|s| s.parse::<usize>())
                    .transpose()
                    .map_err(|e| {
                        anyhow::anyhow!("invalid WAVEFLOW_ARTWORK_SCANNER_BATCH_SIZE: {e}")
                    })?
                    .unwrap_or(crate::artwork_jobs::DEFAULT_BATCH_SIZE)
                    .max(1)
                    // The scanner ultimately passes this through to SQL
                    // `LIMIT $`, which sqlx binds as `i64`. On a 64-bit
                    // target `usize::MAX > i64::MAX`, so a hostile env
                    // value above `i64::MAX` would wrap to a negative
                    // bind and either error inside Postgres or skew
                    // the query semantics. Clamp here at boot so the
                    // hot path stays a plain `as i64`.
                    .min(i64::MAX as usize);
                Some(crate::artwork_jobs::ArtworkScannerConfig {
                    interval,
                    batch_size,
                })
            }
        } else {
            None
        };

        Ok(Self {
            bind_addr,
            request_timeout_secs,
            database_url,
            db_max_connections,
            jwt_jwks_url,
            jwt_issuer,
            jwt_audience,
            music_root,
            stream_secret,
            artwork,
            artwork_scanner,
        })
    }
}
