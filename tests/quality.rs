//! The real quality of a file, measured from its audio (RFC-016).
//!
//! Every file here is made by FFmpeg from noise, so its spectrum is known: a
//! full band, the wall LAME leaves at 128 kbps, the filter of a 44.1 kHz
//! source resampled to 96 kHz, a 16-bit signal in a 24-bit container. The
//! scanner indexes them for real, and the analysis measures them through the
//! same FFmpeg the server runs.
//!
//! The accounts are rows written by hand and reach the routes through an API
//! token, as in `tests/credits.rs`: nothing here hands a literal to a hash.

#[allow(dead_code)]
mod support;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use support::*;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::services::{AnalysisState, AttentionCategory, BrowsePage, QualityVerdict};
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::{LibraryRole, LibraryVisibility};
use waveflow_server::AppState;

struct Shelf {
    state: AppState,
    config: waveflow_server::Config,
    owner: Uuid,
    library: LibraryRecord,
    music: PathBuf,
    _temp: tempfile::TempDir,
}

async fn account(state: &AppState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

async fn shelf(name: &str) -> Shelf {
    let (temp, config, state) = test_app().await;
    let owner = account(&state, name).await;
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    Shelf {
        state,
        config,
        owner,
        library: LibraryRecord {
            id,
            name: name.into(),
            root_path: root,
        },
        music,
        _temp: temp,
    }
}

/// Runs FFmpeg with these arguments, failing the test on any error.
fn ffmpeg(args: &[&str]) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("FFmpeg must be on PATH");
    assert!(
        output.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// White noise: a full band, which no lossy encoder keeps.
fn noise(rate: u32, seconds: u32, seed: u32) -> String {
    format!("anoisesrc=d={seconds}:c=white:a=0.3:r={rate}:s={seed}")
}

/// A genuine 16-bit FLAC: the full band, and every bit it declares.
fn genuine(path: &Path, seed: u32) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, seed),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A FLAC made from a 128 kbps MP3 of the same noise.
fn from_mp3(path: &Path, scratch: &Path) {
    let mp3 = scratch.join(format!(
        "{}.mp3",
        path.file_stem().unwrap().to_string_lossy()
    ));
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, 7),
        "-ac",
        "2",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "128k",
        mp3.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-i",
        mp3.to_str().unwrap(),
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A 44.1 kHz recording resampled to 96 kHz and written at 24 bits: a hi-res
/// file holding nothing past what the CD held.
fn upsampled(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, 11),
        "-ac",
        "2",
        "-af",
        "aresample=96000",
        "-sample_fmt",
        "s32",
        "-bits_per_raw_sample",
        "24",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// Noise recorded at 96 kHz: a genuine hi-res file, full band to Nyquist.
fn hi_res(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(96_000, 6, 13),
        "-ac",
        "2",
        "-sample_fmt",
        "s32",
        "-bits_per_raw_sample",
        "24",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A 16-bit signal written into a 24-bit container: its low eight bits are
/// zero in every sample.
///
/// Through a 16-bit WAV, in two runs: in one filter graph FFmpeg negotiates
/// the 16-bit step away and the noise keeps all 24 bits.
fn padded(path: &Path, scratch: &Path) {
    let wav = scratch.join("padded-16.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, 17),
        "-ac",
        "2",
        "-c:a",
        "pcm_s16le",
        wav.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-i",
        wav.to_str().unwrap(),
        "-sample_fmt",
        "s32",
        "-bits_per_raw_sample",
        "24",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

impl Shelf {
    async fn scan(&self) {
        run_scan(&self.state, self.owner, self.library.clone()).await;
    }

    async fn analyze(&self) -> usize {
        self.state.services.analyze_pending_quality().await.unwrap()
    }

    async fn track(&self, relative: &str) -> Uuid {
        let id: String =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
                .bind(self.library.id.to_string())
                .bind(relative)
                .fetch_one(self.state.db.pool())
                .await
                .unwrap_or_else(|_| panic!("no track at {relative:?}"));
        Uuid::parse_str(&id).unwrap()
    }

    async fn verdict(&self, relative: &str) -> QualityVerdict {
        let track = self.track(relative).await;
        self.state
            .services
            .track_quality(self.owner, track)
            .await
            .unwrap()
            .quality
            .verdict
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM track_quality")
            .fetch_one(self.state.db.pool())
            .await
            .unwrap()
    }

    /// A token for the account named `username`, minted by the owner.
    async fn token(&self, username: &str) -> String {
        self.state
            .services
            .create_api_token(self.owner, &[], username, "quality", &[])
            .await
            .unwrap()
            .1
    }

    async fn join(&self, user: Uuid, role: LibraryRole) {
        self.state
            .db
            .add_library_member(self.owner, self.library.id, user, role, now_ms())
            .await
            .unwrap();
    }
}

async fn send(
    router: &axum::Router,
    token: &str,
    request: Request<Body>,
) -> axum::response::Response {
    let (mut parts, body) = request.into_parts();
    parts
        .headers
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    router
        .clone()
        .oneshot(Request::from_parts(parts, body))
        .await
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn each_claim_is_checked_against_the_audio() {
    let shelf = shelf("claims").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    upsampled(&shelf.music.join("upsampled.flac"));
    hi_res(&shelf.music.join("hi-res.flac"));
    padded(&shelf.music.join("padded.flac"), &scratch);
    std::fs::copy(scratch.join("from-mp3.mp3"), shelf.music.join("lossy.mp3")).unwrap();
    shelf.scan().await;

    assert_eq!(
        shelf.verdict("genuine.flac").await,
        QualityVerdict::Pending,
        "nothing is measured on the answering path"
    );
    assert_eq!(
        shelf.analyze().await,
        5,
        "the five lossless files, not the MP3"
    );

    let detail = |relative: &'static str| {
        let shelf = &shelf;
        async move {
            let track = shelf.track(relative).await;
            shelf
                .state
                .services
                .track_quality(shelf.owner, track)
                .await
                .unwrap()
        }
    };
    let fake = detail("from-mp3.flac").await;
    assert_eq!(fake.quality.verdict, QualityVerdict::LossySource);
    let cutoff = fake.quality.cutoff_hz.expect("a wall");
    assert!(
        (15_000.0..18_000.0).contains(&cutoff),
        "LAME's wall at 128 kbps, found at {cutoff}"
    );
    assert_eq!(fake.spectrum.as_ref().map(Vec::len), Some(256));

    let genuine = detail("genuine.flac").await;
    assert_eq!(genuine.quality.verdict, QualityVerdict::Consistent);
    assert_eq!(genuine.quality.cutoff_hz, None, "white noise has no wall");
    assert_eq!(genuine.quality.effective_bits, Some(16));

    let resampled = detail("upsampled.flac").await;
    assert_eq!(
        resampled.quality.verdict,
        QualityVerdict::Upsampled,
        "{:?}",
        resampled.quality
    );
    assert_eq!(
        detail("hi-res.flac").await.quality.verdict,
        QualityVerdict::Consistent
    );

    let padded = detail("padded.flac").await;
    assert_eq!(padded.quality.verdict, QualityVerdict::Padded);
    assert_eq!(padded.quality.effective_bits, Some(16));

    assert_eq!(shelf.verdict("lossy.mp3").await, QualityVerdict::Lossy);
    assert_eq!(
        shelf.analyze().await,
        0,
        "a measured file is not measured again"
    );
}

#[tokio::test]
async fn a_measurement_belongs_to_the_content_it_measured() {
    let shelf = shelf("content").await;
    genuine(&shelf.music.join("a.flac"), 3);
    genuine(&shelf.music.join("b.flac"), 5);
    shelf.scan().await;
    assert_eq!(shelf.analyze().await, 2);
    let a = shelf.track("a.flac").await;

    // Moved: the scanner relocates it by its hash, and the measurement holds.
    std::fs::create_dir_all(shelf.music.join("moved")).unwrap();
    std::fs::rename(
        shelf.music.join("a.flac"),
        shelf.music.join("moved").join("a.flac"),
    )
    .unwrap();
    shelf.scan().await;
    assert_eq!(shelf.track("moved/a.flac").await, a, "the same track");
    assert_eq!(
        shelf.verdict("moved/a.flac").await,
        QualityVerdict::Consistent
    );
    assert_eq!(shelf.analyze().await, 0, "a move re-measures nothing");

    // Changed in place: the same track, another content, nothing current.
    let b = shelf.track("b.flac").await;
    genuine(&shelf.music.join("b.flac"), 9);
    shelf.scan().await;
    assert_eq!(shelf.track("b.flac").await, b, "the same track");
    assert_eq!(shelf.verdict("b.flac").await, QualityVerdict::Pending);
    assert_eq!(shelf.analyze().await, 1);
    assert_eq!(shelf.verdict("b.flac").await, QualityVerdict::Consistent);
}

#[tokio::test]
async fn another_ffmpeg_or_another_analyser_measures_again() {
    let shelf = shelf("decoder").await;
    genuine(&shelf.music.join("a.flac"), 3);
    genuine(&shelf.music.join("b.flac"), 5);
    shelf.scan().await;
    assert_eq!(shelf.analyze().await, 2);
    let digest = shelf.state.media.ffmpeg_digest().to_owned();
    sqlx::query("UPDATE track_quality SET decoder = ? WHERE track_id = ?")
        .bind("0".repeat(64))
        .bind(shelf.track("a.flac").await.to_string())
        .execute(shelf.state.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE track_quality SET version = version + 1 WHERE track_id = ?")
        .bind(shelf.track("b.flac").await.to_string())
        .execute(shelf.state.db.pool())
        .await
        .unwrap();

    // What the boot does, with this process's own FFmpeg.
    let removed = shelf
        .state
        .services
        .use_quality_decoder(waveflow_server::quality::Decoder {
            digest,
            webp: false,
        })
        .await
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(shelf.verdict("a.flac").await, QualityVerdict::Pending);
    assert_eq!(shelf.verdict("b.flac").await, QualityVerdict::Pending);
    assert_eq!(shelf.analyze().await, 2);
}

/// Where a FLAC file's first audio frame starts: past `fLaC` and every
/// metadata block, the last of which sets the top bit of its header.
fn flac_frames_start(bytes: &[u8]) -> usize {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut at = 4;
    loop {
        let last = bytes[at] & 0x80 != 0;
        let length = u32::from_be_bytes([0, bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        at += 4 + length;
        if last {
            return at;
        }
    }
}

#[tokio::test]
async fn a_file_ffmpeg_cannot_read_is_recorded_once() {
    let shelf = shelf("unreadable").await;
    let whole = shelf.config.data_dir.join("whole.flac");
    genuine(&whole, 3);
    // The metadata intact and every frame zeroed: the scanner reads the
    // stream's description, and FFmpeg finds no frame to decode.
    let mut bytes = std::fs::read(&whole).unwrap();
    let frames = flac_frames_start(&bytes);
    bytes[frames..].fill(0);
    std::fs::write(shelf.music.join("broken.flac"), &bytes).unwrap();
    shelf.scan().await;

    assert_eq!(shelf.analyze().await, 1);
    assert_eq!(
        shelf.verdict("broken.flac").await,
        QualityVerdict::Unreadable
    );
    assert_eq!(
        shelf.analyze().await,
        0,
        "not retried while it is the same file"
    );
}

#[tokio::test]
async fn a_running_scan_holds_the_analysis_back() {
    let shelf = shelf("yield").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;

    let scan = shelf.state.scanner.begin_scan(shelf.library.id);
    let services = shelf.state.services.clone();
    let pass = tokio::spawn(async move { services.analyze_pending_quality().await.unwrap() });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        shelf.rows().await,
        0,
        "nothing is measured while a scan runs"
    );
    assert!(!pass.is_finished());

    drop(scan);
    assert_eq!(pass.await.unwrap(), 1);
    assert_eq!(shelf.rows().await, 1);
}

/// Shaped noise, as the desktop's own tests make it: white noise through an
/// FFT mask, so the spectrum is exactly what the test says.
fn shaped_noise(rate: f64, seconds: f64, seed: u32, gain_db: impl Fn(f64) -> f64) -> Vec<f32> {
    use realfft::RealFftPlanner;
    const FRAME: usize = 4096;
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FRAME);
    let inverse = planner.plan_fft_inverse(FRAME);
    let mut state = seed;
    let mut out = Vec::new();
    while (out.len() as f64) < seconds * rate {
        let mut block: Vec<f32> = (0..FRAME)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) - 0.5
            })
            .collect();
        let mut spectrum = forward.make_output_vec();
        forward.process(&mut block, &mut spectrum).unwrap();
        for (k, bin) in spectrum.iter_mut().enumerate() {
            let hz = k as f64 * rate / FRAME as f64;
            *bin *= 10f32.powf((gain_db(hz) / 20.0) as f32);
        }
        let mut back = inverse.make_output_vec();
        inverse.process(&mut spectrum, &mut back).unwrap();
        out.extend(back.iter().map(|value| value / FRAME as f32));
    }
    out
}

/// A 16-bit PCM WAV, so both decoders hand the meter the same integers.
fn write_wav(path: &Path, rate: u32, channels: &[Vec<f32>]) {
    let frames = channels[0].len();
    let count = channels.len() as u16;
    let data_len = (frames * channels.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * u32::from(count) * 2).to_le_bytes());
    bytes.extend_from_slice(&(count * 2).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        for channel in channels {
            let sample = (channel[frame] * 4.0).clamp(-1.0, 1.0);
            bytes.extend_from_slice(&((sample * 32_767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}

/// The server's pipeline and the desktop's `analyze_file` find the same wall,
/// or the same absence of one, on the desktop's own test signals (RFC-016
/// decision 2).
#[tokio::test]
async fn the_port_finds_what_the_desktop_finds() {
    let temp = tempfile::tempdir().unwrap();
    let rate = 44_100.0;
    let tilt = |hz: f64| -3.0 * (hz.max(1000.0) / 1000.0).log2();
    let mut signals: Vec<(String, Vec<Vec<f32>>)> = Vec::new();
    for cut in [16_000.0, 19_000.0, 20_000.0] {
        signals.push((
            format!("cut-{cut}"),
            vec![shaped_noise(rate, 10.0, 1, |hz| {
                tilt(hz) + if hz > cut { -60.0 } else { 0.0 }
            })],
        ));
    }
    signals.push(("full".into(), vec![shaped_noise(rate, 10.0, 2, tilt)]));
    signals.push((
        "fade".into(),
        vec![shaped_noise(rate, 10.0, 3, |hz| {
            tilt(hz) - 12.0 * (hz.max(6000.0) / 6000.0).log2()
        })],
    ));
    let lows = shaped_noise(rate, 10.0, 4, |hz| {
        tilt(hz) + if hz > 15_000.0 { -80.0 } else { 0.0 }
    });
    let highs = shaped_noise(rate, 10.0, 5, |hz| {
        tilt(hz) + if hz > 15_000.0 { 0.0 } else { -80.0 }
    });
    signals.push((
        "opposition".into(),
        vec![
            lows.iter().zip(&highs).map(|(l, h)| l + h).collect(),
            lows.iter().zip(&highs).map(|(l, h)| l - h).collect(),
        ],
    ));

    let bin = rate / 4096.0;
    let mut walls = 0;
    for (name, channels) in signals {
        let path = temp.path().join(format!("{name}.wav"));
        write_wav(&path, rate as u32, &channels);
        let desktop = waveflow_core::analysis::analyze_file(&path)
            .unwrap()
            .bandwidth
            .expect("the desktop measured it");
        let server = waveflow_server::quality::measure(
            Path::new("ffprobe"),
            Path::new("ffmpeg"),
            &path,
            std::time::Duration::from_secs(60),
        )
        .await;
        let waveflow_server::quality::Measurement::Measured { cutoff_hz, .. } = server else {
            panic!("{name}: the server measured nothing: {server:?}");
        };
        match (desktop.cutoff_hz, cutoff_hz) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                walls += 1;
                assert!((a - b).abs() <= bin, "{name}: {a} against {b}");
            }
            other => panic!("{name}: the two disagree on the wall: {other:?}"),
        }
    }
    assert_eq!(walls, 3, "the three cuts, and nothing else");
}

#[tokio::test]
async fn members_read_the_quality_and_only_curators_ask_for_it_again() {
    let shelf = shelf("routes").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    shelf.scan().await;
    shelf.analyze().await;
    let listener = account(&shelf.state, "listener").await;
    account(&shelf.state, "stranger").await;
    shelf.join(listener, LibraryRole::Listener).await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let owning = shelf.token("routes").await;
    let listening = shelf.token("listener").await;
    let outside = shelf.token("stranger").await;
    let library = shelf.library.id;
    let fake = shelf.track("from-mp3.flac").await;

    let summary = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/libraries/{library}/quality")),
        )
        .await,
    )
    .await;
    assert_eq!(summary["enabled"], true);
    assert_eq!(summary["lossless"], 2);
    assert_eq!(summary["measured"], 2);
    assert_eq!(summary["verdicts"][0]["verdict"], "lossy_source");
    assert_eq!(summary["verdicts"][0]["count"], 1);

    let list = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/libraries/{library}/quality/tracks")),
        )
        .await,
    )
    .await;
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], fake.to_string(), "the worst first");
    assert_eq!(list[0]["quality"]["verdict"], "lossy_source");
    assert_eq!(list[0]["quality"]["nyquist_hz"], 22_050.0);
    assert_eq!(list[1]["quality"]["verdict"], "consistent");

    let filtered = json_body(
        send(
            &router,
            &owning,
            get(&format!(
                "/api/v2/libraries/{library}/quality/tracks?verdict=consistent"
            )),
        )
        .await,
    )
    .await;
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(
        send(
            &router,
            &owning,
            get(&format!(
                "/api/v2/libraries/{library}/quality/tracks?verdict=fine"
            )),
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let detail = json_body(
        send(
            &router,
            &listening,
            get(&format!("/api/v2/tracks/{fake}/quality")),
        )
        .await,
    )
    .await;
    assert_eq!(detail["verdict"], "lossy_source");
    assert_eq!(detail["spectrum"].as_array().unwrap().len(), 256);

    let analysis = || {
        Request::post(format!("/api/v2/libraries/{library}/quality/analysis"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, &listening, analysis()).await.status(),
        StatusCode::NOT_FOUND,
        "a listener cannot spend the operator's CPU"
    );
    assert_eq!(shelf.rows().await, 2);
    assert_eq!(
        send(&router, &owning, analysis()).await.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        shelf.rows().await,
        0,
        "the library's measurements are dropped"
    );
    assert_eq!(shelf.analyze().await, 2);

    for uri in [
        format!("/api/v2/libraries/{library}/quality"),
        format!("/api/v2/libraries/{library}/quality/tracks"),
        format!("/api/v2/tracks/{fake}/quality"),
        format!("/api/v2/tracks/{fake}/spectrogram"),
    ] {
        assert_eq!(
            send(&router, &outside, get(&uri)).await.status(),
            StatusCode::NOT_FOUND,
            "a stranger is told nothing at {uri}"
        );
    }
    assert_eq!(
        send(&router, &outside, analysis()).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_spectrogram_is_revalidated_by_whoever_asks() {
    let shelf = shelf("spectrogram").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let listener = account(&shelf.state, "listener").await;
    shelf.join(listener, LibraryRole::Listener).await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let listening = shelf.token("listener").await;
    let track = shelf.track("a.flac").await;
    let uri = format!("/api/v2/tracks/{track}/spectrogram");

    let response = send(&router, &listening, get(&uri)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        content_type == "image/webp" || content_type == "image/jpeg",
        "{content_type}"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-cache",
        "kept, but asked about before every use"
    );
    let etag = response.headers()[header::ETAG]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let magic: &[u8] = if content_type == "image/webp" {
        b"RIFF"
    } else {
        &[0xFF, 0xD8]
    };
    assert!(bytes.starts_with(magic), "a picture of the type it names");

    let revalidate = || {
        Request::get(&uri)
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, &listening, revalidate()).await.status(),
        StatusCode::NOT_MODIFIED
    );

    shelf
        .state
        .db
        .remove_library_member(shelf.owner, shelf.library.id, listener, now_ms())
        .await
        .unwrap();
    assert_eq!(
        send(&router, &listening, revalidate()).await.status(),
        StatusCode::NOT_FOUND,
        "a browser's copy does not outlive the membership"
    );
}

#[tokio::test]
async fn one_spectrogram_per_account_at_a_time() {
    let shelf = shelf("bound").await;
    // Long enough that the first is still being drawn when the second asks.
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 240, 3),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        shelf.music.join("long.flac").to_str().unwrap(),
    ]);
    shelf.scan().await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let token = shelf.token("bound").await;
    let uri = format!(
        "/api/v2/tracks/{}/spectrogram",
        shelf.track("long.flac").await
    );

    let (first, second) = tokio::join!(
        send(&router, &token, get(&uri)),
        send(&router, &token, get(&uri))
    );
    let mut statuses = [first.status(), second.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS]);
    let busy = if first.status() == StatusCode::TOO_MANY_REQUESTS {
        first
    } else {
        second
    };
    assert!(busy.headers().contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn needs_attention_lists_the_fake_lossless_files() {
    let shelf = shelf("attention").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    padded(&shelf.music.join("padded.flac"), &scratch);
    shelf.scan().await;

    let count = || async {
        shelf
            .state
            .services
            .attention_summary(shelf.owner, shelf.library.id)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.category == AttentionCategory::ProbableFakeLossless)
            .unwrap()
            .count
    };
    assert_eq!(count().await, 0, "nothing before it is measured");
    shelf.analyze().await;
    assert_eq!(count().await, 1, "the fake, and not the padded file");
    let tracks = shelf
        .state
        .services
        .attention_tracks(
            shelf.owner,
            shelf.library.id,
            AttentionCategory::ProbableFakeLossless,
            BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].song.id, shelf.track("from-mp3.flac").await);
}

