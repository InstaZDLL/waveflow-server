//! Transcoding through the façade: formats it cannot produce, AAC, and replay
//! gain.

use super::*;

/// A format the façade cannot produce plays something rather than nothing
/// (RFC-022 decision 6), and `aac` is one it can (decision 5).
///
/// The unknown format is read as no format at all: the original while the cap
/// admits it, MP3 at the cap when it does not — exactly the answers a request
/// naming no format already gets, so the fallback adds no path of its own. Each
/// answer is identified by its `Content-Type` and its first bytes, because a
/// refusal is an HTTP 200 too and would pass a status check.
#[tokio::test]
async fn a_format_the_facade_cannot_produce_falls_back_and_aac_is_produced() {
    let (_temp, config, state) = test_app().await;
    let owner = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, 'format-owner', 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4());
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("format-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav_of_len(&music.join("Format.wav"), 8_000);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Formats",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        waveflow_server::storage::catalog::LibraryRecord {
            id: library,
            name: "Formats".into(),
            root_path: root,
        },
    )
    .await;
    let song = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let stream = |extra: &'static str| {
        let router = router.clone();
        let uri = format!("/rest/stream.view?apiKey={api_key}&id={song}&v=1.16.1&c=golden{extra}");
        async move {
            let response = router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{extra}");
            let kind = response.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned();
            let bytes = http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes();
            (kind, bytes)
        }
    };

    // Asked for FLAC over a WAV: the original, which is what the client is
    // after, where it used to be error 10.
    for extra in ["&format=flac", "&format=wav", "&format=nonsense"] {
        let (kind, bytes) = stream(extra).await;
        assert_eq!(kind, "audio/wav", "{extra}");
        assert!(bytes.starts_with(b"RIFF"), "{extra}");
    }
    // Under a cap the source exceeds, the same fallback a request naming no
    // format takes: MP3 at the cap.
    let (kind, bytes) = stream("&format=flac&maxBitRate=32").await;
    assert_eq!(kind, "audio/mpeg");
    assert!(bytes.len() > 32);
    // AAC is produced, under each name a client spells it with.
    for extra in ["&format=aac", "&format=m4a", "&format=AAC"] {
        let (kind, bytes) = stream(extra).await;
        assert_eq!(kind, "audio/aac", "{extra}");
        assert!(bytes[0] == 0xFF && bytes[1] & 0xF0 == 0xF0, "{extra}");
    }
    // And the formats it always knew are untouched.
    let (kind, _) = stream("&format=mp3").await;
    assert_eq!(kind, "audio/mpeg");
    let (kind, bytes) = stream("&format=raw").await;
    assert_eq!(kind, "audio/wav");
    assert!(bytes.starts_with(b"RIFF"));
}

/// The mean volume FFmpeg measures in an encoded stream, in dB.
fn mean_volume(bytes: &[u8], scratch: &std::path::Path) -> f64 {
    let path = scratch.join(format!("{}.mp3", Uuid::new_v4()));
    std::fs::write(&path, bytes).unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(&path)
        .args(["-af", "volumedetect", "-f", "null", "-"])
        .output()
        .expect("FFmpeg is required");
    let report = String::from_utf8_lossy(&output.stderr);
    report
        .lines()
        .find_map(|line| line.split("mean_volume:").nth(1))
        .and_then(|value| value.trim().trim_end_matches("dB").trim().parse().ok())
        .unwrap_or_else(|| panic!("no mean volume in: {report}"))
}

/// ReplayGain on the Subsonic stream (RFC-022, what was still open): the
/// operator's default levels a transcode, `replayGain=` overrides it either
/// way, the original is served untouched whatever the mode, and a mode this
/// server does not know is refused rather than guessed.
#[tokio::test]
async fn a_subsonic_transcode_carries_the_tracks_replay_gain_when_asked() {
    use http_body_util::BodyExt;

    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.subsonic_replay_gain = waveflow_server::media::replay_gain::ReplayGainMode::Track;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let (owner, api_key) = subsonic_account(&state, "levelled").await;
    let music = config.data_dir.join("levelled-music");
    std::fs::create_dir_all(&music).unwrap();
    let source = music.join("Tone.flac");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "flac",
        ])
        .arg(&source)
        .output()
        .expect("FFmpeg is required");
    assert!(made.status.success());
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "levelled",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        waveflow_server::storage::catalog::LibraryRecord {
            id: library,
            name: "levelled".into(),
            root_path: root,
        },
    )
    .await;
    let track: String = sqlx::query_scalar("SELECT id FROM track WHERE library_id = ?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE track SET replay_gain_track_gain = -10.0, replay_gain_track_peak = NULL \
         WHERE id = ?",
    )
    .bind(&track)
    .execute(state.db.pool())
    .await
    .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let stream = |extra: String| {
        let router = router.clone();
        let uri = format!(
            "/rest/stream.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={track}{extra}"
        );
        async move {
            let response = router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let kind = response.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            (kind, bytes)
        }
    };

    let (kind, plain) = stream("&format=mp3&replayGain=off".into()).await;
    assert_eq!(kind, "audio/mpeg");
    let (_, levelled) = stream("&format=mp3".into()).await;
    let (_, asked) = stream("&format=mp3&replayGain=album".into()).await;
    let plain = mean_volume(&plain, temp.path());
    let levelled = mean_volume(&levelled, temp.path());
    let asked = mean_volume(&asked, temp.path());
    assert!(
        (plain - levelled - 10.0).abs() < 1.0,
        "the default levels the encode by the track gain: {plain} then {levelled}"
    );
    assert!(
        (plain - asked - 10.0).abs() < 1.0,
        "album falls back to the track's gain: {plain} then {asked}"
    );

    // The original is never re-encoded to carry it.
    let (kind, original) = stream("&format=raw&replayGain=track".into()).await;
    assert_ne!(kind, "audio/mpeg");
    assert_eq!(
        original.as_ref(),
        std::fs::read(&source).unwrap().as_slice()
    );

    let (kind, refused) = stream("&format=mp3&replayGain=loud".into()).await;
    assert!(kind.starts_with("application/json"), "{kind}");
    let refused: serde_json::Value = serde_json::from_slice(&refused).unwrap();
    assert_eq!(refused["subsonic-response"]["error"]["code"], 10);
}
