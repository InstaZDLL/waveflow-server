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
mod support;
use support::*;

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

/// A library nobody opened refuses, and says so rather than staying silent.
///
/// The flag is the whole point of decision 1: upgrading a server must not turn
/// a read-only installation into one that grows. The owner here is an admin and
/// the library's owner — the only thing standing between them and an open
/// session is the flag.
#[tokio::test]
async fn a_library_that_was_never_opened_refuses_every_offer() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "closed-library").await;

    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4096, "flac"), offer(2, 4096, "flac")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let verdicts = body["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), 2);
    for verdict in verdicts {
        assert_eq!(verdict["decision"], "library_closed");
        assert!(verdict["session"].is_null());
    }
    assert_eq!(
        open_sessions(&state).await,
        0,
        "a closed library must not have opened a session"
    );
}

/// Opened, the same offers earn sessions — and the session carries what the
/// client needs to start sending.
#[tokio::test]
async fn an_opened_library_accepts_and_opens_a_session() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "open-library").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(&config, &state, &fixture, vec![offer(1, 4096, "flac")]).await;

    assert_eq!(status, StatusCode::OK);
    let verdict = &body["verdicts"][0];
    assert_eq!(verdict["decision"], "accepted");
    assert_eq!(verdict["full_hash"], format!("{:064x}", 1));
    let session = &verdict["session"];
    assert_eq!(session["next_chunk"], 0);
    assert_eq!(session["received_bytes"], 0);
    assert_eq!(session["chunk_bytes"], config.uploads.chunk_bytes);
    assert!(session["session_id"].is_string());
    assert_eq!(open_sessions(&state).await, 1);
}

/// A member who may not upload is told nothing at all.
///
/// Not `library_closed`: that answer would confirm the library exists to
/// somebody who has no business knowing whether it does. The listener here is a
/// member of the library, and it is open — so the 404 is about the role and
/// nothing else.
#[tokio::test]
async fn a_listener_is_refused_the_way_a_stranger_is() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let owner_fixture = fixture(&config, &state, "listener-owner").await;
    state
        .db
        .set_library_accepts_uploads(owner_fixture.owner, owner_fixture.library, true, now_ms())
        .await
        .unwrap();

    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let listener = state
        .db
        .create_account("listener-member", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner_fixture.owner,
            owner_fixture.library,
            listener,
            LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let listener_token = login_token(
        &waveflow_server::app(&config, state.clone()),
        "listener-member",
        &password,
    )
    .await;

    let as_listener = Fixture {
        owner: listener,
        library: owner_fixture.library,
        token: listener_token,
    };
    let (listener_status, _) =
        negotiate(&config, &state, &as_listener, vec![offer(1, 4096, "flac")]).await;

    // A stranger to the library, for comparison: the two answers must be
    // indistinguishable, which is the whole of the rule.
    let stranger = fixture(&config, &state, "stranger").await;
    let as_stranger = Fixture {
        owner: stranger.owner,
        library: owner_fixture.library,
        token: stranger.token,
    };
    let (stranger_status, _) =
        negotiate(&config, &state, &as_stranger, vec![offer(1, 4096, "flac")]).await;

    assert_eq!(listener_status, StatusCode::NOT_FOUND);
    assert_eq!(stranger_status, StatusCode::NOT_FOUND);
    assert_eq!(open_sessions(&state).await, 0);
}

/// A file this library already holds is not wanted, and the answer names the
/// track so the client can reconcile without sweeping the catalogue.
#[tokio::test]
async fn a_file_the_library_already_holds_is_not_wanted() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "already-held").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = catalog_input(1, "Held Artist");
    input.full_hash = format!("{:064x}", 7);
    state
        .db
        .apply_catalog_track(fixture.library, scan_id, &input, None, false)
        .await
        .unwrap();

    let (status, body) = negotiate(&config, &state, &fixture, vec![offer(7, 4096, "flac")]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "present");
    assert!(
        body["verdicts"][0]["track_id"].is_string(),
        "a present verdict names the track that holds those bytes"
    );
    assert_eq!(
        open_sessions(&state).await,
        0,
        "nothing to transfer means nothing to open"
    );
}

