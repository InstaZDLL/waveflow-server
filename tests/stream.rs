//! End-to-end tests for `/api/v1/stream/*` + the mint endpoint.
//!
//! Strategy: spin up a temp music-root directory with a known
//! payload, mint a signed URL via the JWT path, then exercise the
//! stream side both with and without `Range`. Also covers the
//! security cases: tampered token, foreign-user mint, traversal
//! attempt, streaming-disabled. Expiry is covered by the
//! `stream_token::tests::rejects_an_expired_token` unit test, so
//! we don't redo it through the HTTP stack.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_app_with_jwt_and_stream, spawn_authenticated};

/// 2 MiB of pseudo-random bytes — large enough that range responses
/// are observably partial AND small enough to read fully in a test.
const PAYLOAD_LEN: usize = 2 * 1024 * 1024;
const STREAM_SECRET: &[u8] = b"unit-test-stream-secret-please-rotate";

fn build_payload() -> Vec<u8> {
    // Deterministic — no randomness, no test flake. The exact bytes
    // don't matter, only that range slices line up against them.
    (0..PAYLOAD_LEN).map(|i| (i % 251) as u8).collect()
}

struct StreamingSetup {
    base: String,
    token: String,
    track_id: i64,
    profile_id: i64,
    library_id: i64,
    payload: Vec<u8>,
    music_root: tempfile::TempDir,
    harness: std::sync::Arc<support::JwksHarness>,
}

/// Bootstrap: spin up an auth'd user + a JWT-only app, swap the app
/// for a streaming-enabled one with the same harness, create a
/// profile + library, drop a payload file in the music root, insert
/// a track row pointing at it, and return everything together.
async fn bootstrap(pool: PgPool, file_path: &str) -> StreamingSetup {
    let payload = build_payload();
    let music_root = tempfile::tempdir().expect("tempdir");
    let abs = music_root.path().join(file_path);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).expect("mkdir music_root subdir");
    }
    std::fs::write(&abs, &payload).expect("write payload");

    // Use the authenticated harness for the bootstrap so the user
    // row + JWT are minted, then spin up a parallel app instance
    // with the streaming context wired in. Both apps see the same
    // Postgres, so the user/profile rows the bootstrap creates are
    // visible to the streaming app instance via its own queries.
    let auth = spawn_authenticated(pool.clone(), "stream-test-user").await;

    let stream_base = spawn_app_with_jwt_and_stream(
        pool.clone(),
        auth.harness.verifier_arc(),
        music_root.path().to_path_buf(),
        STREAM_SECRET.to_vec(),
    )
    .await;

    // Create a profile + library on the streaming-enabled app.
    let profile_id = create_profile(&stream_base, &auth.token, "Stream Tests").await;
    let library_id = create_library(&stream_base, &auth.token, profile_id, "Test Lib").await;
    let track_id = create_track(&stream_base, &auth.token, profile_id, library_id, file_path).await;

    StreamingSetup {
        base: stream_base,
        token: auth.token,
        track_id,
        profile_id,
        library_id,
        payload,
        music_root,
        harness: auth.harness,
    }
}

