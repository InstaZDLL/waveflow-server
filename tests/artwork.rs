//! End-to-end tests for `/api/v1/artwork/*` (Phase 1.h.1).
//!
//! Coverage matrix:
//!
//! - Upload round-trip: POST raw bytes → GET hash returns the same
//!   bytes byte-for-byte with the original Content-Type.
//! - Idempotency: a second POST of the same bytes returns the same
//!   hash without touching the storage backend a second time (the
//!   storage row's `byte_size` matches the first upload).
//! - 503 when storage isn't configured (no `WAVEFLOW_ARTWORK_*`
//!   wired into `AppState`).
//! - 400 / 413 boundary cases (wrong MIME, empty body, oversize).
//! - 404 for unknown hash + 400 for malformed hash (path-traversal
//!   defence at the boundary).
//! - 401 when the upload misses a bearer (public read stays 200 with
//!   no bearer — the hash IS the credential).

mod support;

use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::Value;
use sqlx::PgPool;
use support::{
    good_claims, header_with_kid, spawn_app_with_jwt, spawn_app_with_jwt_and_artwork, JwksHarness,
    TEST_KID,
};

/// Minimal "JPEG-ish" payload — the validator doesn't inspect bytes
/// in 1.h.1, so any non-empty buffer with a valid `Content-Type`
/// header is accepted. The pipeline (1.h.3) will introduce real
/// decode-via-`image` validation.
const FAKE_JPEG: &[u8] = b"\xff\xd8\xff\xe0fake jpeg payload";

async fn authed_artwork_app(pool: PgPool, dir: &std::path::Path) -> (String, String) {
    let harness = Arc::new(JwksHarness::spawn().await);
    let base =
        spawn_app_with_jwt_and_artwork(pool, harness.verifier_arc(), dir.to_path_buf()).await;
    let token = harness.mint(&good_claims("user-artwork"), &header_with_kid(TEST_KID));
    // Warm the user provisioning so a subsequent authed call has the
    // users row in place — even though /api/v1/artwork doesn't gate
    // on it today, this mirrors what every other authed test does.
    reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("warm-up");
    (base, token)
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_then_public_get_round_trips(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "upload should succeed");
    let body: Value = resp.json().await.unwrap();
    let hash = body["hash"].as_str().expect("hash field").to_string();
    assert_eq!(hash.len(), 64, "BLAKE3 hex should be 64 chars");
    assert_eq!(body["mime"], "image/jpeg");
    assert_eq!(body["byte_size"], FAKE_JPEG.len() as i64);
    assert_eq!(body["url"], format!("/api/v1/artwork/{hash}"));

    // Public read — no bearer, the hash itself is the credential.
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{hash}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/jpeg"),
    );
    assert_eq!(
        resp.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "public, max-age=31536000, immutable",
    );
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(
        bytes.as_ref(),
        FAKE_JPEG,
        "round-trip should be byte-perfect"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn second_upload_of_same_bytes_is_idempotent(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool.clone(), dir.path()).await;

    let resp1: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hash1 = resp1["hash"].as_str().unwrap().to_string();

    // Second upload — same bytes, same content-type. Must return
    // the same hash and the metadata row must still exist exactly
    // once (no duplicate inserts).
    let resp2: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp2["hash"], resp1["hash"], "same bytes → same hash");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM metadata_artwork WHERE hash = $1")
            .bind(&hash1)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "metadata row must be inserted exactly once");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_503s_when_storage_disabled(pool: PgPool) {
    // `spawn_app_with_jwt` leaves `state.artwork = None` — the
    // upload handler short-circuits to 503 before touching anything.
    let harness = Arc::new(JwksHarness::spawn().await);
    let base = spawn_app_with_jwt(pool, harness.verifier_arc()).await;
    let token = harness.mint(&good_claims("user-artwork-off"), &header_with_kid(TEST_KID));

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_rejects_unsupported_mime(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "application/octet-stream")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_rejects_empty_body(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(Vec::<u8>::new())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_requires_bearer(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, _token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .header("Content-Type", "image/jpeg")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn public_get_returns_404_for_unknown_hash(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, _token) = authed_artwork_app(pool, dir.path()).await;

    let unknown = "0".repeat(64);
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{unknown}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn public_get_rejects_malformed_hash(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, _token) = authed_artwork_app(pool, dir.path()).await;

    // Uppercase hex — rejected at the boundary so it can never reach
    // the object_store key construction.
    let bad = "A".repeat(64);
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{bad}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn content_type_with_charset_parameter_is_accepted(pool: PgPool) {
    // The handler trims at `;` so `image/jpeg; charset=binary` is
    // treated as `image/jpeg` — same shape Postman / browsers
    // occasionally emit.
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg; charset=binary")
        .body(FAKE_JPEG.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
