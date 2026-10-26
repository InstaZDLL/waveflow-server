use super::*;

#[test]
fn a_padded_sixteen_bit_file_uses_sixteen_bits() {
    // A 16-bit sample decoded to s32 is shifted left by sixteen.
    assert_eq!(effective_bits(0x7FFF_0000 | 0x0001_0000, false), Some(16));
    // A 24-bit one by eight, and a genuine one sets its lowest bit.
    assert_eq!(effective_bits(0x0000_0100, false), Some(24));
    assert_eq!(effective_bits(0x0000_0001, false), Some(32));
    assert_eq!(effective_bits(0, false), None);
    assert_eq!(effective_bits(0x0001_0000, true), None);
}

#[test]
fn the_spectrum_is_stored_under_its_loudest_band() {
    let mut db = vec![-20.0; 2049];
    for level in db.iter_mut().skip(1500) {
        *level = -80.0;
    }
    let stored = quantise_spectrum(&db);
    assert_eq!(stored.len(), SPECTRUM_BANDS);
    assert_eq!(stored[0], 0);
    assert_eq!(stored[SPECTRUM_BANDS - 1], 120);
    assert_eq!(spectrum_db(&stored)[SPECTRUM_BANDS - 1], -60.0);
}

#[test]
fn a_spectrogram_gives_up_before_the_request_does() {
    assert_eq!(
        spectrogram_deadline(Duration::from_secs(30)),
        Duration::from_secs(20)
    );
    assert_eq!(
        spectrogram_deadline(Duration::from_secs(10)),
        Duration::from_secs(5)
    );
    assert_eq!(
        spectrogram_deadline(Duration::from_secs(2)),
        Duration::from_secs(1)
    );
    // The shortest setting the configuration accepts still leaves the
    // route time to answer for itself.
    assert_eq!(
        spectrogram_deadline(Duration::from_secs(1)),
        Duration::from_millis(500)
    );
}

#[test]
fn the_decoder_digest_reads_past_the_first_line() {
    let one = b"ffmpeg version 8.0\nconfiguration: --enable-libwebp\nlibavcodec 62. 11.100 / 62. 11.100\n";
    let two = b"ffmpeg version 8.0\nconfiguration: --disable-libwebp\nlibavcodec 62. 11.100 / 62. 11.100\n";
    let three = b"ffmpeg version 8.0\nconfiguration: --enable-libwebp\nlibavcodec 62. 11.100 / 62. 12.100\n";
    assert_ne!(decoder_digest(one), decoder_digest(two));
    assert_ne!(decoder_digest(one), decoder_digest(three));
    assert_eq!(decoder_digest(one).len(), 64);
}

/// The probe, the measurement and the spectrogram open files and nothing
/// else (RFC-023 decision 2): an input that is a URL is refused by
/// ffprobe and by FFmpeg before either connects to anything, and a real
/// file still measures.
#[tokio::test]
async fn the_analysis_opens_files_and_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real.wav");
    crate::media::tests::silent_wav(&real, 1);
    let ffmpeg = Path::new("ffmpeg");
    let ffprobe = Path::new("ffprobe");
    let deadline = Duration::from_secs(60);
    assert!(
        matches!(
            measure(ffprobe, ffmpeg, &real, deadline).await,
            Measurement::Silent { .. }
        ),
        "a real file must still be measured"
    );

    let (address, connections, listener) = crate::media::tests::counting_listener().await;
    let url = format!("http://{address}/track.flac");
    assert!(matches!(
        measure(ffprobe, ffmpeg, Path::new(&url), deadline).await,
        Measurement::Unreadable(Failure::Decode)
    ));
    assert!(spectrogram(ffmpeg, Path::new(&url), false, deadline)
        .await
        .is_none());
    tokio::time::sleep(Duration::from_millis(200)).await;
    listener.abort();
    assert_eq!(
        connections.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "neither tool may contact anything"
    );
}
