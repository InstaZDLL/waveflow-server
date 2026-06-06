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
    db,
    storage::ArtworkStorage,
    sync::{SyncHub, DEFAULT_COMPACTION_INTERVAL, DEFAULT_FLUSH_INTERVAL},
    AppState, StreamCtx,
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
    let verifier = JwtVerifier::new(JwtVerifierConfig {
        jwks_url: config.jwt_jwks_url.clone(),
        issuer: config.jwt_issuer.clone(),
        audience: config.jwt_audience.clone(),
    })
    .map_err(|err| anyhow::anyhow!("JWT verifier init failed: {err}"))?;
    let jwt_verifier = Arc::new(verifier);
    info!(jwks_url = %config.jwt_jwks_url, "JWT auth path enabled");

    // Streaming context — Some when both knobs are set, None disables
    // both `/api/v1/stream/*` routes. Canonicalise the music root at
    // boot so the per-request handler doesn't redo it; a missing
    // directory aborts startup loudly.
    let stream_ctx = match (config.music_root.clone(), config.stream_secret.clone()) {
        (Some(root), Some(secret)) => {
            let canonical = tokio::fs::canonicalize(&root).await.map_err(|err| {
                anyhow::anyhow!("WAVEFLOW_MUSIC_ROOT {root:?} is not accessible: {err}")
            })?;
            info!(music_root = %canonical.display(), "streaming enabled");
            Some(Arc::new(StreamCtx {
                music_root: canonical,
                secret,
            }))
        }
        _ => {
            info!("streaming disabled (WAVEFLOW_MUSIC_ROOT + WAVEFLOW_STREAM_SECRET unset)");
            None
        }
    };

    // Artwork storage — Some when the artwork backend is configured
    // (`WAVEFLOW_ARTWORK_LOCAL_DIR` for the LocalFileSystem path or
    // `WAVEFLOW_ARTWORK_S3_BUCKET` + creds for the S3 family).
    // None disables both `/api/v1/artwork/*` routes. The local
    // backend creates the root directory on the fly so a fresh
    // container only needs the env set, not a pre-existing dir; the
    // S3 backend defers credential validation to first use, so a
    // bad key / bucket surfaces as 500 on the first call rather
    // than blocking boot.
    let artwork_storage = match config.artwork.as_ref() {
        Some(backend) => {
            let storage = ArtworkStorage::from_backend(backend)?;
            info!(backend = ?backend, "artwork storage enabled");
            Some(storage)
        }
        None => {
            info!(
                "artwork storage disabled (set WAVEFLOW_ARTWORK_LOCAL_DIR or \
                 WAVEFLOW_ARTWORK_S3_BUCKET to enable)"
            );
            None
        }
    };

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    let local = listener.local_addr()?;
    info!(addr = %local, "waveflow-server listening");

    // Sync hub — broadcast channel + ACK debouncer + nightly
    // compaction. The two `JoinHandle`s are intentionally retained as
    // `_flush_task` / `_compaction_task` so the underscored bindings
    // keep ownership through `main`'s scope — dropping a handle
    // doesn't cancel the task itself, but rebinding-by-shadowing or
    // explicit `drop` would, and a future refactor that loses the
    // bindings should hear from the borrow checker on the next read.
    let (sync_hub, _flush_task, _compaction_task) = SyncHub::spawn(
        db.clone(),
        DEFAULT_FLUSH_INTERVAL,
        DEFAULT_COMPACTION_INTERVAL,
    );
    info!(
        flush_interval = ?DEFAULT_FLUSH_INTERVAL,
        compaction_interval = ?DEFAULT_COMPACTION_INTERVAL,
        "sync hub started",
    );

    // Background artwork scanner — spawned when both the storage
    // backend AND the scanner are configured at boot. Held in
    // `_artwork_scanner_task` for the binary's lifetime; dropping
    // the handle wouldn't cancel the task, but a future refactor
    // that loses the binding should fail the borrow checker on the
    // next read.
    let _artwork_scanner_task = match (artwork_storage.as_ref(), config.artwork_scanner.as_ref()) {
        (Some(storage), Some(scanner_cfg)) => {
            info!(
                interval_secs = scanner_cfg.interval.as_secs(),
                batch_size = scanner_cfg.batch_size,
                "artwork background scanner started",
            );
            Some(waveflow_server::artwork_jobs::spawn(
                db.clone(),
                storage.clone(),
                scanner_cfg.clone(),
            ))
        }
        _ => {
            if artwork_storage.is_some() {
                info!("artwork background scanner disabled by WAVEFLOW_ARTWORK_SCANNER_DISABLED");
            }
            None
        }
    };

    let state = AppState {
        db,
        jwt_verifier,
        stream_ctx,
        sync: sync_hub,
        artwork: artwork_storage,
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
