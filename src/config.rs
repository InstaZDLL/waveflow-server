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
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("database_url", &self.database_url)
            .field("db_max_connections", &self.db_max_connections)
            .field("jwt_jwks_url", &self.jwt_jwks_url)
            .field("jwt_issuer", &self.jwt_issuer)
            .field("jwt_audience", &self.jwt_audience)
            .field("music_root", &self.music_root)
            .field(
                "stream_secret",
                &self.stream_secret.as_ref().map(|_| "<redacted>"),
            )
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
        // we bail at boot instead.
        let music_root = std::env::var("WAVEFLOW_MUSIC_ROOT")
            .ok()
            .map(std::path::PathBuf::from);
        let stream_secret = std::env::var("WAVEFLOW_STREAM_SECRET")
            .ok()
            .map(|s| s.into_bytes());
        if music_root.is_some() != stream_secret.is_some() {
            anyhow::bail!(
                "streaming requires both WAVEFLOW_MUSIC_ROOT and \
                 WAVEFLOW_STREAM_SECRET to be set together (or both \
                 unset to disable)"
            );
        }

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
        })
    }
}
