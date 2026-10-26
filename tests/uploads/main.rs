//! Negotiating a file the server has not been given yet.
//!
//! Everything here happens before a byte moves: what the server will take, what
//! it refuses, and what it refuses to say. RFC-008 has the reasoning.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::config::UploadLimits;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod listings;
mod negotiation;
mod sweeps;
mod transfers;

/// An app whose upload limits this test chose.
///
/// The shared fixture's limits are deliberately small but fixed, and a test
/// that has to meet one bound without tripping another — the quota without the
/// session cap, say — needs to say which.
async fn upload_app(
    tune: impl FnOnce(&mut UploadLimits),
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    tune(&mut config.uploads);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

struct Fixture {
    owner: uuid::Uuid,
    library: uuid::Uuid,
    token: String,
}

/// An admin owner, a library on disk, and a bearer token for the router.
async fn fixture(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    name: &str,
) -> Fixture {
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account(name, &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let token = login_token(
        &waveflow_server::app(config, state.clone()),
        name,
        &password,
    )
    .await;
    Fixture {
        owner,
        library,
        token,
    }
}

fn offer(hash_seed: usize, size: i64, extension: &str) -> serde_json::Value {
    serde_json::json!({
        "full_hash": format!("{:064x}", hash_seed),
        "size_bytes": size,
        "extension": extension,
    })
}

async fn negotiate(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    fixture: &Fixture,
    offers: Vec<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let router = waveflow_server::app(config, state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/libraries/{}/uploads", fixture.library))
                .header("authorization", format!("Bearer {}", fixture.token))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "offers": offers }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, json_body(response).await)
}

async fn open_sessions(state: &waveflow_server::AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM upload_session")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// The transfer: fragments, and what the server does with them.
// ---------------------------------------------------------------------------
fn blake3_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

async fn put_chunk(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    token: &str,
    session: &str,
    index: usize,
    bytes: &[u8],
) -> (StatusCode, serde_json::Value) {
    let router = waveflow_server::app(config, state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v2/uploads/{session}/chunks/{index}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/octet-stream")
                .body(Body::from(bytes.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, json_body(response).await)
}

async fn commit(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    token: &str,
    session: &str,
) -> (StatusCode, serde_json::Value) {
    let router = waveflow_server::app(config, state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/uploads/{session}/commit"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, json_body(response).await)
}

/// An opted-in library, and a WAV that lives outside it waiting to be sent.
async fn ready_to_send(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    name: &str,
) -> (Fixture, Vec<u8>) {
    let fixture = fixture(config, state, name).await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let source = config.data_dir.join(format!("{name}-source.wav"));
    write_test_wav(&source);
    let bytes = std::fs::read(&source).unwrap();
    std::fs::remove_file(&source).unwrap();
    (fixture, bytes)
}

async fn open_session(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    fixture: &Fixture,
    bytes: &[u8],
    extension: &str,
) -> String {
    let (status, body) = negotiate(
        config,
        state,
        fixture,
        vec![serde_json::json!({
            "full_hash": blake3_of(bytes),
            "size_bytes": bytes.len(),
            "extension": extension,
        })],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "accepted");
    body["verdicts"][0]["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned()
}
