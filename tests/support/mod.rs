//! Fixtures shared by every integration target.
//!
//! A module rather than a target of its own: anything directly under
//! `tests/` is compiled as its own test binary, and this is not one.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;
use waveflow_server::catalog::CatalogTrackInput;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::Config;

pub async fn test_app() -> (TempDir, Config, waveflow_server::AppState) {
    let temp = tempfile::tempdir().unwrap();
    let config = Config::for_data_dir(temp.path().join("data"));
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

/// A [`CatalogTrackInput`] the fixtures below build on.
///
/// They differ from each other on purpose — a browse test must not
/// accidentally match a catalogue test's rows — so this holds only what they
/// had in common and nothing that depends on their arguments; the rest sits at
/// rest here and is overridden by both, so no fixture ever reads these values.
/// What each one still sets is the whole of what makes it that fixture, and a
/// new field on the struct is filled in one place instead of two.
fn blank_input() -> CatalogTrackInput {
    CatalogTrackInput {
        artists: Vec::new(),
        album_artists: Vec::new(),
        roles: Vec::new(),
        performer_pairs: Vec::new(),
        channels: Some(2),
        codec: Some("FLAC".into()),
        musical_key: None,
        tag_rating: None,
        musicbrainz_recording_id: None,
        musicbrainz_release_id: None,
        musicbrainz_artist_id: None,
        replay_gain_track_gain: None,
        replay_gain_track_peak: None,
        replay_gain_album_gain: None,
        replay_gain_album_peak: None,
        bpm: None,
        sort_title: None,
        sort_album: None,
        sort_album_artist: None,
        sort_artist: None,
        comment: None,
        isrc: None,
        moods: None,
        explicit_status: None,
        original_release_date: None,
        release_date: None,
        release_types: None,
        record_labels: None,
        disc_subtitle: None,
        artwork: None,
        lyrics_hash: blake3::hash(b"").to_hex().to_string(),
        lyrics: Vec::new(),
        relative_path: String::new(),
        file_size: 0,
        modified_at: 0,
        quick_hash: String::new(),
        full_hash: String::new(),
        title: String::new(),
        artist: None,
        album: None,
        album_artist: None,
        is_compilation: false,
        genre: None,
        year: None,
        track_number: None,
        disc_number: None,
        duration_ms: 0,
        bitrate: None,
        sample_rate: None,
        bit_depth: None,
    }
}

pub fn catalog_input(index: usize, artist: &str) -> CatalogTrackInput {
    CatalogTrackInput {
        relative_path: format!("track-{index}.flac"),
        file_size: 1024 + index as i64,
        modified_at: 1_700_000_000_000 + index as i64,
        quick_hash: format!("{:064x}", index + 1),
        full_hash: format!("{:064x}", index + 101),
        title: format!("Compilation track {index}"),
        artist: Some(artist.into()),
        album: Some("Shared compilation".into()),
        album_artist: None,
        is_compilation: true,
        genre: Some("Rock; Pop".into()),
        year: Some(2026),
        track_number: Some(index as i64 + 1),
        disc_number: Some(1),
        duration_ms: 180_000,
        bitrate: Some(1_000),
        sample_rate: Some(48_000),
        bit_depth: Some(24),
        ..blank_input()
    }
}

pub async fn run_scan(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    library: LibraryRecord,
) {
    scan_once(state, owner, library).await;
}

/// Queues a scan and waits for it, answering the job it queued.
///
/// [`run_scan`] is this without the identifier, which is all most callers want.
/// A test that has to read the job back — to assert what a scan skipped, or
/// that a second one ran at all — needs the id, and had been carrying its own
/// copy of this loop to get it.
pub async fn scan_once(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    library: LibraryRecord,
) -> uuid::Uuid {
    let id = state
        .scanner
        .trigger(library, Some(owner), "manual")
        .await
        .unwrap();
    for _ in 0..200 {
        let job = state
            .db
            .scan_job_for_user(owner, id)
            .await
            .unwrap()
            .unwrap();
        if job.status == "completed" {
            return id;
        }
        if job.status == "failed" {
            panic!("scan failed: {:?}", job.message);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("scan timed out");
}

pub fn write_test_wav(path: &std::path::Path) {
    write_test_wav_of_len(path, 800);
}

/// The same file at another length, which is what a re-encode looks like from
/// the outside: different bytes, same tags.
pub fn write_test_wav_of_len(path: &std::path::Path, sample_count: usize) {
    let sample_rate = 8_000u32;
    let samples = vec![0i16; sample_count];
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

pub fn write_test_dsf(path: &std::path::Path) {
    let samples_per_channel = 32_768u64;
    let channels = 2u32;
    let rate = 2_822_400u32;
    let payload_bytes = (samples_per_channel / 8) * channels as u64;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DSD ");
    bytes.extend_from_slice(&28u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&52u64.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&samples_per_channel.to_le_bytes());
    bytes.extend_from_slice(&4096u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(payload_bytes + 12).to_le_bytes());
    bytes.extend(std::iter::repeat_n(0xAA, payload_bytes as usize));
    std::fs::write(path, bytes).unwrap();
}

pub fn write_test_png(path: &std::path::Path) {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z4m8AAAAASUVORK5CYII=")
        .unwrap();
    std::fs::write(path, bytes).unwrap();
}

pub async fn wait_for_cache_file(dir: &std::path::Path, extension: &str) -> std::path::PathBuf {
    for _ in 0..100 {
        if let Some(path) = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some(extension))
        {
            return path;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("transcode cache file was not committed")
}

pub fn generate_audio_fixture(path: &std::path::Path, codec: &str, extension: &str) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.15",
            "-metadata",
            &format!("title=Matrix {extension}"),
            "-metadata",
            "artist=Alpha; Beta",
            "-metadata",
            "album=WaveFlow format matrix",
            "-metadata",
            "album_artist=Matrix Artist",
            "-metadata",
            "genre=Electronic; Test",
            // A credit that is not the track's artist, so a test can show that
            // correcting the artist leaves the other twelve roles alone.
            "-metadata",
            "composer=Session Composer",
            "-c:a",
            codec,
        ])
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("FFmpeg is required for the format matrix: {error}"));
    assert!(
        output.status.success(),
        "FFmpeg failed for {extension}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn json_request(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

pub async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

/// Whether a JSON object omits a key outright.
///
/// `value[key].is_null()` cannot tell an absent key from one explicitly set to
/// `null`, and under the OpenSubsonic presence rule the two say different
/// things: absent means the server does not support the field at all, where a
/// null would be a value it chose to send. The server emits no explicit nulls
/// today, which is exactly why the assertion has to name which one it means.
pub fn omits(value: &serde_json::Value, key: &str) -> bool {
    value.as_object().expect("a JSON object").get(key).is_none()
}

pub async fn subsonic_json(
    router: &axum::Router,
    method: &str,
    api_key: &str,
    extra: &str,
) -> serde_json::Value {
    let response = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/{method}.view?apiKey={api_key}&v=1.16.1&c=golden&f=json{extra}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert!(status.is_success(), "{method} returned {status}: {body}");
    body
}

pub async fn login_token(router: &axum::Router, username: &str, password: &str) -> String {
    login_session(router, username, password).await.0
}

/// The access token and the device the login registered.
///
/// A caller that has to name its device on a mutation needs both, and the
/// device id is only ever handed out here.
pub async fn login_session(
    router: &axum::Router,
    username: &str,
    password: &str,
) -> (String, String) {
    let response = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": username,
                "password": password,
                "device_name": "Route test"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["device_id"].as_str().unwrap().to_owned(),
    )
}

/// Catalogue fixture for the native browse endpoints. Unlike [`catalog_input`]
/// it is not a compilation, so `album_artist_id` is populated and the artist
/// drill-down has something to resolve.
#[allow(clippy::too_many_arguments)]
pub fn browse_input(
    index: usize,
    title: &str,
    album: &str,
    artist: &str,
    track_number: Option<i64>,
    disc_number: Option<i64>,
) -> CatalogTrackInput {
    CatalogTrackInput {
        relative_path: format!("browse-{index}.flac"),
        file_size: 2048 + index as i64,
        modified_at: 1_700_000_000_000 + index as i64,
        quick_hash: format!("{:064x}", index + 500),
        full_hash: format!("{:064x}", index + 900),
        title: title.into(),
        artist: Some(artist.into()),
        album: Some(album.into()),
        album_artist: Some(artist.into()),
        is_compilation: false,
        genre: Some("Ambient".into()),
        year: Some(2024),
        track_number,
        disc_number,
        duration_ms: 120_000,
        bitrate: Some(900),
        sample_rate: Some(44_100),
        bit_depth: Some(16),
        ..blank_input()
    }
}
