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
