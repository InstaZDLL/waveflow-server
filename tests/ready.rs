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

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn migration_creates_library_table(pool: PgPool) {
    // Same canary as `profile`: a renamed / dropped library.sql would
    // pass `sqlx::test` provisioning but every 1.b.5 CRUD test would
    // explode on the first INSERT. Lean on information_schema so the
    // check stays cheap and doesn't depend on the column shape.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'library'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(exists, "library table missing after migrations");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn migration_creates_track_table(pool: PgPool) {
    // Same canary as `library`: a renamed / dropped track.sql would
    // pass `sqlx::test` provisioning but every 1.b.5b CRUD test
    // would explode on the first INSERT.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'track'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(exists, "track table missing after migrations");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn migration_creates_playlist_table(pool: PgPool) {
    // Same canary as the other tables: a renamed / dropped
    // playlist.sql would pass `sqlx::test` provisioning but every
    // 1.b.5c CRUD test would explode on the first INSERT.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM information_schema.tables
            WHERE table_schema = 'public' AND table_name = 'playlist'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(exists, "playlist table missing after migrations");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn migration_adds_users_external_id_column(pool: PgPool) {
    // Phase 1.d.1 seed — the JWT middleware (lands in PR2/PR3)
    // resolves `sub` claims against this column. A renamed / dropped
    // migration would let the middleware compile but every JWT
    // lookup would fail with a column-not-found at runtime; cheaper
    // to fail this canary at boot.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public'
              AND table_name   = 'users'
              AND column_name  = 'external_id'
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(exists, "users.external_id column missing after migrations");
}

/// Defense-in-depth probe on the `users.external_id` column shape.
/// Direct INSERT (bypassing the lazy-provision middleware) so a
/// future regression that loosens the constraints surfaces here.
///
/// Phase 1.d.2 made the column NOT NULL — JWT auth is the only path
/// and every row must carry the `sub` claim it was provisioned from.
/// Blank values still trip the `users_external_id_non_blank` CHECK
/// (`23514`), distinct from the unique-violation case (`23505`) the
/// upsert in `find_or_provision_by_external_id` handles.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn users_external_id_rejects_null_and_blank(pool: PgPool) {
    // NULL is now forbidden — the 1.d.2 migration set the column
    // NOT NULL. SQLSTATE 23502 = not_null_violation.
    let err = sqlx::query("INSERT INTO users (created_at, external_id) VALUES (1, NULL)")
        .execute(&pool)
        .await
        .expect_err("NULL external_id should violate NOT NULL");
    let code = match &err {
        sqlx::Error::Database(db_err) => db_err.code().map(|c| c.into_owned()),
        other => panic!("expected Database error, got {other:?}"),
    };
    assert_eq!(
        code.as_deref(),
        Some("23502"),
        "NULL external_id should fail with not_null_violation (23502), got {code:?}"
    );

    // Empty string and whitespace-only must trip the CHECK with
    // SQLSTATE 23514 (check_violation).
    for blank in ["", "   ", "\t\n "] {
        let err = sqlx::query("INSERT INTO users (created_at, external_id) VALUES (1, $1)")
            .bind(blank)
            .execute(&pool)
            .await
            .expect_err(&format!(
                "external_id = {blank:?} should trip the CHECK constraint"
            ));
        let code = match &err {
            sqlx::Error::Database(db_err) => db_err.code().map(|c| c.into_owned()),
            other => panic!("expected Database error, got {other:?}"),
        };
        assert_eq!(
            code.as_deref(),
            Some("23514"),
            "external_id = {blank:?} should fail with check_violation (23514), got {code:?}"
        );
    }
}
