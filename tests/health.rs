//! End-to-end smoke test for the /health endpoint.
//!
//! Spawns the real axum app against a per-test Postgres database
//! (provisioned by `#[sqlx::test]`), fires a reqwest call at it, and
//! validates the response shape. Template for every future endpoint
//! test — it confirms the router wiring + middleware actually reach
//! the handler, not just that the handler compiles.

mod support;

use serde_json::Value;
use sqlx::PgPool;
use support::spawn_app;

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn health_returns_ok_with_version(pool: PgPool) {
    let base = spawn_app(pool).await;

    let body: Value = reqwest::Client::new()
        .get(format!("{base}/health"))
        .send()
        .await
        .expect("request failed")
        .error_for_status()
        .expect("non-2xx response")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(body["status"], "ok");
    let version = body["version"].as_str().expect("version missing");
    assert!(
        !version.is_empty(),
        "version should mirror CARGO_PKG_VERSION"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn health_propagates_inbound_request_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    let provided = "test-request-id-1234";
    let resp = reqwest::Client::new()
        .get(format!("{base}/health"))
        .header("x-request-id", provided)
        .send()
        .await
        .expect("request failed");

    let echoed = resp
        .headers()
        .get("x-request-id")
        .expect("server dropped x-request-id")
        .to_str()
        .unwrap();

    assert_eq!(
        echoed, provided,
        "server must echo a client-supplied request id verbatim"
    );
}
