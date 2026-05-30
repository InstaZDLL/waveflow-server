//! Shared integration-test harness: spawn the real axum app on a
//! kernel-assigned port against a caller-provided Postgres pool, and
//! hand back the URL the test should hit.
//!
//! Each test that uses this gets its pool from `#[sqlx::test(...)]`,
//! which creates a fresh per-test database, runs migrations, and
//! drops the database when the test exits — no fixtures to clean up
//! manually.

use std::net::SocketAddr;

use sqlx::PgPool;
use waveflow_server::{app, AppState, Config};

/// Boot the app with an arbitrary `dev_auth_enabled` value. The two
/// convenience wrappers below are the only spellings tests should
/// use; the boolean stays here so a future "exhaustively test both
/// branches" style ever gets a single point to extend.
async fn spawn_app_with_dev_auth(pool: PgPool, dev_auth_enabled: bool) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = Config {
        bind_addr: addr,
        request_timeout_secs: 5,
        // `database_url` is unused by `app()` once the pool is built —
        // it lives in the config purely so `Config::from_env` is the
        // single source of truth for env reading. Placeholder string
        // is fine inside the test.
        database_url: "<test>".into(),
        db_max_connections: 1,
        dev_auth_enabled,
    };

    let state = AppState { db: pool };
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(config, state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    format!("http://{addr}")
}

/// Spawn the app with the dev `X-User-Id` shim **enabled** — what
/// most integration tests want. Use [`spawn_app_prod_gate`] for the
/// few that exercise the 503 path.
pub async fn spawn_app(pool: PgPool) -> String {
    spawn_app_with_dev_auth(pool, true).await
}

/// Spawn the app with the production-default config — dev auth
/// **disabled** so every `/api/v1/*` request short-circuits to 503.
/// Used by `dev_auth_gate_returns_503_when_disabled`.
#[allow(dead_code)] // some test files don't use this helper
pub async fn spawn_app_prod_gate(pool: PgPool) -> String {
    spawn_app_with_dev_auth(pool, false).await
}
