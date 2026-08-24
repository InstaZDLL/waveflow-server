//! Streaming, transcoding and the tickets a browser plays from.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::Config;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

#[tokio::test]
async fn media_streaming_ranges_transcodes_caches_and_isolates_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("media-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("media-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("media-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Range.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Media library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Media library".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let media = state.media.clone();
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "media-owner", password).await;
    let intruder_token = login_token(&router, "media-intruder", password).await;
    let uri = format!("/api/v2/tracks/{}/stream", track.id);

    let response = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 0-9/1644");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(bytes.len(), 10);

    let unsatisfiable = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=99999-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsatisfiable.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let hidden = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {intruder_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let transcode_uri = format!("{uri}?format=mp3&bitrate=96");
    let response = router
        .clone()
        .oneshot(
            Request::get(&transcode_uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["accept-ranges"], "none");
    assert_eq!(response.headers()["content-type"], "audio/mpeg");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 100
    );

    let cache_file = wait_for_cache_file(&config.transcode_cache_dir, "mp3").await;
    assert!(cache_file.metadata().unwrap().len() > 100);
    let cached_range = router
        .clone()
        .oneshot(
            Request::get(&transcode_uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cached_range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        cached_range
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len(),
        32
    );

    // Two consumers of the same missing cache key converge on one FFmpeg job:
    // the second waits for the per-key guard, then reads the committed file.
    let concurrent_uri = format!("{uri}?format=mp3&bitrate=112");
    let first_router = router.clone();
    let second_router = router.clone();
    let first_token = owner_token.clone();
    let second_token = owner_token.clone();
    let first_uri = concurrent_uri.clone();
    // Read before the pair runs, so what follows can name the file this pair
    // created rather than count every one the test has ever produced.
    let cached_mp3 = |dir: &std::path::Path| {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some("mp3"))
            .collect::<std::collections::HashSet<_>>()
    };
    let before = cached_mp3(&config.transcode_cache_dir);
    let first = async move {
        let response = first_router
            .oneshot(
                Request::get(first_uri)
                    .header("authorization", format!("Bearer {first_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Checked before the body is read: two failures with empty bodies
        // would compare equal below and say nothing at all.
        assert_eq!(response.status(), StatusCode::OK);
        response.into_body().collect().await.unwrap().to_bytes()
    };
    let second = async move {
        let response = second_router
            .oneshot(
                Request::get(concurrent_uri)
                    .header("authorization", format!("Bearer {second_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.into_body().collect().await.unwrap().to_bytes()
    };
    let (first_bytes, second_bytes) = tokio::join!(first, second);
    assert!(
        first_bytes.len() > 1_000,
        "a transcode of a real track is not a handful of bytes: {}",
        first_bytes.len()
    );
    assert_eq!(first_bytes, second_bytes);
    let created = cached_mp3(&config.transcode_cache_dir)
        .difference(&before)
        .count();
    assert_eq!(
        created, 1,
        "duplicate consumers must create only one file for the cache key they share"
    );

    // A browser audio element opens every resource with `Range: bytes=0-`, so
    // a cold transcode must answer it rather than refuse the range. Refusing
    // made a web client fail on the first play of a track and succeed on the
    // second, once the cache existed.
    let cold_open = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=128"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_open.status(), StatusCode::OK);
    assert_eq!(cold_open.headers()["content-type"], "audio/mpeg");
    assert!(
        cold_open
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 100
    );

    // iOS probes a resource with a two-byte range before it plays anything, so
    // a bounded range from zero has to open the stream too. Juliet failed on
    // the first play of every track until it did.
    let cold_probe = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=136"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_probe.status(), StatusCode::OK);
    assert_eq!(cold_probe.headers()["content-type"], "audio/mpeg");
    // Drained and awaited: an unread transcode holds its per-user permit, and
    // the checks below would meet 429 instead of what they are testing.
    cold_probe.into_body().collect().await.unwrap();
    for _ in 0..100 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "a drained transcode must release its permit"
    );

    // A range that actually seeks still has no meaning before the transcode
    // exists, and keeps its refusal.
    let cold_seek = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=144"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=64-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_seek.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let live_seek = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=opus&bitrate=64&offsetMs=25"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live_seek.status(), StatusCode::OK);
    assert_eq!(live_seek.headers()["accept-ranges"], "none");
    drop(live_seek);
    for _ in 0..100 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "abandoned FFmpeg was not cancelled"
    );
    assert!(!std::fs::read_dir(&config.transcode_cache_dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".part-")));

    sqlx::query("UPDATE track SET relative_path = '../outside.wav' WHERE id = ?")
        .bind(track.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let escaped = router
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(escaped.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stream_tickets_authorise_browser_playback_without_a_bearer() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("ticket-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("ticket-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("ticket-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Ticket.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Tickets",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Tickets".into(),
            root_path: root,
        },
    )
    .await;

    // Kept before `state` moves into the router, to mint an expired ticket below.
    let secret_box = std::sync::Arc::clone(&state.secret_box);
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "ticket-owner", password).await;
    let intruder_token = login_token(&router, "ticket-intruder", password).await;

    let tracks = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tracks = json_body(tracks).await;
    let track_id = tracks[0]["id"].as_str().unwrap().to_owned();

    let mint = |token: Option<String>| {
        let router = router.clone();
        let track_id = track_id.clone();
        async move {
            let mut request = Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"));
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    // Minting requires a bearer; redeeming must not.
    assert_eq!(mint(None).await.status(), StatusCode::UNAUTHORIZED);

    let issued = mint(Some(owner_token.clone())).await;
    assert_eq!(issued.status(), StatusCode::OK);
    let issued = json_body(issued).await;
    let url = issued["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/v2/stream/"));
    assert!(issued["expires_at"].as_i64().unwrap() > now_ms());

    // The ticket URL stays relative even when a public URL is configured, and
    // that asymmetry with createShare is deliberate: a share link is made to
    // leave the application, a ticket is not. An absolute ticket would let the
    // server point playback at a host the user never authenticated against, so
    // clients are right to reject absolute or protocol-relative values — this
    // pins the guarantee they rely on.
    let mut public_config = config.clone();
    public_config.public_url = Some("https://waveflow.example".to_owned());
    let public_router = waveflow_server::app(&public_config, state.clone());
    let public_ticket = public_router
        .oneshot(
            Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_ticket.status(), StatusCode::OK);
    let public_url = json_body(public_ticket).await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        public_url.starts_with("/api/v2/stream/"),
        "ticket URL must stay relative, got {public_url}"
    );

    // The ticket URL plays with no Authorization header at all.
    let played = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(played.headers()["accept-ranges"], "bytes");

    // Range requests work, which is what a browser seek relies on.
    let ranged = router
        .clone()
        .oneshot(
            Request::get(&url)
                .header("range", "bytes=0-15")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);

    // An expired ticket is refused even though it is otherwise well formed.
    let expired = waveflow_server::stream_ticket::mint(
        &secret_box,
        owner,
        uuid::Uuid::parse_str(&track_id).unwrap(),
        now_ms() - 1,
    )
    .unwrap();
    let expired = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{expired}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::NOT_FOUND);

    // A tampered ticket is indistinguishable from an unknown track. The flipped
    // character sits mid-ticket: trailing base64url characters carry spare bits,
    // so changing one there can decode to the same bytes.
    let prefix_len = "/api/v2/stream/".len();
    let mut tampered: Vec<char> = url.chars().collect();
    let middle = prefix_len + (tampered.len() - prefix_len) / 2;
    tampered[middle] = if tampered[middle] == 'A' { 'B' } else { 'A' };
    let tampered: String = tampered.into_iter().collect();
    let forged = router
        .clone()
        .oneshot(Request::get(&tampered).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::NOT_FOUND);

    // A tenant without access cannot mint a ticket in the first place.
    assert_eq!(
        mint(Some(intruder_token)).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn startup_reports_missing_ffmpeg() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::for_data_dir(temp.path().join("data"));
    config.ffmpeg_path = temp.path().join("missing-ffmpeg");
    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("initialization unexpectedly accepted a missing FFmpeg"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("ffmpeg is required"));
}