/// The same bytes in a different library are still wanted here.
///
/// The lookup is scoped to the target library, and both halves of that matter:
/// a server-wide answer would leave this library believing it holds a track it
/// does not, and would tell its member that a library they cannot see holds
/// exactly this file.
#[tokio::test]
async fn a_file_another_library_holds_is_still_wanted_here() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let elsewhere = fixture(&config, &state, "other-library").await;
    let scan_id = state
        .db
        .create_scan_job(elsewhere.library, Some(elsewhere.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = catalog_input(1, "Elsewhere Artist");
    input.full_hash = format!("{:064x}", 9);
    state
        .db
        .apply_catalog_track(elsewhere.library, scan_id, &input, None, false)
        .await
        .unwrap();

    let here = fixture(&config, &state, "this-library").await;
    state
        .db
        .set_library_accepts_uploads(here.owner, here.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(&config, &state, &here, vec![offer(9, 4096, "flac")]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["verdicts"][0]["decision"], "accepted",
        "another library holding these bytes says nothing about this one"
    );
    assert!(body["verdicts"][0]["track_id"].is_null());
}

/// An extension the scanner cannot index earns a verdict, not a session.
#[tokio::test]
async fn a_format_the_catalogue_cannot_read_is_refused() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "bad-format").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4096, "exe"), offer(2, 4096, "flac")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "unsupported_format");
    assert!(body["verdicts"][0]["session"].is_null());
    assert_eq!(
        body["verdicts"][1]["decision"], "accepted",
        "one bad offer decides only itself"
    );
    assert_eq!(open_sessions(&state).await, 1);
}

/// Above the per-file ceiling, refused before a byte moves.
#[tokio::test]
async fn a_file_above_the_ceiling_is_refused() {
    let (_temp, config, state) = upload_app(|limits| limits.max_file_bytes = 4096).await;
    let fixture = fixture(&config, &state, "too-large").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4097, "flac"), offer(2, 4096, "flac")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "too_large");
    assert_eq!(body["verdicts"][1]["decision"], "accepted");
}

/// Two offers in one batch cannot both be told there is room for them.
///
/// This is the reservation, and it is the reason a session holds what it
/// declared: with the quota read once and never held, both offers below see the
/// same free space and the library ends up over its ceiling with neither
/// negotiation having done anything wrong.
#[tokio::test]
async fn the_second_offer_sees_what_the_first_one_reserved() {
    let (_temp, config, state) = upload_app(|limits| {
        limits.max_file_bytes = 4096;
        limits.library_quota_bytes = 6000;
        limits.sessions_per_user = 8;
    })
    .await;
    let fixture = fixture(&config, &state, "quota-race").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4096, "flac"), offer(2, 4096, "flac")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "accepted");
    assert_eq!(
        body["verdicts"][1]["decision"], "quota_exceeded",
        "the first offer's reservation is part of what the second one sees"
    );
    assert_eq!(open_sessions(&state).await, 1);
}

/// A separate negotiation sees the earlier one's reservation too.
#[tokio::test]
async fn a_later_negotiation_sees_an_open_session_reservation() {
    let (_temp, config, state) = upload_app(|limits| {
        limits.max_file_bytes = 4096;
        limits.library_quota_bytes = 6000;
        limits.sessions_per_user = 8;
    })
    .await;
    let fixture = fixture(&config, &state, "quota-later").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (_, first) = negotiate(&config, &state, &fixture, vec![offer(1, 4096, "flac")]).await;
    assert_eq!(first["verdicts"][0]["decision"], "accepted");
    let (_, second) = negotiate(&config, &state, &fixture, vec![offer(2, 4096, "flac")]).await;
    assert_eq!(second["verdicts"][0]["decision"], "quota_exceeded");

    // Expire the reservation and the room comes back — which is what says the
    // refusal above was the reservation and not something permanent.
    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    let (_, third) = negotiate(&config, &state, &fixture, vec![offer(2, 4096, "flac")]).await;
    assert_eq!(third["verdicts"][0]["decision"], "accepted");
    assert_eq!(
        open_sessions(&state).await,
        1,
        "the expired session was swept rather than left holding space"
    );
}

/// What a library already holds counts against its quota.
#[tokio::test]
async fn the_catalogue_counts_against_the_quota() {
    let (_temp, config, state) = upload_app(|limits| {
        limits.max_file_bytes = 4096;
        limits.library_quota_bytes = 6000;
    })
    .await;
    let fixture = fixture(&config, &state, "quota-catalogue").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = catalog_input(1, "Occupying Artist");
    input.full_hash = format!("{:064x}", 42);
    input.file_size = 4000;
    state
        .db
        .apply_catalog_track(fixture.library, scan_id, &input, None, false)
        .await
        .unwrap();

    let (status, body) = negotiate(&config, &state, &fixture, vec![offer(1, 4096, "flac")]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["verdicts"][0]["decision"], "quota_exceeded",
        "the files already on disk are what the quota is about"
    );
}

