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
    // The file has a length, so the refusal states which range would have been
    // satisfiable — and states the real one, read from the fixture rather than
    // written down, so the assertion cannot drift away from the file.
    let length = std::fs::metadata(music.join("Range.wav")).unwrap().len();
    assert_eq!(
        unsatisfiable
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok()),
        Some(format!("bytes */{length}").as_str())
    );
    // And it still accepts ranges: only the range asked for was wrong. Saying
    // `none` here would contradict every other answer this file gives.
    assert_eq!(
        unsatisfiable
            .headers()
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok()),
        Some("bytes")
    );

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
    // Nothing has been produced yet, so there is no complete length to give.
    // The unsatisfied-range form has nowhere to put "unknown", and `bytes */0`
    // is not unknown — it is a claim that the resource is empty, which a client
    // is entitled to believe and never ask again. The header is omitted.
    assert!(
        cold_seek.headers().get("content-range").is_none(),
        "a refusal with no known length must not state one: {:?}",
        cold_seek.headers().get("content-range")
    );
    // This one genuinely takes no range until the transcode is whole, which is
    // the other half of the same distinction.
    assert_eq!(
        cold_seek
            .headers()
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok()),
        Some("none")
    );

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

/// A server whose transcode ceilings the test chose, one long track, and its
/// owner: the setting of #185, which is about what a seek leaves behind.
///
/// Driven through `MediaService` rather than the router. What is under test is
/// the cache and the slots, and nobody has to sign in to reach them.
struct Seekable {
    _temp: tempfile::TempDir,
    state: waveflow_server::AppState,
    owner: uuid::Uuid,
    track: uuid::Uuid,
}

impl Seekable {
    async fn new(global: usize, per_user: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::for_data_dir(temp.path().join("data"));
        // Before `initialize`, which copies the ceilings into the service.
        config.transcode_global_limit = global;
        config.transcode_per_user_limit = per_user;
        let state = waveflow_server::initialize(&config).await.unwrap();
        // Nobody signs in, so the account only has to exist for the library to
        // belong to it.
        let owner = state
            .db
            .create_account("seek-owner", "never-signs-in", AccountRole::Admin, now_ms())
            .await
            .unwrap();
        let music = config.data_dir.join("seek-music");
        std::fs::create_dir_all(&music).unwrap();
        // Three minutes: far more MP3 than the stream's channel and FFmpeg's pipe
        // hold together, so a play whose consumer reads one chunk and leaves is
        // still encoding when it is abandoned.
        write_test_wav_of_len(&music.join("Long.wav"), 8_000 * 180);
        let root = std::fs::canonicalize(&music).unwrap();
        let library = state
            .db
            .create_library(
                owner,
                "Seek library",
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
                id: library,
                name: "Seek library".into(),
                root_path: root,
            },
        )
        .await;
        let track = state
            .db
            .list_tracks_for_user(owner, library)
            .await
            .unwrap()
            .pop()
            .unwrap()
            .id;
        Self {
            _temp: temp,
            state,
            owner,
            track,
        }
    }

    /// An MP3 stream of the track, from `offset_ms`.
    async fn stream(
        &self,
        offset_ms: u64,
        range: Option<&str>,
    ) -> Result<axum::response::Response, waveflow_server::media::MediaError> {
        let track = self
            .state
            .db
            .stream_track_for_user(self.owner, self.track)
            .await
            .unwrap()
            .unwrap();
        self.state
            .media
            .serve(
                self.owner,
                track,
                waveflow_server::media::StreamQuery {
                    format: waveflow_server::media::OutputFormat::Mp3,
                    bitrate: Some(96),
                    offset_ms,
                },
                range,
            )
            .await
    }

    /// Whether the whole transcode is in the cache, learned the way a client
    /// learns it: a range that seeks is refused by a live transcode and
    /// answered by a cached file.
    async fn cached(&self) -> bool {
        matches!(
            self.stream(0, Some("bytes=64-127")).await,
            Ok(response) if response.status() == StatusCode::PARTIAL_CONTENT
        )
    }

    /// Waits until no transcode runs, a cache fill included.
    async fn settle(&self) {
        for _ in 0..3_000 {
            if self.state.media.active_transcodes() == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("a transcode was still running after thirty seconds");
    }
}