/// With the analysis off, the server draws one spectrogram at a time; a
/// stranger who asks while that one is drawn is told nothing exists, not that
/// the server is busy with it.
#[tokio::test]
async fn a_stranger_is_told_nothing_even_at_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.quality_parallelism = 0;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let owner = account(&state, "off").await;
    let music = config.data_dir.join("off");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, "off", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let shelf = Shelf {
        state,
        config,
        owner,
        library: LibraryRecord {
            id,
            name: "off".into(),
            root_path: root,
        },
        music,
        _temp: temp,
    };
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 240, 3),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        shelf.music.join("long.flac").to_str().unwrap(),
    ]);
    shelf.scan().await;
    account(&shelf.state, "stranger").await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let owning = shelf.token("off").await;
    let outside = shelf.token("stranger").await;
    let track = shelf.track("long.flac").await;
    let uri = format!("/api/v2/tracks/{track}/spectrogram");

    assert_eq!(shelf.analyze().await, 0, "the operator turned it off");
    let summary = json_body(
        send(
            &router,
            &owning,
            get(&format!("/api/v2/libraries/{}/quality", shelf.library.id)),
        )
        .await,
    )
    .await;
    assert_eq!(summary["enabled"], false);
    assert_eq!(summary["measured"], 0);

    let (drawn, refused) = tokio::join!(send(&router, &owning, get(&uri)), async {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        send(&router, &outside, get(&uri)).await
    });
    assert_eq!(drawn.status(), StatusCode::OK);
    assert_eq!(
        refused.status(),
        StatusCode::NOT_FOUND,
        "access is checked before the bound"
    );
}