/// Re-offering a file finds the session it already has.
///
/// A client that restarts mid-transfer sends its offers again. A second session
/// would strand the first one's reservation until it expired, and leave the
/// staging area it had already filled behind.
#[tokio::test]
async fn re_offering_a_file_finds_its_open_session() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "resumed").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (_, first) = negotiate(&config, &state, &fixture, vec![offer(1, 4096, "flac")]).await;
    let session_id = first["verdicts"][0]["session"]["session_id"].clone();
    // Pretend the client got some of it across before it restarted.
    sqlx::query("UPDATE upload_session SET next_chunk=3, received_bytes=1234")
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, second) = negotiate(&config, &state, &fixture, vec![offer(1, 4096, "flac")]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["verdicts"][0]["decision"], "accepted");
    assert_eq!(
        second["verdicts"][0]["session"]["session_id"], session_id,
        "the same file must find the same session"
    );
    assert_eq!(
        second["verdicts"][0]["session"]["next_chunk"], 3,
        "and it must say where the transfer actually stopped"
    );
    assert_eq!(second["verdicts"][0]["session"]["received_bytes"], 1234);
    assert_eq!(open_sessions(&state).await, 1);
}

/// An account holds as many sessions as it may, and no more.
#[tokio::test]
async fn an_account_cannot_hold_more_sessions_than_its_share() {
    let (_temp, config, state) = upload_app(|limits| limits.sessions_per_user = 1).await;
    let fixture = fixture(&config, &state, "session-cap").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4096, "flac"), offer(2, 4096, "flac")],
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "accepted");
    assert_eq!(body["verdicts"][1]["decision"], "too_many_sessions");
    assert_eq!(open_sessions(&state).await, 1);
}

/// A full batch does not fit in the router's global ceiling, and still gets
/// through.
///
/// Two hundred offers is roughly twenty kilobytes of JSON against a global
/// sixteen-kilobyte limit. The route carries its own bound so the rest of the
/// server keeps the small one: an API whose every route accepts sixteen
/// kilobytes cannot be drowned in a request body, and that is worth more than
/// the convenience of raising it everywhere.
#[tokio::test]
async fn a_full_batch_passes_a_ceiling_the_rest_of_the_server_keeps() {
    let (_temp, config, state) = upload_app(|limits| {
        limits.batch_limit = 200;
        limits.sessions_per_user = 200;
        limits.library_quota_bytes = 1 << 30;
    })
    .await;
    let fixture = fixture(&config, &state, "full-batch").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let offers: Vec<serde_json::Value> = (1..=200).map(|seed| offer(seed, 4096, "flac")).collect();
    let encoded = serde_json::json!({ "offers": offers }).to_string();
    assert!(
        encoded.len() > 16 * 1024,
        "the point of this test is a body the global limit would refuse, got {}",
        encoded.len()
    );

    let (status, body) = negotiate(&config, &state, &fixture, offers).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"].as_array().unwrap().len(), 200);
    assert_eq!(open_sessions(&state).await, 200);
}

