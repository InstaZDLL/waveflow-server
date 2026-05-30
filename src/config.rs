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

    /// `WAVEFLOW_DEV_AUTH=1` — opt in to the Phase 1.b `X-User-Id`
    /// header auth shim on `/api/v1/profiles/*`. Default: `false`.
    ///
    /// The shim is intentionally trivial to forge (any caller can
    /// send any `i64`), so it must NEVER be on in production. Phase
    /// 1.d.1-PR3 widens the auth surface to accept either the shim
    /// OR a JWT-verified `Authorization: Bearer` header, and the
    /// production gate is now "both auth paths off" — see
    /// [`Self::auth_disabled_at_boot`]. Keeping the shim behind an
    /// opt-in env var means a stray container on a public LAN can't
    /// accidentally expose tenant data to anyone who guesses an
    /// integer id.
    pub dev_auth_enabled: bool,

    /// `WAVEFLOW_JWT_JWKS_URL` — URL of the upstream JWKS document.
    /// When set together with `jwt_issuer` and `jwt_audience`, the
    /// boot path constructs a [`crate::auth::JwtVerifier`] and the
    /// middleware accepts `Authorization: Bearer …` tokens.
    /// `None` leaves the JWT path off.
    pub jwt_jwks_url: Option<String>,

    /// `WAVEFLOW_JWT_ISSUER` — expected `iss` claim on verified
    /// tokens. Paired with [`Self::jwt_jwks_url`]; both must be
    /// `Some` for the JWT auth path to activate.
    pub jwt_issuer: Option<String>,

    /// `WAVEFLOW_JWT_AUDIENCE` — expected `aud` claim on verified
    /// tokens. Paired with [`Self::jwt_jwks_url`]; both must be
    /// `Some` for the JWT auth path to activate.
    pub jwt_audience: Option<String>,
}

impl Config {
    /// True when neither auth path is configured — every `/api/v1/*`
    /// request must short-circuit to 503. The production-default
    /// state on a fresh binary: the operator hasn't yet pointed at
    /// a JWKS, hasn't yet enabled the dev shim, and we'd rather
    /// fail closed than ship an open server.
    pub fn auth_disabled_at_boot(&self) -> bool {
        !self.dev_auth_enabled && !self.has_jwt_config()
    }

    /// True when every JWT env var that the verifier needs is set.
    /// Boot uses this to decide whether to build a verifier; the
    /// middleware uses [`crate::AppState::jwt_verifier`] which is
    /// `Some` exactly when this holds at boot.
    pub fn has_jwt_config(&self) -> bool {
        self.jwt_jwks_url.is_some() && self.jwt_issuer.is_some() && self.jwt_audience.is_some()
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

        // Strict equality on "1" — `true`, `yes`, `on` etc. don't
        // count. Footgun-resistant: the only way to enable the
        // forgeable-header shim is to send the exact string the
        // README documents.
        let dev_auth_enabled = std::env::var("WAVEFLOW_DEV_AUTH").as_deref() == Ok("1");

        // JWT auth knobs. All three must land together — a partial
        // config would build a verifier with wrong / missing
        // claims-validation parameters, which fails closed but
        // confusingly (every token rejected with InvalidClaims).
        // Boot fails fast instead.
        let jwt_jwks_url = std::env::var("WAVEFLOW_JWT_JWKS_URL").ok();
        let jwt_issuer = std::env::var("WAVEFLOW_JWT_ISSUER").ok();
        let jwt_audience = std::env::var("WAVEFLOW_JWT_AUDIENCE").ok();
        let jwt_partial = [
            jwt_jwks_url.is_some(),
            jwt_issuer.is_some(),
            jwt_audience.is_some(),
        ];
        let some_count = jwt_partial.iter().filter(|x| **x).count();
        if some_count != 0 && some_count != 3 {
            anyhow::bail!(
                "JWT auth requires WAVEFLOW_JWT_JWKS_URL, WAVEFLOW_JWT_ISSUER and \
                 WAVEFLOW_JWT_AUDIENCE to all be set, or all unset. Currently {} of 3 are set.",
                some_count
            );
        }

        Ok(Self {
            bind_addr,
            request_timeout_secs,
            database_url,
            db_max_connections,
            dev_auth_enabled,
            jwt_jwks_url,
            jwt_issuer,
            jwt_audience,
        })
    }
}
