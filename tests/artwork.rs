//! End-to-end tests for `/api/v1/artwork/*` (Phase 1.h.1 + 1.h.3).
//!
//! Coverage matrix:
//!
//! - Upload round-trip: POST real JPEG bytes → GET hash returns the
//!   same bytes byte-for-byte with the original Content-Type.
//! - Pipeline runs synchronously on upload: response carries
//!   `variants[]` with thumb + preview entries; their bytes round-
//!   trip through `GET /api/v1/artwork/{parent}/{variant}` AND
//!   through the bare `GET /api/v1/artwork/{variant_hash}`.
//! - Idempotency: a second POST of the same bytes returns the same
//!   hash + the same variant set without re-running the pipeline.
//! - 503 when storage isn't configured.
//! - 400 / 413 boundary cases (wrong MIME, empty body, oversize,
//!   non-image bytes — pipeline decode failure surfaces as 400).
//! - 404 for unknown parent + 400 for malformed hash / unknown
//!   variant suffix (path-traversal defence at the boundary).
//! - 401 when the upload misses a bearer (public read stays 200 with
//!   no bearer — the hash IS the credential).

mod support;

use std::io::Cursor;
use std::sync::Arc;

use image::{ImageBuffer, ImageFormat, Rgb};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::PgPool;
use support::{
    good_claims, header_with_kid, spawn_app_with_jwt, spawn_app_with_jwt_and_artwork, JwksHarness,
    TEST_KID,
};