async fn create_profile(base: &str, token: &str, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .bearer_auth(token)
        .json(&json!({ "name": name, "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_i64().unwrap()
}

async fn create_library(base: &str, token: &str, profile_id: i64, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .bearer_auth(token)
        .json(&json!({ "name": name }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_i64().unwrap()
}

async fn create_track(
    base: &str,
    token: &str,
    profile_id: i64,
    library_id: i64,
    file_path: &str,
) -> i64 {
    let body = json!({
        "title": "Test Song",
        "file_path": file_path,
        "file_size": PAYLOAD_LEN as i64,
        "duration_ms": 240000,
        "track_number": 1,
        "disc_number": 1,
        "year": 2026,
        "bitrate": 320,
        "sample_rate": 44100,
        "channels": 2,
        "bit_depth": 16,
        "codec": "FLAC",
    });
    let created: Value = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_i64().unwrap()
}

async fn mint_stream_url(setup: &StreamingSetup) -> String {
    let resp: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/libraries/{}/tracks/{}/stream-url",
            setup.base, setup.profile_id, setup.library_id, setup.track_id,
        ))
        .bearer_auth(&setup.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    resp["url"].as_str().unwrap().to_string()
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn full_stream_returns_200_and_full_bytes(pool: PgPool) {
    let setup = bootstrap(pool, "Music/song.flac").await;
    let url = mint_stream_url(&setup).await;

    let resp = reqwest::Client::new()
        .get(format!("{}{url}", setup.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok()),
        Some("bytes")
    );
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("audio/flac")
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), PAYLOAD_LEN);
    assert_eq!(&body[..], &setup.payload[..]);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn range_request_returns_206_and_slice(pool: PgPool) {
    let setup = bootstrap(pool, "Music/song.flac").await;
    let url = mint_stream_url(&setup).await;

    let resp = reqwest::Client::new()
        .get(format!("{}{url}", setup.base))
        .header("range", "bytes=1000-1999")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        resp.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some(format!("bytes 1000-1999/{}", PAYLOAD_LEN).as_str())
    );
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), 1000);
    assert_eq!(&body[..], &setup.payload[1000..2000]);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn open_range_returns_206_to_eof(pool: PgPool) {
    let setup = bootstrap(pool, "Music/song.flac").await;
    let url = mint_stream_url(&setup).await;

    let start = PAYLOAD_LEN - 100;
    let resp = reqwest::Client::new()
        .get(format!("{}{url}", setup.base))
        .header("range", format!("bytes={start}-"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), 100);
    assert_eq!(&body[..], &setup.payload[start..]);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn out_of_range_returns_416(pool: PgPool) {
    let setup = bootstrap(pool, "Music/song.flac").await;
    let url = mint_stream_url(&setup).await;

    let resp = reqwest::Client::new()
        .get(format!("{}{url}", setup.base))
        .header("range", format!("bytes={}-99999999", PAYLOAD_LEN + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tampered_token_is_rejected(pool: PgPool) {
    let setup = bootstrap(pool, "Music/song.flac").await;
    let url = mint_stream_url(&setup).await;

    // Flip a character deep inside the signature segment so the
    // change is guaranteed to alter the underlying HMAC bytes. The
    // base64url-no-pad encoding has no insignificant positions, but
    // padding-like trailing characters can still flip to a synonym
    // within the same byte; picking the penultimate char dodges
    // that edge case.
    assert!(url.len() >= 2, "minted URL too short to tamper");
    let mut chars: Vec<char> = url.chars().collect();
    let target = chars.len() - 2;
    chars[target] = if chars[target] == 'a' { 'b' } else { 'a' };
    let tampered: String = chars.into_iter().collect();

    let resp = reqwest::Client::new()
        .get(format!("{}{tampered}", setup.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn foreign_user_minting_a_foreign_track_404s(pool: PgPool) {
    // Both users hit the SAME streaming app + same JWKS harness, so
    // both tokens authenticate cryptographically. The test exercises
    // the tenant-authorization layer instead — user B hits user A's
    // (profile, library, track) triple and the repository's
    // `*_for_user` query refuses to return a row, surfacing as 404
    // (same no-leak rule the rest of the resource endpoints use).
    let setup = bootstrap(pool.clone(), "Music/song.flac").await;
    let foreign_token = setup.harness.mint(
        &support::good_claims("stream-test-foreign"),
        &support::header_with_kid(support::TEST_KID),
    );

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/libraries/{}/tracks/{}/stream-url",
            setup.base, setup.profile_id, setup.library_id, setup.track_id,
        ))
        .bearer_auth(&foreign_token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    drop(setup.music_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn path_traversal_attempts_404(pool: PgPool) {
    // Drop a file OUTSIDE the music root (simulating a path-traversal
    // target) and then attempt to reference it via `../` in the
    // `track.file_path`. Because the mint endpoint signs whatever
    // file_path the row carries, this is the worst-case scenario for
    // the stream side's canonicalize-then-prefix-check guard.
    let setup = bootstrap(pool.clone(), "Music/legit.flac").await;
    let outside_root = tempfile::tempdir().expect("outside tmpdir");
    let secret_path = outside_root.path().join("secret.txt");
    std::fs::write(&secret_path, b"do not exfiltrate").unwrap();

    // Insert a track row with a `../`-laced path. Going through the
    // CREATE handler would require us to circumvent its own validation
    // (the codec / numbers etc. — already validated by serde), so we
    // INSERT directly into the DB. The track resolves under the same
    // user as `setup`'s auth, just via a different file_path.
    let evil_rel = "../../../etc/passwd"; // canonicalize must reject.
    sqlx::query(
        "INSERT INTO track (library_id, title, file_path, file_size, duration_ms, codec, added_at) \
         VALUES ($1, 'evil', $2, 1, 1, 'FLAC', 1)",
    )
    .bind(setup.library_id)
    .bind(evil_rel)
    .execute(&pool)
    .await
    .unwrap();
    let evil_id: i64 =
        sqlx::query_scalar("SELECT id FROM track WHERE file_path = $1 AND library_id = $2")
            .bind(evil_rel)
            .bind(setup.library_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let mint_resp: Value = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/{}/libraries/{}/tracks/{}/stream-url",
            setup.base, setup.profile_id, setup.library_id, evil_id,
        ))
        .bearer_auth(&setup.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let url = mint_resp["url"].as_str().unwrap();

    let resp = reqwest::Client::new()
        .get(format!("{}{url}", setup.base))
        .send()
        .await
        .unwrap();
    // canonicalize() either fails to resolve (file doesn't exist
    // under the music root) or resolves outside it — both map to 404.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    drop(setup.music_root);
    drop(outside_root);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn streaming_disabled_returns_503(pool: PgPool) {
    // No `spawn_app_with_jwt_and_stream` — use the plain auth helper,
    // which leaves `stream_ctx = None`. Both endpoints should 503.
    let auth = spawn_authenticated(pool, "stream-test-disabled").await;

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/profiles/1/libraries/1/tracks/1/stream-url",
            auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp = reqwest::Client::new()
        .get(format!("{}/api/v1/stream/anything", auth.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
