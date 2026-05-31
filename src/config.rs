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
#[derive(Debug, Clone)]
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

        Ok(Self {
            bind_addr,
            request_timeout_secs,
            database_url,
            db_max_connections,
            jwt_jwks_url,
            jwt_issuer,
            jwt_audience,
        })
    }
}