/// Generate a decodable JPEG with the given dimensions. 1.h.1 only
/// looked at the Content-Type header; 1.h.3 runs the upload through
/// the resize pipeline (`image::load_from_memory`), so the test
/// payload now has to be a real bitmap. A diagonal gradient keeps
/// the JPEG encoder out of the pure-flat-colour fast path that some
/// decoders skip differently.
fn synth_jpeg(width: u32, height: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(width, height, |x, y| {
        let r = ((x * 255) / width.max(1)) as u8;
        let g = ((y * 255) / height.max(1)) as u8;
        Rgb([r, g, 128])
    });
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
        .expect("encode synth jpeg");
    buf
}

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

    let jpeg = synth_jpeg(800, 600);
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(jpeg.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "upload should succeed");
    let body: Value = resp.json().await.unwrap();
    let hash = body["hash"].as_str().expect("hash field").to_string();
    assert_eq!(hash.len(), 64, "BLAKE3 hex should be 64 chars");
    assert_eq!(body["mime"], "image/jpeg");
    assert_eq!(body["byte_size"], jpeg.len() as i64);
    assert_eq!(body["url"], format!("/api/v1/artwork/{hash}"));
    // Pipeline now runs synchronously — response should advertise
    // the two variants.
    let variants = body["variants"].as_array().expect("variants array");
    assert_eq!(variants.len(), 2, "should ship thumb + preview");
    assert_eq!(variants[0]["variant"], "preview"); // alphabetical
    assert_eq!(variants[1]["variant"], "thumb");

    // Public read — no bearer, the parent hash itself is the credential.
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
    assert_eq!(bytes.as_ref(), jpeg, "round-trip should be byte-perfect");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn second_upload_of_same_bytes_is_idempotent(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool.clone(), dir.path()).await;

    let jpeg = synth_jpeg(800, 600);
    let resp1: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(jpeg.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hash1 = resp1["hash"].as_str().unwrap().to_string();
    let variants1 = resp1["variants"]
        .as_array()
        .expect("variants on first upload");

    // Second upload — same bytes, same content-type. Must return
    // the same hash + the same variant set, and the metadata row
    // must still exist exactly once (no duplicate inserts).
    let resp2: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(jpeg)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp2["hash"], resp1["hash"], "same bytes → same hash");
    let variants2 = resp2["variants"]
        .as_array()
        .expect("variants echoed on idempotent re-upload");
    assert_eq!(
        variants2.len(),
        variants1.len(),
        "variant count must match on idempotent re-upload",
    );
    for (a, b) in variants1.iter().zip(variants2.iter()) {
        assert_eq!(a["variant"], b["variant"]);
        assert_eq!(a["hash"], b["hash"], "variant hashes must be identical");
    }

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
        .body(synth_jpeg(64, 64))
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
        .body(synth_jpeg(64, 64))
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
async fn upload_rejects_oversized_body(pool: PgPool) {
    // Drive the boundary: 4 MiB + 1 byte. The handler-side
    // `body.len() > MAX_UPLOAD_BYTES` check is the authoritative
    // gatekeeper — the auth router widens the axum default body
    // limit just enough (`MAX_UPLOAD_BYTES + 1024`) for this
    // borderline payload to actually reach the handler and produce
    // our own 413, instead of axum's pre-handler 413 which would
    // pass the test for the wrong reason.
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let oversized = vec![0u8; 4 * 1024 * 1024 + 1];

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_requires_bearer(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, _token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .header("Content-Type", "image/jpeg")
        .body(synth_jpeg(64, 64))
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
        .body(synth_jpeg(64, 64))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn upload_rejects_non_image_bytes(pool: PgPool) {
    // MIME header claims JPEG, but the body is plain text — the
    // pipeline's `image::load_from_memory` rejects it. The handler
    // maps the decode error to 400 so the client knows it sent
    // garbage (vs. 5xx which would imply a server-side glitch).
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(b"this is plain text, not an image".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn variant_endpoint_returns_resized_bytes(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    // 1600×900 → preview clamps to 480×270, thumb to 128×72.
    let body: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(synth_jpeg(1600, 900))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let parent_hash = body["hash"].as_str().unwrap().to_string();

    for variant in ["thumb", "preview"] {
        let resp = reqwest::Client::new()
            .get(format!("{base}/api/v1/artwork/{parent_hash}/{variant}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{variant} should be 200");
        assert_eq!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("image/jpeg"),
            "{variant} mime",
        );
        let bytes = resp.bytes().await.unwrap();
        assert!(!bytes.is_empty(), "{variant} bytes non-empty");
        // The variant's BLAKE3 hash must match what the upload
        // response advertised; otherwise the GET handler is serving
        // a different blob than the row claims.
        let expected_hash = body["variants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["variant"] == variant)
            .unwrap()["hash"]
            .as_str()
            .unwrap()
            .to_string();
        let observed_hash = blake3::hash(&bytes).to_hex().to_string();
        assert_eq!(
            observed_hash, expected_hash,
            "{variant} served bytes must match the advertised hash",
        );
    }
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn variant_endpoint_rejects_unknown_variant(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let body: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(synth_jpeg(64, 64))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let parent_hash = body["hash"].as_str().unwrap().to_string();

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{parent_hash}/hero"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn variant_endpoint_returns_404_when_parent_missing(pool: PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let (base, _token) = authed_artwork_app(pool, dir.path()).await;

    // Well-shaped hash but no upload ever happened — variant lookup
    // misses (the variant row needs a parent), so 404.
    let unknown = "0".repeat(64);
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{unknown}/thumb"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn bare_get_resolves_variant_hash_via_fallback(pool: PgPool) {
    // The `UploadResponse.variants[].url` advertises the
    // `/parent/variant` shape, but a client that persisted only the
    // variant hash should still resolve it through the bare
    // `/api/v1/artwork/{hash}` route — the handler falls back to
    // the variant table when the parent metadata misses.
    let dir = tempfile::tempdir().unwrap();
    let (base, token) = authed_artwork_app(pool, dir.path()).await;

    let body: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/artwork"))
        .bearer_auth(&token)
        .header("Content-Type", "image/jpeg")
        .body(synth_jpeg(800, 600))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let thumb_hash = body["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["variant"] == "thumb")
        .unwrap()["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/artwork/{thumb_hash}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.bytes().await.unwrap();
    let observed = blake3::hash(&bytes).to_hex().to_string();
    assert_eq!(
        observed, thumb_hash,
        "bare GET of a variant hash must serve its bytes",
    );
}
