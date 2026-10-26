//! FFmpeg and the transcoder: its absence, its ceilings, AAC, and the
//! compression left off streams.

use super::*;

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
    // What a client may ask `format=` for, so it can offer AAC without
    // guessing from a version number (RFC-022 decision 5).
    assert_eq!(status["formats"], serde_json::json!(["mp3", "opus", "aac"]));
    // The pair is only useful if it bounds anything, and the config refuses the
    // other order at startup.
    assert!(
        config.transcode_per_user_limit <= config.transcode_global_limit,
        "one account may not outrun the whole server"
    );
}

/// AAC is the third transcode, written as ADTS (RFC-022 decision 5).
///
/// The live answer is read the way a player reads it: its type, and the ADTS
/// sync word at the first byte — a container with a header in front, an MP4
/// for instance, would not start with one. The committed cache file is then
/// asked of `ffprobe` rather than trusted, and served by byte range, which is
/// the whole reason ADTS was chosen over a fragmented MP4 written to a pipe.
#[tokio::test]
async fn an_aac_transcode_streams_as_adts_and_seeks_once_cached() {
    let (_temp, config, state) = test_app().await;
    // Drawn at run time, never written down: a literal reaching the hash is
    // what CodeQL reports as a hard-coded credential.
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("aac-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("aac-music");
    std::fs::create_dir_all(&music).unwrap();
    // Three seconds, so the encode is many frames rather than one.
    write_test_wav_of_len(&music.join("Aac.wav"), 8_000 * 3);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "AAC library",
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
            name: "AAC library".into(),
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
    let token = login_token(&router, "aac-owner", &password).await;
    let uri = format!("/api/v2/tracks/{}/stream?format=aac&bitrate=64", track.id);
    let get = |range: Option<&'static str>| {
        let router = router.clone();
        let uri = uri.clone();
        let token = token.clone();
        async move {
            let mut request = Request::get(&uri).header("authorization", format!("Bearer {token}"));
            if let Some(range) = range {
                request = request.header("range", range);
            }
            router
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    let live = get(None).await;
    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(live.headers()["content-type"], "audio/aac");
    assert_eq!(live.headers()["accept-ranges"], "none");
    let bytes = live.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() > 100, "{} bytes", bytes.len());
    assert!(
        bytes[0] == 0xFF && bytes[1] & 0xF0 == 0xF0,
        "no ADTS sync word at the first byte: {:02x} {:02x}",
        bytes[0],
        bytes[1]
    );

    let cached = wait_for_cache_file(&config.transcode_cache_dir, "aac").await;
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=format_name:stream=codec_name",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&cached)
        .output()
        .expect("ffprobe must be on PATH");
    let probe = String::from_utf8_lossy(&probe.stdout);
    assert!(probe.contains("codec_name=aac"), "{probe}");
    assert!(probe.contains("format_name=aac"), "{probe}");

    let ranged = get(Some("bytes=0-31")).await;
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged.headers()["content-type"], "audio/aac");
    assert_eq!(
        ranged.into_body().collect().await.unwrap().to_bytes().len(),
        32
    );

    // The native API still refuses a format it cannot produce: its clients
    // read `formats` from the status route, and only the Subsonic façade falls
    // back (RFC-022 decision 6).
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{}/stream?format=flac", track.id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
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