/// A batch is bounded, and a malformed offer fails the request rather than
/// earning a verdict: a hash that is not a hash is a client bug, not a file the
/// server declines.
#[tokio::test]
async fn a_malformed_or_oversized_batch_is_refused_whole() {
    let (_temp, config, state) = upload_app(|limits| limits.batch_limit = 2).await;
    let fixture = fixture(&config, &state, "malformed").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();

    let (too_many, _) = negotiate(
        &config,
        &state,
        &fixture,
        vec![
            offer(1, 1, "flac"),
            offer(2, 1, "flac"),
            offer(3, 1, "flac"),
        ],
    )
    .await;
    assert_eq!(too_many, StatusCode::UNPROCESSABLE_ENTITY);

    let (empty, _) = negotiate(&config, &state, &fixture, Vec::new()).await;
    assert_eq!(empty, StatusCode::UNPROCESSABLE_ENTITY);

    let (bad_hash, _) = negotiate(
        &config,
        &state,
        &fixture,
        vec![serde_json::json!({
            "full_hash": "not-a-hash",
            "size_bytes": 4096,
            "extension": "flac",
        })],
    )
    .await;
    assert_eq!(bad_hash, StatusCode::UNPROCESSABLE_ENTITY);

    let (repeated, _) = negotiate(
        &config,
        &state,
        &fixture,
        vec![offer(1, 4096, "flac"), offer(1, 4096, "flac")],
    )
    .await;
    assert_eq!(
        repeated,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the same file twice in one batch would reserve twice and race itself"
    );

    assert_eq!(open_sessions(&state).await, 0);
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

/// The whole thing: a file arrives in pieces and is a track by the time the
/// server answers.
///
/// Waiting for the next scan would leave the client with a successful transfer
/// and a catalogue that ignores it, with nothing to say for how long.
#[tokio::test]
async fn a_file_arrives_in_fragments_and_is_a_track_at_once() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "arrives").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let mut sent = 0usize;
    let mut index = 0usize;
    while sent < bytes.len() {
        let end = (sent + 512).min(bytes.len());
        let (status, state_body) = put_chunk(
            &config,
            &state,
            &fixture.token,
            &session,
            index,
            &bytes[sent..end],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "fragment {index}");
        assert_eq!(state_body["next_chunk"], index as i64 + 1);
        assert_eq!(state_body["received_bytes"], end as i64);
        sent = end;
        index += 1;
    }
    assert!(
        index > 1,
        "the point of this test is more than one fragment"
    );

    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        committed["full_hash"],
        blake3_of(&bytes),
        "the hash is what the server computed, and it matches what arrived"
    );
    let track_id = committed["track_id"].as_str().unwrap();

    // The track exists now, not at the next scan.
    let tracks = state
        .db
        .list_tracks_for_user(fixture.owner, fixture.library)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id.to_string(), track_id);

    // Filed by hash, under the directory the server owns, keeping the
    // extension the walk recognises it by.
    let expected = format!(".waveflow-managed/{}.wav", blake3_of(&bytes));
    assert_eq!(tracks[0].relative_path, expected);
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    assert!(root.join(&expected).is_file());

    // The library feed announced it, because the apply did — nothing here had
    // to remember to.
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event \
         WHERE library_id=? AND entity_type='track' AND action='upsert'",
    )
    .bind(fixture.library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(events, 1);

    // And the session is gone, so the space it reserved is now the space the
    // file occupies rather than both at once.
    assert_eq!(open_sessions(&state).await, 0);
}

/// A fragment sent twice is answered, not rejected.
///
/// An acknowledgement lost after the write is what a dropped link ordinarily
/// produces. Treating the client's honest retry as a fault would make the
/// protocol fragile exactly where it exists to absorb interruptions.
#[tokio::test]
async fn a_fragment_sent_twice_is_answered_rather_than_refused() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "repeated").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let (first, first_body) =
        put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);
    assert_eq!(first_body["next_chunk"], 1);

    let (again, again_body) =
        put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(again, StatusCode::OK, "a re-sent fragment is not a fault");
    assert_eq!(
        again_body["next_chunk"], 1,
        "and it does not advance the transfer a second time"
    );
    assert_eq!(again_body["received_bytes"], 512);
}

/// A fragment out of order is refused, because the gap would only surface at
/// the final hash.
#[tokio::test]
async fn a_fragment_from_the_future_is_refused() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "future").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let (status, _) = put_chunk(
        &config,
        &state,
        &fixture.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // And a fragment short of what its position calls for, which would put
    // every later one at the wrong offset.
    let (short, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..10]).await;
    assert_eq!(short, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Bytes that are not what was promised are discarded, and leave nothing.
///
/// The declared hash exists to avoid a transfer. Letting it establish an
/// identity would let any authorised member pass one file off as another.
#[tokio::test]
async fn bytes_that_are_not_what_was_promised_are_discarded() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "mismatch").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    // The same length, different content — so only the hash can tell.
    let mut other = bytes.clone();
    let last = other.len() - 1;
    other[last] ^= 0xff;
    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &other).await;
    assert_eq!(status, StatusCode::OK);

    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(committed, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0,
        "nothing was catalogued"
    );
    assert_eq!(open_sessions(&state).await, 0, "and the session is closed");
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let leftovers: Vec<_> = std::fs::read_dir(root.join(".waveflow-managed"))
        .map(|dir| dir.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "and nothing was left on the disk: {leftovers:?}"
    );
}

/// A file the catalogue cannot read does not get to stay.
///
/// The extension was never proof. Reading the file through the scan's own
/// extractor is, and what it cannot open is removed rather than left occupying
/// a disk the catalogue could never show it on.
#[tokio::test]
async fn a_file_the_catalogue_cannot_read_does_not_stay() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let fixture = fixture(&config, &state, "unreadable").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let bytes = b"MZ this is not audio, whatever it is called".to_vec();
    let session = open_session(&config, &state, &fixture, &bytes, "flac").await;

    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(status, StatusCode::OK);

    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(committed, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let leftovers: Vec<_> = std::fs::read_dir(root.join(".waveflow-managed"))
        .map(|dir| dir.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "the file that could not be read was removed: {leftovers:?}"
    );
}

