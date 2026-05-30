//! waveflow-server binary entrypoint.
//!
//! Wires the runtime: load `.env`, install tracing, build the axum
//! router from [`waveflow_server::app`], bind, and serve until SIGINT.
//! The router itself lives behind a library entry point so integration
//! tests can spawn the same app in-process without re-creating the
//! plumbing.

use std::net::SocketAddr;

use tokio::signal;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use std::sync::Arc;

use waveflow_server::{
    app,
    auth::{JwtVerifier, JwtVerifierConfig},
    config::Config,
    db, AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `.env` is dev-only and best-effort. In release deploys env vars
    // come from the systemd unit / container; the dotenv miss is the
    // expected path and we don't log it.
    let _ = dotenvy::dotenv();

    init_tracing();

    let config = Config::from_env()?;

    // Connect to Postgres and apply pending migrations before opening
    // the listener — surfacing schema mismatches at boot is what makes
    // the readiness probe trustworthy. A failure here aborts startup
    // with a non-zero exit code so the orchestrator backs off.
    let db = db::connect(&config).await?;
    db::run_migrations(&db).await?;
    info!("postgres pool ready, migrations applied");

    // Build the JWT verifier eagerly so a bad JWKS URL fails boot
    // (the operator sees the error immediately, not on the first
    // request). The verifier itself doesn't fetch the JWKS until a
    // token actually needs verifying.
    let jwt_verifier = if config.has_jwt_config() {
        let verifier = JwtVerifier::new(JwtVerifierConfig {
            jwks_url: config
                .jwt_jwks_url
                .clone()
                .expect("has_jwt_config checked above"),
            issuer: config
                .jwt_issuer
                .clone()
                .expect("has_jwt_config checked above"),
            audience: config
                .jwt_audience
                .clone()
                .expect("has_jwt_config checked above"),
        })
        .map_err(|err| anyhow::anyhow!("JWT verifier init failed: {err}"))?;
        info!("JWT auth path enabled");
        Some(Arc::new(verifier))
    } else {
        None
    };

    if config.auth_disabled_at_boot() {
        // The server still boots — `/health` and `/ready` stay up
        // so a deploy in this state can be probed — but every
        // `/api/v1/*` request will short-circuit to 503. Warn loudly
        // so an operator who flipped a wrong env var sees it without
        // having to read the body of a request.
        tracing::warn!(
            "no auth configured: every /api/v1/* request will return 503. \
             Set WAVEFLOW_DEV_AUTH=1 (dev only) or the WAVEFLOW_JWT_* triple."
        );
    }

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    let local = listener.local_addr()?;
    info!(addr = %local, "waveflow-server listening");

    let state = AppState {
        db,
        jwt_verifier,
        dev_auth_enabled: config.dev_auth_enabled,
    };
    axum::serve(
        listener,
        app(config, state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("waveflow-server shut down cleanly");
    Ok(())
}

/// Install a `tracing` subscriber. `RUST_LOG` controls verbosity (we
/// fall back to `info` for the binary's own crates). `WAVEFLOW_LOG_FORMAT=json`
/// switches to JSON for log aggregators; anything else stays on the
/// pretty terminal formatter used in dev.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,waveflow_server=debug,tower_http=debug"));

    let json_mode = std::env::var("WAVEFLOW_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let registry = tracing_subscriber::registry().with(filter);
    if json_mode {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer().compact()).init();
    }
}

/// Wait for SIGINT (Ctrl+C) or SIGTERM (systemd / container stop).
/// On Windows we only listen for Ctrl+C — SIGTERM doesn't exist there.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("SIGINT received, shutting down"),
        _ = terminate => info!("SIGTERM received, shutting down"),
    }
}