/// The analysis's state, as the library's owner reads it.
async fn analysis(shelf: &Shelf) -> waveflow_server::services::QualityAnalysis {
    shelf
        .state
        .services
        .quality_summary(shelf.owner, shelf.library.id)
        .await
        .unwrap()
        .analysis
}

/// A minute of noise: long enough that a pass over a few of them can be
/// watched while it runs.
fn minute(path: &Path, seed: u32) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 60, seed),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

#[tokio::test]
async fn the_summary_says_where_the_analysis_stands() {
    let shelf = shelf("status").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;

    let before = analysis(&shelf).await;
    assert_eq!(
        before.state,
        AnalysisState::Queued,
        "files wait, and no pass is on them"
    );
    assert_eq!(before.in_flight, 0);
    assert_eq!(before.last_measured_at, None);

    let scan = shelf.state.scanner.begin_scan(shelf.library.id);
    assert_eq!(
        analysis(&shelf).await.state,
        AnalysisState::WaitingForScan,
        "its own library is being scanned"
    );
    drop(scan);

    shelf.analyze().await;
    let after = analysis(&shelf).await;
    assert_eq!(after.state, AnalysisState::Idle);
    assert_eq!(after.in_flight, 0);
    assert_eq!(after.pass_started_at, None);
    assert!(after.last_measured_at.is_some());
}

