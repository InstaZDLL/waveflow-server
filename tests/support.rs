//! Shared integration-test harness: spawn the real axum app on a
//! kernel-assigned port against a caller-provided Postgres pool, and
//! hand back the URL the test should hit.
//!
//! Each test that uses this gets its pool from `#[sqlx::test(...)]`,
//! which creates a fresh per-test database, runs migrations, and
//! drops the database when the test exits — no fixtures to clean up
//! manually.

use std::{net::SocketAddr, sync::Arc};

use sqlx::PgPool;
use waveflow_server::{app, auth::JwtVerifier, AppState, Config};

/// Boot knobs for the shared `spawn_app_…` family. Keeps the per-test
/// configuration surface in one place — every new "spawn variant"
/// adds an entry here, the wrappers below stay one-liners.
#[derive(Default)]
pub struct SpawnOptions {
    pub dev_auth_enabled: bool,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
}

/// Boot the app with an arbitrary configuration. The convenience
/// wrappers below are the only spellings tests should use; the
/// builder stays here so a future "exhaustively test both branches"
/// style ever gets a single point to extend.
async fn spawn_app_with(pool: PgPool, opts: SpawnOptions) -> String {
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
        dev_auth_enabled: opts.dev_auth_enabled,
        // The Config copies of the JWT triple are only read at boot
        // (to construct the verifier). The integration tests inject
        // a pre-built verifier instead, so these stay `None`.
        jwt_jwks_url: None,
        jwt_issuer: None,
        jwt_audience: None,
    };

    let state = AppState {
        db: pool,
        jwt_verifier: opts.jwt_verifier,
        dev_auth_enabled: opts.dev_auth_enabled,
    };
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

/// Spawn the app with the dev `X-User-Id` shim **enabled** and no
/// JWT verifier — what the existing 1.b.5 integration tests want.
/// Use [`spawn_app_prod_gate`] for the 503 path, or
/// [`spawn_app_with_jwt`] for the new JWT-middleware tests.
#[allow(dead_code)] // jwt_middleware.rs only uses the JWT-enabled variants
pub async fn spawn_app(pool: PgPool) -> String {
    spawn_app_with(
        pool,
        SpawnOptions {
            dev_auth_enabled: true,
            ..SpawnOptions::default()
        },
    )
    .await
}

/// Spawn the app with the production-default config — dev auth
/// **disabled**, no JWT verifier — so every `/api/v1/*` request
/// short-circuits to 503. Used by the various
/// `dev_auth_gate_returns_503_when_disabled` tests.
#[allow(dead_code)] // some test files don't use this helper
pub async fn spawn_app_prod_gate(pool: PgPool) -> String {
    spawn_app_with(pool, SpawnOptions::default()).await
}

/// Spawn the app with the JWT path enabled (a caller-supplied
/// verifier pointed at a mock JWKS) and the dev shim **off**. The
/// `tests/jwt_middleware.rs` battery uses this to exercise the
/// production-shape auth path end-to-end. Note: `POST /api/v1/users`
/// is the test bootstrap entry — it needs the shim to mint a user
/// with an `external_id`, so callers that need to seed users should
/// flip to [`spawn_app_with_jwt_and_shim`] instead.
#[allow(dead_code)]
pub async fn spawn_app_with_jwt(pool: PgPool, verifier: Arc<JwtVerifier>) -> String {
    spawn_app_with(
        pool,
        SpawnOptions {
            dev_auth_enabled: false,
            jwt_verifier: Some(verifier),
        },
    )
    .await
}

/// Spawn the app with both auth paths enabled. The middleware
/// gives JWT precedence so a request that carries both headers
/// goes through the cryptographically-trusted path. Used by the
/// transition-shape tests that need to mint users via the open
/// `POST /api/v1/users` (shim path) and then authenticate
/// follow-on requests via Bearer.
#[allow(dead_code)]
pub async fn spawn_app_with_jwt_and_shim(pool: PgPool, verifier: Arc<JwtVerifier>) -> String {
    spawn_app_with(
        pool,
        SpawnOptions {
            dev_auth_enabled: true,
            jwt_verifier: Some(verifier),
        },
    )
    .await
}
