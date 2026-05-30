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

/// Spawn the app in the background and return its base URL.
pub async fn spawn_app(pool: PgPool) -> String {
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