/// A pass over three files is watched while it runs: it says it is running,
/// names the file it is decoding, and the count moves one file at a time
/// rather than all at once at the end.
#[tokio::test]
async fn a_running_pass_moves_one_file_at_a_time() {
    let shelf = shelf("moving").await;
    for (index, seed) in [3, 5, 7].into_iter().enumerate() {
        minute(&shelf.music.join(format!("{index}.flac")), seed);
    }
    shelf.scan().await;

    let services = shelf.state.services.clone();
    let pass = tokio::spawn(async move { services.analyze_pending_quality().await.unwrap() });
    let mut seen_running = false;
    let mut seen_partial = false;
    for _ in 0..2000 {
        if pass.is_finished() {
            break;
        }
        let now = analysis(&shelf).await;
        if now.state == AnalysisState::Running && now.in_flight == 1 {
            assert!(now.pass_started_at.is_some());
            seen_running = true;
        }
        let rows = shelf.rows().await;
        if (1..3).contains(&rows) && !pass.is_finished() {
            seen_partial = true;
        }
        if seen_running && seen_partial {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        seen_running,
        "a running pass says so, with its file in flight"
    );
    assert!(
        seen_partial,
        "a measurement is written as it finishes, not at the end of the pass"
    );
    assert_eq!(pass.await.unwrap(), 3);
    assert_eq!(analysis(&shelf).await.state, AnalysisState::Idle);
}

/// A scan of somebody else's library holds every measurement back, but it
/// changes nothing its outsiders are shown: the pass reads as running and its
/// file as in flight, exactly as with no scan at all.
#[tokio::test]
async fn a_scan_the_caller_cannot_see_is_not_named() {
    let shelf = shelf("mine").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let stranger = account(&shelf.state, "stranger").await;
    let theirs_root = shelf.config.data_dir.join("theirs");
    std::fs::create_dir_all(&theirs_root).unwrap();
    let theirs = shelf
        .state
        .db
        .create_library(
            stranger,
            "theirs",
            &std::fs::canonicalize(&theirs_root).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let scan = shelf.state.scanner.begin_scan(theirs);
    let services = shelf.state.services.clone();
    let pass = tokio::spawn(async move { services.analyze_pending_quality().await.unwrap() });
    // Long enough for the pass to have reached the scan and stopped there.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!pass.is_finished(), "the pass waits for the scan");
    let held = analysis(&shelf).await;
    assert_eq!(
        (held.state, held.in_flight),
        (AnalysisState::Running, 1),
        "nothing says another library is being scanned"
    );
    assert!(held.pass_started_at.is_some());

    drop(scan);
    assert_eq!(pass.await.unwrap(), 1);
    assert_eq!(analysis(&shelf).await.state, AnalysisState::Idle);
}

#[tokio::test]
async fn the_route_carries_the_analysis() {
    let shelf = shelf("route-status").await;
    genuine(&shelf.music.join("a.flac"), 3);
    shelf.scan().await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let token = shelf.token("route-status").await;
    let summary = json_body(
        send(
            &router,
            &token,
            get(&format!("/api/v2/libraries/{}/quality", shelf.library.id)),
        )
        .await,
    )
    .await;
    assert_eq!(summary["analysis"]["state"], "queued");
    assert_eq!(summary["analysis"]["in_flight"], 0);
    assert!(summary["analysis"]["last_measured_at"].is_null());
}

/// The header and the list are one answer: for every verdict, the summary
/// counts exactly the files the list carries under it, including when some
/// files are measured, some wait, and a measured one has gone unavailable.
#[tokio::test]
async fn the_summary_counts_what_the_list_lists() {
    let shelf = shelf("agree").await;
    let scratch = shelf.config.data_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    genuine(&shelf.music.join("genuine.flac"), 3);
    from_mp3(&shelf.music.join("from-mp3.flac"), &scratch);
    padded(&shelf.music.join("padded.flac"), &scratch);
    genuine(&shelf.music.join("gone.flac"), 9);
    std::fs::copy(scratch.join("from-mp3.mp3"), shelf.music.join("lossy.mp3")).unwrap();
    shelf.scan().await;
    shelf.analyze().await;
    // Measured, then gone: its row stays, and neither side may count it.
    std::fs::remove_file(shelf.music.join("gone.flac")).unwrap();
    // Added after the pass: pending on both sides.
    genuine(&shelf.music.join("late.flac"), 11);
    shelf.scan().await;

    let summary = shelf
        .state
        .services
        .quality_summary(shelf.owner, shelf.library.id)
        .await
        .unwrap();
    let listed = shelf
        .state
        .services
        .quality_tracks(
            shelf.owner,
            shelf.library.id,
            None,
            None,
            BrowsePage::default(),
        )
        .await
        .unwrap();
    for entry in &summary.verdicts {
        let in_list = listed
            .iter()
            .filter(|track| track.quality.verdict == entry.verdict)
            .count() as i64;
        assert_eq!(entry.count, in_list, "{:?}", entry.verdict);
    }
    assert_eq!(listed.len(), 5);
    assert_eq!(summary.lossless, 4);
    assert_eq!(summary.measured, 3);
}
