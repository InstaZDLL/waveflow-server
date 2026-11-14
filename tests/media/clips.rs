//! A track's music video over HTTP (RFC-029 decisions 5 to 8): what a track
//! says about it, the three routes that serve and ticket it, and who may see it.

use super::*;

/// A moving picture with a soundtrack, as a music video is.
fn write_video_mp4(path: &std::path::Path) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=blue:s=64x64:d=0.2"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=0.2"])
        .args(["-shortest", "-c:v", "mpeg4", "-c:a", "aac"])
        .arg(path)
        .output()
        .expect("FFmpeg is required for the music video fixtures");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    state: waveflow_server::AppState,
    router: axum::Router,
    owner: uuid::Uuid,
    token: String,
    library_id: uuid::Uuid,
    /// The track with a clip beside it, and the one without.
    song: String,
    plain: String,
    clip: std::path::PathBuf,
}

async fn fixture(name: &str) -> Fixture {
    let (temp, config, state) = test_app().await;
    let (owner, token) = admin_with_api_token(&state, name).await;
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(music.join("Album")).unwrap();
    write_tagged_flac(&music.join("Album/Song.flac"), "Song", "Album", 440);
    write_tagged_flac(&music.join("Album/Plain.flac"), "Plain", "Album", 550);
    let clip = music.join("Album/Song.mp4");
    write_video_mp4(&clip);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    scan_once(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());
    let id = |path: &'static str| {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT id FROM track WHERE relative_path = ?")
                .bind(path)
                .fetch_one(state.db.pool())
                .await
                .unwrap()
        }
    };
    let song = id("Album/Song.flac").await;
    let plain = id("Album/Plain.flac").await;
    Fixture {
        _temp: temp,
        state,
        router,
        owner,
        token,
        library_id,
        song,
        plain,
        clip,
    }
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    range: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(bearer) = bearer {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    if let Some(range) = range {
        request = request.header("range", range);
    }
    router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Mints a ticket of the route `kind` names (`clip-ticket`, `stream-ticket`)
/// and returns its URL.
async fn ticket(fixture: &Fixture, track: &str, kind: &str) -> String {
    let response = send(
        &fixture.router,
        "POST",
        &format!("/api/v2/tracks/{track}/{kind}"),
        Some(&fixture.token),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "{kind}");
    json_body(response).await["url"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Decision 8: a track says whether it has a clip, its format and size, and
/// nothing that names the file.
#[tokio::test]
async fn a_track_says_whether_it_has_a_music_video() {
    let fixture = fixture("clip-field").await;
    let size = std::fs::metadata(&fixture.clip).unwrap().len() as i64;
    let tracks = json_body(
        send(
            &fixture.router,
            "GET",
            &format!("/api/v2/songs?library_id={}", fixture.library_id),
            Some(&fixture.token),
            None,
        )
        .await,
    )
    .await;
    let clip_of = |id: &str| {
        tracks
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == id)
            .map(|track| track["clip"].clone())
            .unwrap()
    };
    assert_eq!(
        clip_of(&fixture.song),
        serde_json::json!({ "format": "mp4", "byte_size": size })
    );
    assert_eq!(clip_of(&fixture.plain), serde_json::Value::Null);

    let detail = json_body(
        send(
            &fixture.router,
            "GET",
            &format!("/api/v2/tracks/{}", fixture.song),
            Some(&fixture.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["clip"]["format"], "mp4");
    assert!(
        !detail.to_string().contains("Song.mp4"),
        "a clip's path is never sent"
    );
}

/// Decisions 5 and 6: the clip is served whole or by range, as a video, to a
/// bearer and to a ticket, and the ticket's URL is relative.
#[tokio::test]
async fn a_music_video_is_served_whole_or_by_range() {
    let fixture = fixture("clip-serve").await;
    let bytes = std::fs::read(&fixture.clip).unwrap();
    let alias = format!("/api/v2/tracks/{}/clip", fixture.song);

    assert_eq!(
        send(&fixture.router, "GET", &alias, None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let whole = send(&fixture.router, "GET", &alias, Some(&fixture.token), None).await;
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(whole.headers()["content-type"], "video/mp4");
    assert_eq!(whole.headers()["accept-ranges"], "bytes");
    assert_eq!(whole.headers()["cache-control"], "private, no-store");
    assert_eq!(whole.into_body().collect().await.unwrap().to_bytes(), bytes);

    let url = ticket(&fixture, &fixture.song, "clip-ticket").await;
    assert!(url.starts_with("/api/v2/clip-stream/"), "{url}");
    let ranged = send(&fixture.router, "GET", &url, None, Some("bytes=4-11")).await;
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        ranged.headers()["content-range"],
        format!("bytes 4-11/{}", bytes.len())
    );
    assert_eq!(
        ranged.into_body().collect().await.unwrap().to_bytes(),
        bytes[4..12]
    );
}

/// Decision 7: every refusal answers the same 404 — a track with no clip, a
/// track that does not exist, a clip gone from the disk since the scan, and a
/// ticket of another kind, in both directions.
#[tokio::test]
async fn every_refusal_is_the_same_404() {
    let fixture = fixture("clip-refusals").await;
    let not_found = |uri: String, bearer: bool| {
        let router = fixture.router.clone();
        let token = fixture.token.clone();
        async move {
            let method = if uri.ends_with("-ticket") {
                "POST"
            } else {
                "GET"
            };
            let bearer = bearer.then_some(token);
            let status = send(&router, method, &uri, bearer.as_deref(), None)
                .await
                .status();
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    };
    let plain = &fixture.plain;
    let nobody = uuid::Uuid::new_v4();
    not_found(format!("/api/v2/tracks/{plain}/clip"), true).await;
    not_found(format!("/api/v2/tracks/{plain}/clip-ticket"), true).await;
    not_found(format!("/api/v2/tracks/{nobody}/clip"), true).await;
    not_found(format!("/api/v2/tracks/{nobody}/clip-ticket"), true).await;
    not_found("/api/v2/clip-stream/forged".to_owned(), false).await;

    // An audio ticket opens no clip, and a clip ticket opens neither the audio
    // nor a canvas: the kind is sealed, and every route asserts its own.
    let audio = ticket(&fixture, &fixture.song, "stream-ticket").await;
    let audio = audio.trim_start_matches("/api/v2/stream/");
    not_found(format!("/api/v2/clip-stream/{audio}"), false).await;
    let clip = ticket(&fixture, &fixture.song, "clip-ticket").await;
    let clip = clip.trim_start_matches("/api/v2/clip-stream/");
    not_found(format!("/api/v2/stream/{clip}"), false).await;
    not_found(format!("/api/v2/canvas-stream/{clip}"), false).await;

    // Resolved at every request: a clip deleted since the scan is a 404, the
    // ticket minted before it a 404 too, and no new ticket is minted for it,
    // so a client renewing after a 404 learns the clip is gone.
    std::fs::remove_file(&fixture.clip).unwrap();
    not_found(format!("/api/v2/tracks/{}/clip", fixture.song), true).await;
    not_found(format!("/api/v2/clip-stream/{clip}"), false).await;
    not_found(format!("/api/v2/tracks/{}/clip-ticket", fixture.song), true).await;
}

/// Decision 7: whoever may play the track may watch its clip, re-checked at
/// every redemption, so a stranger never sees it and a disabled account loses
/// it at once rather than at the ticket's expiry.
#[tokio::test]
async fn only_whoever_may_play_the_track_watches_its_clip() {
    let fixture = fixture("clip-access").await;
    let (_stranger, stranger) = admin_with_api_token(&fixture.state, "clip-stranger").await;
    for (method, uri) in [
        ("GET", format!("/api/v2/tracks/{}/clip", fixture.song)),
        (
            "POST",
            format!("/api/v2/tracks/{}/clip-ticket", fixture.song),
        ),
    ] {
        let status = send(&fixture.router, method, &uri, Some(&stranger), None)
            .await
            .status();
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} as a stranger"
        );
    }

    let url = ticket(&fixture, &fixture.song, "clip-ticket").await;
    set_disabled(&fixture.state, fixture.owner, true).await;
    let status = send(&fixture.router, "GET", &url, None, None)
        .await
        .status();
    assert_eq!(status, StatusCode::NOT_FOUND, "a disabled account's ticket");
    set_disabled(&fixture.state, fixture.owner, false).await;
    let status = send(&fixture.router, "GET", &url, None, None)
        .await
        .status();
    assert_eq!(status, StatusCode::OK, "the refusal was the account's");
}

/// Decision 10: the Subsonic façade is frozen. A song with a clip answers
/// exactly what it answered before, `isVideo` false and no new field.
#[tokio::test]
async fn the_subsonic_facade_says_nothing_of_a_clip() {
    let fixture = fixture("clip-subsonic").await;
    let api_key = format!("wfsk_{}", uuid::Uuid::new_v4().simple());
    fixture
        .state
        .db
        .set_subsonic_credential(
            fixture.owner,
            fixture.owner,
            &fixture
                .state
                .secret_box
                .encrypt(uuid::Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let with = subsonic_json(
        &fixture.router,
        "getSong",
        &api_key,
        &format!("&id={}", fixture.song),
    )
    .await;
    let without = subsonic_json(
        &fixture.router,
        "getSong",
        &api_key,
        &format!("&id={}", fixture.plain),
    )
    .await;
    let song = &with["subsonic-response"]["song"];
    assert_eq!(song["isVideo"], false);
    let keys = |song: &serde_json::Value| {
        let mut keys: Vec<String> = song.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(
        keys(song),
        keys(&without["subsonic-response"]["song"]),
        "a clip adds no field to a Subsonic song"
    );
}