/// A session is not an authorisation the client carries away with it.
///
/// The server already holds this rule for playback: a stream ticket re-checks
/// membership on each redemption so revoking access takes effect immediately
/// rather than when the ticket expires. A transfer lasts far longer than a
/// ticket and costs far more, so a member removed mid-transfer must stop
/// writing at the next request rather than at the end of the file.
#[tokio::test]
async fn a_member_removed_mid_transfer_stops_writing() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (owner_fixture, bytes) = ready_to_send(&config, &state, "revoked-owner").await;

    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let manager = state
        .db
        .create_account("revoked-manager", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner_fixture.owner,
            owner_fixture.library,
            manager,
            LibraryRole::Manager,
            now_ms(),
        )
        .await
        .unwrap();
    let token = login_token(
        &waveflow_server::app(&config, state.clone()),
        "revoked-manager",
        &password,
    )
    .await;
    let as_manager = Fixture {
        owner: manager,
        library: owner_fixture.library,
        token,
    };

    let session = open_session(&config, &state, &as_manager, &bytes, "wav").await;
    let (first, _) = put_chunk(
        &config,
        &state,
        &as_manager.token,
        &session,
        0,
        &bytes[..512],
    )
    .await;
    assert_eq!(first, StatusCode::OK);

    state
        .db
        .remove_library_member(
            owner_fixture.owner,
            owner_fixture.library,
            manager,
            now_ms(),
        )
        .await
        .unwrap();

    let (after, _) = put_chunk(
        &config,
        &state,
        &as_manager.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(
        after,
        StatusCode::NOT_FOUND,
        "a session must not outlive the membership that justified it"
    );
    let (committed, _) = commit(&config, &state, &as_manager.token, &session).await;
    assert_eq!(
        committed,
        StatusCode::NOT_FOUND,
        "and the commit is re-checked too, not only the fragments"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(owner_fixture.owner, owner_fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
}

/// A library closed while a transfer was running stops it, and says which it
/// was.
///
/// Not a 404: the caller is still entitled to be here. It is the library that
/// stopped taking files, and saying so is what lets a client stop instead of
/// retrying.
#[tokio::test]
async fn a_library_closed_mid_transfer_stops_the_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "closed-midway").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();

    let (after, _) = put_chunk(
        &config,
        &state,
        &fixture.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(after, StatusCode::CONFLICT);
    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(committed, StatusCode::CONFLICT);
}

/// A half-written file is never something a scan can index.
///
/// Not because the directory is hidden — nothing about the walk skips hidden
/// directories, and counting on that would be an illusion. Because the walk
/// recognises a file by its extension, and a fragment carries none it knows.
///
/// The fragment here is a **complete, playable WAV** that is only part of what
/// the transfer promised. That is the case that matters: a truncated file would
/// fail to parse and be skipped whatever it was called, so a test built on one
/// proves nothing about the naming rule. This one a scan would happily index —
/// as a track whose hash stops being true the moment the next fragment lands.
#[tokio::test]
async fn nothing_half_written_is_ever_visible_to_a_scan() {
    // Built before the app, because the fragment size has to be the whole
    // file's length and the service copies its limits at startup.
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("half-written-source.wav");
    write_test_wav(&source);
    let whole = std::fs::read(&source).unwrap();

    // A fragment the size of the whole file, and a promise of more to come: the
    // staging area ends up holding a file that is valid on its own.
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = whole.len() as i64).await;
    let fixture = fixture(&config, &state, "half-written").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![serde_json::json!({
            "full_hash": blake3_of(b"a longer file that will never finish arriving"),
            "size_bytes": whole.len() + 512,
            "extension": "wav",
        })],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "accepted");
    let session = body["verdicts"][0]["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (sent, sent_body) = put_chunk(&config, &state, &fixture.token, &session, 0, &whole).await;
    assert_eq!(sent, StatusCode::OK);
    assert_eq!(sent_body["received_bytes"], whole.len() as i64);

    let library = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap();
    // The staging file really is inside the library the scan walks, and it
    // really is a file a scan could read — so this test fails for the right
    // reason if the naming rule ever changes.
    let staged = library
        .root_path
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staged.is_file());
    assert_eq!(std::fs::read(&staged).unwrap(), whole);

    run_scan(&state, fixture.owner, library).await;

    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0,
        "an unfinished transfer must not become a track whose hash stops being true"
    );
}