/// Issue #185, as a client lives it. The first play starts at the top and is
/// dropped the moment the listener seeks, and the seek goes through
/// `offset_ms`. Neither is a whole play, and yet the next play has to be a
/// cached file a client can seek in by byte range — otherwise every play of the
/// track is a live transcode seeked the same way, and it never leaves that loop.
#[tokio::test]
async fn a_seek_on_the_first_play_leaves_the_next_play_cached() {
    let seekable = Seekable::new(4, 2).await;

    let first = seekable.stream(0, None).await.unwrap();
    let mut body = first.into_body();
    body.frame().await.unwrap().unwrap();
    drop(body);
    seekable.settle().await;
    // The precondition, checked rather than assumed: the abandoned play was
    // killed and left nothing. Had it run to its end, the cache below would say
    // nothing about seeking.
    assert!(
        !seekable.cached().await,
        "the abandoned first play must not have filled the cache itself"
    );

    let seek = seekable.stream(90_000, None).await.unwrap();
    assert_eq!(seek.status(), StatusCode::OK);
    seek.into_body().collect().await.unwrap();
    seekable.settle().await;

    assert!(
        seekable.cached().await,
        "the play after a seek must be a cached file"
    );
}

/// The fill never takes the last slot a live request could want. With two
/// slots on the server and the seek holding one, leaving one free means not
/// starting at all — and nothing is cached however long one waits.
#[tokio::test]
async fn a_seek_fills_the_cache_only_with_a_slot_to_spare() {
    let seekable = Seekable::new(2, 1).await;

    let seek = seekable.stream(90_000, None).await.unwrap();
    // Read at once: a fill is counted before the seek's answer is returned, so
    // one that started would already be here.
    assert_eq!(
        seekable.state.media.active_transcodes(),
        1,
        "the seek, and no fill beside it"
    );
    seek.into_body().collect().await.unwrap();
    seekable.settle().await;
    assert!(!seekable.cached().await);
}

