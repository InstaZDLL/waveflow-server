//! The analysis itself: each claim checked against the audio, when a
//! measurement is current, what a failure records, and the port held to the
//! desktop.

use super::*;

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