/// A received file survives the scan that was already running when it landed.
///
/// That scan started its walk before the file existed, so it cannot have found
/// it. Sweeping it would mark a file that is on disk as gone and announce a
/// deletion to every client seconds after announcing the arrival.
#[tokio::test]
async fn a_received_file_survives_a_scan_that_began_before_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "survives").await;

    // A scan that began before anything was sent — the one whose walk cannot
    // have seen the file.
    let running = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(running, 0, false).await.unwrap();

    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);
    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    let track_id = committed["track_id"].as_str().unwrap().to_owned();

    // And now that scan finishes, sweeping what it did not find.
    let swept = state
        .db
        .mark_unseen_unavailable(fixture.library, running)
        .await
        .unwrap();
    assert_eq!(swept, 0);
    let available: i64 = sqlx::query_scalar("SELECT is_available FROM track WHERE id=?")
        .bind(&track_id)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(available, 1, "the file is on disk; it is not gone");
    let deletes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event WHERE library_id=? AND action='delete'",
    )
    .bind(fixture.library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(deletes, 0, "and no deletion was announced");

    // The next ordinary scan walks past it and stamps it, after which it is an
    // ordinary track in every respect.
    let library = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap();
    run_scan(&state, fixture.owner, library).await;
    let stamped: Option<String> =
        sqlx::query_scalar("SELECT last_seen_scan_id FROM track WHERE id=?")
            .bind(&track_id)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        stamped.is_some(),
        "the first scan to walk past a received file stamps it"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        1,
        "and it does not become a second track"
    );
}

/// An expired session takes its staging area with it.
#[tokio::test]
async fn an_expired_session_takes_its_staging_file_with_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "expiring").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staging.is_file(), "there is something to clean up");

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    // Any negotiation sweeps first, which is where the cleanup happens.
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(999, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !staging.exists(),
        "an abandoned transfer must not keep occupying a disk the quota measures"
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// Committing too early is refused, and costs the client nothing.
///
/// The size check is cheap and it comes first for a reason that is not
/// redundancy: without it the hash would catch the short file anyway, but by
/// then it looks like bytes that are not what was promised — and that verdict
/// destroys the session and its staging area. A client that asked too soon
/// would lose the transfer it had already paid for.
#[tokio::test]
async fn committing_an_unfinished_transfer_does_not_destroy_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "premature").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let (early, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(early, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(
        open_sessions(&state).await,
        1,
        "asking too soon must not close the session"
    );
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    assert!(
        root.join(".waveflow-managed")
            .join(format!("{session}.part"))
            .is_file(),
        "nor throw away what had already arrived"
    );

    // And the transfer picks up exactly where it left off.
    let mut sent = 512usize;
    let mut index = 1usize;
    while sent < bytes.len() {
        let end = (sent + 512).min(bytes.len());
        let (status, _) = put_chunk(
            &config,
            &state,
            &fixture.token,
            &session,
            index,
            &bytes[sent..end],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        sent = end;
        index += 1;
    }
    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(committed["full_hash"], blake3_of(&bytes));
}

/// A session nobody owns leaves nothing behind in the process.
///
/// The per-session locks are created on lookup, so locking before resolving
/// would let any authorised caller mint one entry per uuid they can type — a
/// map that only ever grows, for sessions that never existed. Resolving first
/// bounds the entries to sessions that are real and theirs.
#[tokio::test]
async fn a_session_that_does_not_exist_leaves_nothing_behind() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "phantom").await;

    for _ in 0..16 {
        let stranger = uuid::Uuid::new_v4().to_string();
        let (status, _) =
            put_chunk(&config, &state, &fixture.token, &stranger, 0, &bytes[..512]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (committed, _) = commit(&config, &state, &fixture.token, &stranger).await;
        assert_eq!(committed, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        state.services.tracked_upload_locks(),
        0,
        "sessions that never existed must not be coordinated"
    );

    // A real one is tracked while it runs, and released when it ends.
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.services.tracked_upload_locks(), 1);

    let mut sent = 512usize;
    let mut index = 1usize;
    while sent < bytes.len() {
        let end = (sent + 512).min(bytes.len());
        let (status, _) = put_chunk(
            &config,
            &state,
            &fixture.token,
            &session,
            index,
            &bytes[sent..end],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        sent = end;
        index += 1;
    }
    let (status, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        state.services.tracked_upload_locks(),
        0,
        "and a finished session is not coordinated any more"
    );
}

/// Cleanup does not wait for somebody to offer another file.
///
/// The sweep otherwise only runs inside a negotiation. Nothing breaks — the
/// quota is decided by that same negotiation — but a client that abandons a
/// batch of transfers and never returns would leave its staging files holding
/// the operator's disk with nothing to signal it.
#[tokio::test]
async fn abandoned_transfers_are_swept_without_a_negotiation() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "abandoned").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staging.is_file(), "there is something to reclaim");

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    // No negotiation, no request of any kind: only the scheduled sweep.
    state.services.spawn_upload_sweeper();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while staging.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert!(
        !staging.exists(),
        "an abandoned transfer must not hold the disk until someone happens to upload again"
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A staging file that cannot be removed keeps its session.
///
/// File first, row second was chosen so a failure could be retried. A failure
/// that deletes the row anyway is not a retry — it is exactly the orphan that
/// order was avoiding, since the row is the only thing that remembers the file
/// exists. Here the staging path is made undeletable by turning it into a
/// non-empty directory, which `remove_file` refuses for a reason that is not
/// "already gone".
#[tokio::test]
async fn a_staging_file_that_cannot_be_removed_keeps_its_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    std::fs::remove_file(&staging).unwrap();
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("occupied"), b"x").unwrap();

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    // A negotiation sweeps first; the sweep must decline to forget this one.
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(777, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "the row is the only record that this file exists; dropping it orphans the file"
    );

    // And once the obstruction is gone, the next sweep finishes the job.
    std::fs::remove_dir_all(&staging).unwrap();
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(778, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0, "a retry is what the order was for");
}