/// The fill does not count against the account's own limit. That limit is what
/// the seek itself holds, and a listener allowed one stream must not lose the
/// cache for having used it.
#[tokio::test]
async fn a_seek_fills_the_cache_outside_the_accounts_own_limit() {
    let seekable = Seekable::new(3, 1).await;

    let seek = seekable.stream(90_000, None).await.unwrap();
    assert_eq!(
        seekable.state.media.active_transcodes(),
        2,
        "the seek, and the fill behind it"
    );
    seek.into_body().collect().await.unwrap();
    seekable.settle().await;
    assert!(seekable.cached().await);
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

    // Playing sends no attachment header; that is a download's alone.
    assert!(played.headers().get("content-disposition").is_none());
    let played_length = played.headers()["content-length"].clone();

    // `?download=1` is the same file, saved under the file's own name
    // (RFC-012 decision 5) — no other route, no other read.
    let basename = tracks[0]["relative_path"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    let downloaded = router
        .clone()
        .oneshot(
            Request::get(format!("{url}?download=1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(downloaded.headers()["content-length"], played_length);
    let disposition = downloaded.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.starts_with("attachment; filename=\"")
            && disposition.contains(&format!("filename*=UTF-8''{basename}")),
        "{disposition}"
    );

    // A download is the file: asked for with a transcode, an offset or a bit
    // rate, it is refused rather than handed over under the file's name.
    for refused in [
        format!("{url}?download=1&format=mp3"),
        format!("{url}?download=1&offset_ms=1000"),
        format!("{url}?download=1&bitrate=128"),
    ] {
        let response = router
            .clone()
            .oneshot(Request::get(&refused).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{refused}"
        );
    }

    // An expired ticket is refused even though it is otherwise well formed.
    let expired = waveflow_server::stream_ticket::mint(
        &secret_box,
        waveflow_server::stream_ticket::TicketKind::Audio,
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

    // A ticket of another kind does not open audio, even minted for this very
    // track by this very account under the instance key, and unexpired. The
    // kind is the only thing separating it from the ticket that works two
    // assertions above — which is the whole reason the byte exists.
    let wrong_kind = waveflow_server::stream_ticket::mint(
        &secret_box,
        waveflow_server::stream_ticket::TicketKind::Canvas,
        owner,
        uuid::Uuid::parse_str(&track_id).unwrap(),
        now_ms() + 60_000,
    )
    .unwrap();
    let wrong_kind = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{wrong_kind}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_kind.status(), StatusCode::NOT_FOUND);

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

/// A client cannot size its own concurrency against a ceiling it is not told.
///
/// `active` says how busy the server is; without the two limits it enforces, a
/// caller can only find the line by crossing it — and a `429` costs a request
/// while saying nothing about how far over it went.
#[tokio::test]
async fn transcode_status_reports_the_ceilings_it_enforces() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    state
        .db
        .create_account("headroom", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let token = login_token(&router, "headroom", password).await;

    let response = router
        .oneshot(
            Request::get("/api/v2/transcode/status")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = json_body(response).await;
    assert_eq!(status["active"], 0);
    assert_eq!(status["global_limit"], config.transcode_global_limit);
    assert_eq!(status["per_user_limit"], config.transcode_per_user_limit);
    // The pair is only useful if it bounds anything, and the config refuses the
    // other order at startup.
    assert!(
        config.transcode_per_user_limit <= config.transcode_global_limit,
        "one account may not outrun the whole server"
    );
}

/// The client is compressed; the stream is not, and still accepts ranges.
///
/// Both halves in one test because either alone is reassuring and neither is
/// evidence. A layer applied to the whole router would pass the first; a layer
/// applied nowhere would pass the second.
///
/// The fixture is a **WAV**, chosen rather than tolerated: it is uncompressed
/// audio, so it is precisely the body an encoder would squeeze happily — and
/// precisely the body a player walks through by byte offset. An `image/` or
/// `text/event-stream` probe would pass whether or not the media routes sit
/// outside the layer, because tower-http's default predicate already leaves
/// those alone. Audio is the only content type here that the predicate would
/// have compressed, so it is the only one that can tell the two arrangements
/// apart.
#[tokio::test]
async fn the_client_is_compressed_and_the_stream_is_left_alone() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("compress-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("compress-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Plain.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Compression library",
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
            name: "Compression library".into(),
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

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "compress-owner", password).await;

    // The web client, which is what the layer is for.
    let shell = router
        .clone()
        .oneshot(
            Request::get("/")
                .header("accept-encoding", "gzip, br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(shell.status(), StatusCode::OK);
    assert!(
        shell.headers().contains_key("content-encoding"),
        "the shell arrived uncompressed: {:?}",
        shell.headers()
    );

    // And the stream, which it must never reach. Asked without a range, so the
    // answer is a whole 200 body: `Content-Range` is the one thing tower-http
    // refuses to compress on its own, so a 206 would pass this assertion even
    // with the layer wrapped around everything.
    let stream = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{}/stream", track.id))
                .header("authorization", format!("Bearer {token}"))
                .header("accept-encoding", "gzip, br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert!(
        !stream.headers().contains_key("content-encoding"),
        "the stream was re-encoded: {:?}",
        stream.headers()
    );
    // Re-encoding would have taken these two with it, and they are what a
    // player seeks on.
    assert_eq!(
        stream
            .headers()
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok()),
        Some("bytes")
    );
    let declared: u64 = stream.headers()["content-length"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let on_disk = std::fs::metadata(music.join("Plain.wav")).unwrap().len();
    assert_eq!(
        declared, on_disk,
        "the declared length no longer describes the file a client would seek through"
    );
    let bytes = stream.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"RIFF", "the body is no longer a WAV");
    assert_eq!(bytes.len() as u64, on_disk);
}

/// One real file, scanned, and a share of it created through the native API.
///
/// The temporary directory comes back with the rest because the library lives
/// in it: dropping it would delete the file the share streams.
struct SharedFile {
    _temp: tempfile::TempDir,
    router: axum::Router,
    token: String,
    track: uuid::Uuid,
    share_id: String,
    share_url: String,
}

async fn shared_file(name: &str, share: serde_json::Value) -> SharedFile {
    let (temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account(name, &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(format!("{name}-music"));
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Shared.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .pop()
        .unwrap()
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, name, &password).await;
    let mut body = share;
    body["track_ids"] = serde_json::json!([track]);
    let created = send_json(&router, "POST", "/api/v2/shares", &token, Some(body)).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    SharedFile {
        _temp: temp,
        router,
        token,
        track,
        share_id: created["id"].as_str().unwrap().to_owned(),
        share_url: created["url"].as_str().unwrap().to_owned(),
    }
}

async fn anonymous(
    router: &axum::Router,
    uri: &str,
    range: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::get(uri);
    if let Some(range) = range {
        request = request.header("range", range);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Opens the share as its JSON consumers do, and answers the body.
async fn open_share(router: &axum::Router, url: &str) -> serde_json::Value {
    let opened = anonymous(router, url, None).await;
    assert_eq!(opened.status(), StatusCode::OK);
    json_body(opened).await
}

fn with_query(url: &str, pair: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{pair}")
}

async fn listed_share(shared: &SharedFile) -> serde_json::Value {
    let listed =
        json_body(send_json(&shared.router, "GET", "/api/v2/shares", &shared.token, None).await)
            .await;
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["id"] == shared.share_id.as_str())
        .unwrap()
        .clone()
}

/// A visit is an opening of the share, not a request for its bytes (RFC-020
/// decision 4).
///
/// Every stream request used to count one, under the process-wide writer gate:
/// a single listen with a few seeks — each one a range request — climbed the
/// counter by as many, and every one of them queued behind a scan.
#[tokio::test]
async fn a_share_counts_one_visit_per_opening_and_none_per_stream_request() {
    let shared = shared_file("visit-owner", serde_json::json!({})).await;
    let opened = open_share(&shared.router, &shared.share_url).await;
    assert_eq!(opened["visitCount"], 1);
    let stream = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    for range in ["bytes=0-3", "bytes=4-7", "bytes=0-"] {
        let played = anonymous(&shared.router, &stream, Some(range)).await;
        assert_eq!(played.status(), StatusCode::PARTIAL_CONTENT, "{range}");
    }
    assert_eq!(listed_share(&shared).await["visit_count"], 1);

    open_share(&shared.router, &shared.share_url).await;
    assert_eq!(listed_share(&shared).await["visit_count"], 2);

    // The stream address is the one the opening handed out. Built by hand,
    // without the visit it belongs to, it opens nothing.
    let bare = format!("{}/tracks/{}/stream", shared.share_url, shared.track);
    assert_eq!(
        anonymous(&shared.router, &bare, None).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// An anonymous visitor plays the original or one fixed MP3, and chooses
/// nothing else (RFC-020 decision 5).
///
/// The stream route took the whole stream query, so anyone holding the link
/// could ask for any format at any bitrate, or for a live encode from any
/// offset, each one an FFmpeg process run on the owner's account.
#[tokio::test]
async fn an_anonymous_share_visitor_cannot_choose_a_transcode() {
    let shared = shared_file("transcode-owner", serde_json::json!({})).await;
    let opened = open_share(&shared.router, &shared.share_url).await;
    let stream = opened["tracks"][0]["streamUrl"]
        .as_str()
        .unwrap()
        .to_owned();
    for refused in [
        "format=opus",
        "format=mp3&bitrate=96",
        "format=mp3&offset_ms=50",
        "bitrate=320",
    ] {
        assert_eq!(
            anonymous(&shared.router, &with_query(&stream, refused), None)
                .await
                .status(),
            StatusCode::BAD_REQUEST,
            "{refused}"
        );
    }
    let mp3 = anonymous(&shared.router, &with_query(&stream, "format=mp3"), None).await;
    assert_eq!(mp3.status(), StatusCode::OK);
    assert_eq!(mp3.headers()["content-type"], "audio/mpeg");
    let original = anonymous(&shared.router, &stream, None).await;
    assert_eq!(original.status(), StatusCode::OK);
    let bytes = original.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"RIFF");
}

/// A share may be opened a set number of times, and the opening that reached
/// the cap still plays (RFC-020 decision 6).
#[tokio::test]
async fn a_share_stops_opening_at_its_visit_cap_and_its_last_visit_still_plays() {
    let shared = shared_file("cap-owner", serde_json::json!({ "max_visits": 2 })).await;
    assert_eq!(listed_share(&shared).await["max_visits"], 2);
    open_share(&shared.router, &shared.share_url).await;
    let last = open_share(&shared.router, &shared.share_url).await;
    let stream = last["tracks"][0]["streamUrl"].as_str().unwrap().to_owned();
    assert_eq!(
        anonymous(&shared.router, &shared.share_url, None)
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "the third opening of a share capped at two"
    );
    assert_eq!(listed_share(&shared).await["visit_count"], 2);
    assert_eq!(
        anonymous(&shared.router, &stream, Some("bytes=0-3"))
            .await
            .status(),
        StatusCode::PARTIAL_CONTENT,
        "the visit that reached the cap keeps its music"
    );

    // A cap is a positive number, and lifting it is a named clear.
    let uri = format!("/api/v2/shares/{}", shared.share_id);
    for invalid in [0, -3] {
        assert_eq!(
            send_json(
                &shared.router,
                "PATCH",
                &uri,
                &shared.token,
                Some(serde_json::json!({ "max_visits": invalid })),
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{invalid}"
        );
    }
    let lifted = send_json(
        &shared.router,
        "PATCH",
        &uri,
        &shared.token,
        Some(serde_json::json!({ "clear": ["max_visits"] })),
    )
    .await;
    assert_eq!(lifted.status(), StatusCode::OK);
    assert!(json_body(lifted).await["max_visits"].is_null());
    assert_eq!(
        open_share(&shared.router, &shared.share_url).await["visitCount"],
        3
    );

    // A visit's address is bound to its share: put on another share's token,
    // it opens nothing, even for the same track.
    let other = json_body(
        send_json(
            &shared.router,
            "POST",
            "/api/v2/shares",
            &shared.token,
            Some(serde_json::json!({ "track_ids": [shared.track] })),
        )
        .await,
    )
    .await;
    let other_url = other["url"].as_str().unwrap();
    let crossed = stream.replacen(&shared.share_url, other_url, 1);
    assert_ne!(crossed, stream);
    assert_eq!(
        anonymous(&shared.router, &crossed, None).await.status(),
        StatusCode::NOT_FOUND
    );

    // And a deleted share stops playing at once, whatever the visit carried.
    assert_eq!(
        send_json(&shared.router, "DELETE", &uri, &shared.token, None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous(&shared.router, &stream, None).await.status(),
        StatusCode::NOT_FOUND
    );
}
