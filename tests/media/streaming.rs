//! Streaming: ranges, transcodes, the cache and tenancy, and what a seek leaves
//! cached.

use super::*;

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
        Self::with_cache_budget(global, per_user, None).await
    }

    /// [`Self::new`], with the transcode cache held to `budget` bytes.
    async fn with_cache_budget(global: usize, per_user: usize, budget: Option<u64>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::for_data_dir(temp.path().join("data"));
        // Before `initialize`, which copies the ceilings into the service.
        config.transcode_global_limit = global;
        config.transcode_per_user_limit = per_user;
        if let Some(budget) = budget {
            config.transcode_cache_max_bytes = budget;
        }
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

/// A seek probed by a bounded range is answered with that range, out of an
/// encode of the seek kept under a key of its own.
///
/// iOS opens every resource with `bytes=0-1`, a seek included, and plays
/// nothing until the answer says how long the whole is. A live transcode has no
/// length, so the probe used to get the whole stream as a 200, and Juliet asked
/// three times and crashed (the 2026-10-27 campaign, Opus with `timeOffset`).
/// The size is checked against the encode's rate rather than trusted: MP3 at
/// 96 kbps is 12 000 bytes a second, so the 90 seconds left after the offset
/// fall well inside the band and the whole track's 180 would not.
#[tokio::test]
async fn a_seek_probed_by_a_bounded_range_is_answered_from_its_own_encode() {
    let seekable = Seekable::new(4, 2).await;

    let probe = seekable.stream(90_000, Some("bytes=0-1")).await.unwrap();
    assert_eq!(probe.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(probe.headers()["accept-ranges"], "bytes");
    let content_range = probe.headers()["content-range"]
        .to_str()
        .unwrap()
        .to_owned();
    let total: u64 = content_range
        .strip_prefix("bytes 0-1/")
        .unwrap_or_else(|| panic!("not a two-byte answer: {content_range}"))
        .parse()
        .unwrap();
    assert!(
        (12_000 * 60..12_000 * 120).contains(&total),
        "{total} bytes is not the 90 seconds after the offset"
    );
    assert_eq!(
        probe.into_body().collect().await.unwrap().to_bytes().len(),
        2
    );

    // What the player asks next: a range further in, answered from the same
    // file, where a live transcode would refuse it.
    let next = seekable.stream(90_000, Some("bytes=64-127")).await.unwrap();
    assert_eq!(next.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        next.headers()["content-range"],
        format!("bytes 64-127/{total}").as_str()
    );
    seekable.settle().await;

    // A seek without a range is still a live stream: the listener hears it at
    // once rather than after the encode.
    let live = seekable.stream(30_000, None).await.unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(live.headers()["accept-ranges"], "none");
    live.into_body().collect().await.unwrap();
    seekable.settle().await;
}

/// A seek larger than the whole cache budget is still answered.
///
/// Committing it prunes the cache, and the file just committed is then the
/// only one over the budget: pruned before it was opened, the probe would be a
/// 404 for an encode that succeeded. It is opened before the pruning pass, so
/// the answer reads it whatever the pass does with its name.
#[tokio::test]
async fn a_seek_larger_than_the_cache_is_answered_before_it_is_pruned() {
    let seekable = Seekable::with_cache_budget(4, 2, Some(1)).await;

    let probe = seekable.stream(90_000, Some("bytes=0-1")).await.unwrap();
    assert_eq!(probe.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        probe.into_body().collect().await.unwrap().to_bytes().len(),
        2
    );
    seekable.settle().await;
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
