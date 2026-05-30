//! End-to-end smoke test for the /ready endpoint.
//!
//! `#[sqlx::test]` provisions a fresh Postgres database for each test
//! and runs the embedded migrator, so the readiness probe sees a
//! healthy connection. A future test exercising the unhappy path
//! (degraded DB) would close the pool and assert on the 503 — out of
//! scope for 1.b.2b.

mod support;

use serde_json::Value;
use sqlx::PgPool;
use support::spawn_app;

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn ready_succeeds_when_db_reachable(pool: PgPool) {
    let base = spawn_app(pool).await;

    let body: Value = reqwest::Client::new()
        .get(format!("{base}/ready"))
        .send()
        .await
        .expect("request failed")
        .error_for_status()
        .expect("non-2xx response")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(body["status"], "ready");
    assert_eq!(body["db"], "ok");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn migration_creates_profile_table(pool: PgPool) {
    // Belt-and-braces check on the embedded migrator: if the .sql file
    // were ever dropped or renamed, sqlx::test would still run but the
    // profile table wouldn't exist. Verify it lands so the rest of the
    // CRUD work in 1.b.4 can lean on it.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'profile'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(exists, "profile table missing after migrations");
}
