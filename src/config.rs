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

        Ok(Self {
            bind_addr,
            request_timeout_secs,
        })
    }
}