/// The same rule when a commit gives up on the bytes it received.
#[tokio::test]
async fn a_failed_commit_keeps_a_session_whose_file_it_could_not_remove() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck-commit").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);

    // The transfer looks complete to the catalogue, and the file it points at
    // can be neither hashed nor removed.
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    std::fs::remove_file(&staging).unwrap();
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("occupied"), b"x").unwrap();

    let (status, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "a commit that could not clean up must leave the record that says so"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
}

/// A file stuck under its final name is reclaimed, and only when nobody else
/// could be using it.
///
/// A commit that renamed the file and then could neither move it back nor
/// remove it leaves one there. Nothing else looks under that name, so without
/// the sweep it would outlive the row that remembers it and be lost for good.
///
/// The guards are the point. That name is the hash, not the session's own
/// property: the unique index is per member, so two members can hold sessions
/// for the same hash in one library, and a successful commit by either produces
/// exactly that file.
#[tokio::test]
async fn a_file_stuck_under_its_final_name_is_reclaimed_unless_claimed() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck-final").await;
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let hash = blake3_of(&bytes);
    let relative = format!(".waveflow-managed/{hash}.wav");
    let stuck = root.join(&relative);

    // The state a commit leaves when it can neither move the file back nor
    // remove it: a session, and a file under the final name.
    let _session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    std::fs::create_dir_all(stuck.parent().unwrap()).unwrap();
    std::fs::write(&stuck, &bytes).unwrap();
    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    // A track claiming that path first: the file is in use, and the sweep must
    // not touch it.
    let scan_id = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = catalog_input(1, "Claiming Artist");
    input.relative_path = relative.clone();
    input.full_hash = hash.clone();
    state
        .db
        .apply_catalog_track(fixture.library, scan_id, &input, None, false)
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(555, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        stuck.is_file(),
        "a file a track is using must survive somebody else's abandoned session"
    );

    // Now the same state with nothing claiming it. The track goes first: while
    // it exists the negotiation answers `present`, which is the whole point of
    // the first half.
    sqlx::query("DELETE FROM track WHERE library_id=?")
        .bind(fixture.library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let _session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(556, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !stuck.exists(),
        "and one nobody is using must not outlive the row that remembers it"
    );
}

/// And spared while another member could still be committing it.
///
/// Between a commit's rename and its transaction there is no track yet, so the
/// track guard alone would let a stranger's expired session delete the file
/// that commit is about to catalogue. A live session on the same hash is what
/// says the name is not free.
#[tokio::test]
async fn a_file_another_session_may_be_committing_is_spared() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "contested-final").await;

    let password = security::generate_token("test-password-");
    let hashed = security::hash_password(&password).unwrap();
    let other = state
        .db
        .create_account("contesting-manager", &hashed, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            fixture.owner,
            fixture.library,
            other,
            LibraryRole::Manager,
            now_ms(),
        )
        .await
        .unwrap();
    let as_other = Fixture {
        owner: other,
        library: fixture.library,
        token: login_token(
            &waveflow_server::app(&config, state.clone()),
            "contesting-manager",
            &password,
        )
        .await,
    };

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let hash = blake3_of(&bytes);
    let stuck = root.join(format!(".waveflow-managed/{hash}.wav"));

    // Both sessions are opened before either expires. A negotiation sweeps
    // before it answers, so opening the second one afterwards would sweep the
    // first while nothing yet contested the name — and the test would pass for
    // a reason that has nothing to do with the guard.
    let _abandoned = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let live = open_session(&config, &state, &as_other, &bytes, "wav").await;
    std::fs::create_dir_all(stuck.parent().unwrap()).unwrap();
    std::fs::write(&stuck, &bytes).unwrap();
    sqlx::query("UPDATE upload_session SET expires_at=1 WHERE user_id=?")
        .bind(fixture.owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(444, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        stuck.is_file(),
        "an expired session must not delete the file another one is committing"
    );
    let survivors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&live)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(survivors, 1, "and the live session is untouched");
}

