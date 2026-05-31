//! Shared integration-test harness: spawn the real axum app on a
//! kernel-assigned port against a caller-provided Postgres pool, mint
//! a JWT off a per-test JWKS harness, and hand back the URL + a token
//! the test should authenticate with.
//!
//! Each test that uses this gets its pool from `#[sqlx::test(...)]`,
//! which creates a fresh per-test database, runs migrations, and drops
//! the database when the test exits — no fixtures to clean up
//! manually.
//!
//! Phase 1.d.2 collapsed every spawn variant down to the JWT-only
//! flow. The legacy `spawn_app` (shim) + `spawn_app_with_jwt_and_shim`
//! variants are gone alongside the `X-User-Id` header itself.

#![allow(dead_code)]

// Cargo compiles each `tests/*.rs` as its own crate, so each test
// file does `mod support; mod jwks_harness;` separately. We need
// `jwks_harness` accessible from inside `support.rs` too — but the
// default module resolution from here would look for
// `tests/support/jwks_harness.rs`. The `#[path]` attribute tells
// rustc to read the sibling file directly.
#[path = "jwks_harness.rs"]
mod jwks_harness;

use std::{net::SocketAddr, sync::Arc};

use sqlx::PgPool;
use waveflow_server::{app, auth::JwtVerifier, AppState, Config};

pub use jwks_harness::{good_claims, header_with_kid, JwksHarness, TEST_KID};

/// Spawn the app with a fresh JwksHarness verifier and return the
/// base URL — for tests that hit unauthenticated routes (`/health`,
/// `/ready`, `/openapi.json`, `/reference`) and don't care about
/// the auth surface. Use [`spawn_app_with_jwt`] when the test needs
/// to wire its own harness, or [`spawn_authenticated`] when it needs
/// a token + provisioned user id.
pub async fn spawn_app(pool: PgPool) -> String {
    let harness = JwksHarness::spawn().await;
    spawn_app_with_jwt(pool, harness.verifier_arc()).await
}

/// Spawn the app with a caller-supplied verifier (pointed at a mock
/// JWKS via `JwksHarness`) and return the base URL. Use
/// [`spawn_authenticated`] for the common case where the test also
/// needs a token + pre-provisioned user id.
pub async fn spawn_app_with_jwt(pool: PgPool, verifier: Arc<JwtVerifier>) -> String {
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
        // The Config copies of the JWT triple are only read at boot
        // (to construct the verifier). The integration tests inject
        // a pre-built verifier instead, so these stay empty.
        jwt_jwks_url: String::new(),
        jwt_issuer: String::new(),
        jwt_audience: String::new(),
    };

    let state = AppState {
        db: pool,
        jwt_verifier: verifier,
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

/// One-stop bootstrap for tenant-scoped integration tests:
/// - spins up a JwksHarness + the app wired to it,
/// - mints a token with the supplied `external_id`,
/// - fires a no-op authenticated request so the lazy-provision
///   middleware lands the `users` row and we know its id,
/// - returns everything the test needs to keep going.
///
/// Use this whenever a test needs a "the user is signed in" baseline.
/// Tests that exercise the auth surface itself (bad signature, no kid,
/// etc.) should bypass this and drive [`JwksHarness::mint`] directly.
pub struct Authenticated {
    pub base: String,
    pub token: String,
    pub user_id: i64,
    pub external_id: String,
    pub harness: Arc<JwksHarness>,
    pub pool: PgPool,
}

/// Mint an authenticated test caller with the supplied external_id.
/// Tests that need a second caller call this again with a distinct
/// external_id — each spawns its own harness so the verifier keys
/// don't collide.
pub async fn spawn_authenticated(pool: PgPool, external_id: &str) -> Authenticated {
    let harness = Arc::new(JwksHarness::spawn().await);
    let base = spawn_app_with_jwt(pool.clone(), harness.verifier_arc()).await;
    let token = harness.mint(&good_claims(external_id), &header_with_kid(TEST_KID));

    // Fire a no-op authenticated request so the middleware lazy-
    // provisions the users row. We then SELECT the row to learn its
    // id (BIGSERIAL means the assignment is opaque to the client).
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("warm-up request failed");
    assert!(
        resp.status().is_success(),
        "warm-up request returned {} (body: {:?})",
        resp.status(),
        resp.text().await.ok(),
    );

    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = $1")
        .bind(external_id)
        .fetch_one(&pool)
        .await
        .expect("lazy-provision did not land the users row");

    Authenticated {
        base,
        token,
        user_id,
        external_id: external_id.to_string(),
        harness,
        pool,
    }
}

/// Convenience for tests that need TWO authenticated callers
/// sharing the same base URL — typical pattern for tenant-isolation
/// assertions. Both tokens are minted from the same JWKS harness,
/// so a single app instance verifies both: that mirrors the real
/// deployment where every caller hits the same waveflow-server +
/// the same Better Auth issuer.
pub struct TwoAuthenticated {
    /// Base URL of the single app both callers hit.
    pub base: String,
    pub a: AuthenticatedCaller,
    pub b: AuthenticatedCaller,
    pub harness: Arc<JwksHarness>,
    pub pool: PgPool,
}

pub struct AuthenticatedCaller {
    pub token: String,
    pub user_id: i64,
    pub external_id: String,
}

pub async fn spawn_two_authenticated(
    pool: PgPool,
    external_a: &str,
    external_b: &str,
) -> TwoAuthenticated {
    let harness = Arc::new(JwksHarness::spawn().await);
    let base = spawn_app_with_jwt(pool.clone(), harness.verifier_arc()).await;

    let token_a = harness.mint(&good_claims(external_a), &header_with_kid(TEST_KID));
    let token_b = harness.mint(&good_claims(external_b), &header_with_kid(TEST_KID));

    // Warm both users in via the lazy-provision middleware.
    for token in [&token_a, &token_b] {
        let resp = reqwest::Client::new()
            .get(format!("{base}/api/v1/profiles"))
            .bearer_auth(token)
            .send()
            .await
            .expect("warm-up request failed");
        assert!(
            resp.status().is_success(),
            "warm-up failed: {}",
            resp.status()
        );
    }

    let user_a: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = $1")
        .bind(external_a)
        .fetch_one(&pool)
        .await
        .expect("user A row missing after warm-up");
    let user_b: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = $1")
        .bind(external_b)
        .fetch_one(&pool)
        .await
        .expect("user B row missing after warm-up");

    TwoAuthenticated {
        base,
        a: AuthenticatedCaller {
            token: token_a,
            user_id: user_a,
            external_id: external_a.to_string(),
        },
        b: AuthenticatedCaller {
            token: token_b,
            user_id: user_b,
            external_id: external_b.to_string(),
        },
        harness,
        pool,
    }
}