/// Not being able to tell is not the same as being told there is nothing.
///
/// The sweep asks whether a file is stuck under the final name before deciding
/// the session can go. Reading an I/O error from that question as "absent"
/// drops the row while the file it names may well be there — the orphan this
/// whole path exists to prevent, one question earlier.
///
/// A symlink pointing at itself is the deterministic way to make the question
/// fail: following it errors, while the staging file beside it is an ordinary
/// file the sweep would have removed without a second thought.
///
/// The branch under test is not POSIX-specific, but the way to reach it is:
/// creating a symlink on Windows needs a privilege the runner does not have.
/// Gated rather than made portable — a test that has to be told which error to
/// expect proves less than one that provokes a real one.
#[cfg(unix)]
#[tokio::test]
async fn a_file_the_sweep_cannot_ask_about_keeps_its_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "unaskable").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(sent, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let final_name = root.join(format!(".waveflow-managed/{}.wav", blake3_of(&bytes)));
    std::os::unix::fs::symlink(&final_name, &final_name).unwrap();
    assert!(
        std::fs::metadata(&final_name).is_err(),
        "the point of this test is a path that cannot be resolved"
    );

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(333, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "a question that could not be answered must not be answered as 'nothing there'"
    );
    assert!(
        root.join(format!(".waveflow-managed/{session}.part"))
            .is_file(),
        "and the transfer it had already received is still there"
    );
}

/// A client can tell its own upload from a discovery.
///
/// Without the origin device the feed says only "this track changed", so the
/// client that sent the file reads it back as one it has just found and treats
/// it as a discovery. `sync_event` has carried the same fact since its first
/// day, for the same reason.
#[tokio::test]
async fn a_client_can_tell_its_own_upload_from_a_discovery() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let password = security::generate_token("test-password-");
    let hashed = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("attributed", &hashed, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("attributed");
    std::fs::create_dir_all(&music).unwrap();
    // One track the scan finds by itself, so the feed holds both kinds.
    write_test_wav(&music.join("Found by the scan.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Attributed",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_uploads(owner, library, true, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let (token, device) = login_session(&router, "attributed", &password).await;
    run_scan(
        &state,
        owner,
        waveflow_server::catalog::LibraryRecord {
            id: library,
            name: "Attributed".into(),
            root_path: root,
        },
    )
    .await;

    let fixture = Fixture {
        owner,
        library,
        token: token.clone(),
    };
    let source = config.data_dir.join("attributed-source.wav");
    write_test_wav_of_len(&source, 900);
    let bytes = std::fs::read(&source).unwrap();
    std::fs::remove_file(&source).unwrap();
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);

    // A device belonging to somebody else is refused outright: an unchecked
    // header would let one account attribute its writes to another account's
    // device, and every client filtering its own changes out of the feed would
    // then drop somebody else's.
    let stranger_password = security::generate_token("test-password-");
    let stranger_hash = security::hash_password(&stranger_password).unwrap();
    state
        .db
        .create_account(
            "attributed-stranger",
            &stranger_hash,
            AccountRole::User,
            now_ms(),
        )
        .await
        .unwrap();
    let (_, stranger_device) =
        login_session(&router, "attributed-stranger", &stranger_password).await;
    let borrowed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/uploads/{session}/commit"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &stranger_device)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(borrowed.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let committed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/uploads/{session}/commit"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &device)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let uploaded = json_body(committed).await["track_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let feed = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v2/libraries/{library}/events?after=0&limit=100"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    let events = json_body(feed).await;
    let events = events["events"].as_array().unwrap();

    let mine = events
        .iter()
        .find(|event| event["entity_id"] == uploaded.as_str())
        .expect("the upload is on the feed");
    assert_eq!(
        mine["origin_device_id"], device,
        "the client that sent the file is named"
    );
    let scanned: Vec<_> = events
        .iter()
        .filter(|event| event["entity_id"] != uploaded.as_str())
        .collect();
    assert!(!scanned.is_empty(), "the scan wrote events too");
    for event in scanned {
        assert!(
            event["origin_device_id"].is_null(),
            "a scan is nobody's request in particular: {event}"
        );
    }
}
