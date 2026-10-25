//! The real quality of a file, measured from its audio (RFC-016).
//!
//! What lives here touches no database: decoding a file through FFmpeg, the
//! desktop's wall search ([`bandwidth`]), the effective bit depth, the
//! averaged spectrum as it is stored, and the spectrogram drawn on demand.
//! Which files are measured, when, and what a measurement means to a reader
//! belong to `services::quality`.
//!
//! **FFmpeg decodes, out of process** (decision 2). It reads every lossless
//! format the scanner indexes, APE and WavPack included, which the server's
//! symphonia cannot; a file that hangs it costs a kill at the deadline rather
//! than a blocked thread; and it hands over integers, which the bit depth
//! needs. The rate and the channel count come from ffprobe, not from the track
//! row, so the meter is never fed at a rate the decoder did not produce, and
//! nothing is resampled: a resampler's filter would draw a wall of its own.

pub mod bandwidth;

use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use tokio::{io::AsyncReadExt, process::Command};

/// The server's own generation of what `track_quality` stores: the ported
/// wall search, the bit depth and the spectrum's bands. A change to any of
/// them bumps it, and every lossless file is measured again (decision 5).
pub const QUALITY_VERSION: i64 = 1;

/// How many bands the stored spectrum has, from 0 Hz to Nyquist.
pub const SPECTRUM_BANDS: usize = 256;

/// How long ffprobe gets to name a file's rate and channels.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a spectrogram may take to draw (decision 8), at most. The route
/// uses less when the request timeout leaves less: see
/// [`spectrogram_deadline`].
pub const SPECTROGRAM_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a spectrogram may take under a request timeout of `request`.
///
/// The router's own timeout answers first otherwise, and the caller is told
/// whatever that layer says rather than the `503` the route documents. The
/// access check, the path guard and the answer keep five seconds, or half the
/// request timeout when that is shorter, so the picture always gives up
/// strictly before the request does, whatever an operator sets. A request
/// timeout too short to draw anything then costs the spectrogram and nothing
/// else: it is not a reason to refuse a setting every other route lives with.
pub fn spectrogram_deadline(request: Duration) -> Duration {
    let margin = Duration::from_secs(5).min(request / 2);
    request.saturating_sub(margin).min(SPECTROGRAM_TIMEOUT)
}

/// The picture's parameters, which are part of its `ETag`: a change here is a
/// different picture of the same file.
pub const SPECTROGRAM_FILTER: &str = "[0:a:0]showspectrumpic=s=1024x512:legend=0:scale=log";

/// What one pass over a file found.
#[derive(Debug, Clone, PartialEq)]
pub enum Measurement {
    /// The file decoded and something in it was louder than −60 dBFS.
    Measured {
        cutoff_hz: Option<f64>,
        wall_db: f64,
        effective_bits: Option<i64>,
        spectrum: Vec<u8>,
    },
    /// The file decoded and nothing in it was louder than −60 dBFS.
    Silent { effective_bits: Option<i64> },
    /// FFmpeg could not read it, or ran past its deadline.
    Unreadable(Failure),
}

/// Why a file was unreadable. Never a message: FFmpeg's own words can carry
/// the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Decode,
    Timeout,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decode => "decode",
            Self::Timeout => "timeout",
        }
    }
}

/// The FFmpeg this process runs, as far as a measurement depends on it.
#[derive(Debug, Clone)]
pub struct Decoder {
    /// SHA-256, in hex, of the whole `ffmpeg -version` output: the release,
    /// the configuration and the version of every library, as compiled and as
    /// loaded. Two builds of one release with different decoders differ here,
    /// and so does one binary over an upgraded shared `libavcodec`.
    pub digest: String,
    /// Whether FFmpeg can encode WebP; the spectrogram falls back to JPEG,
    /// which every FFmpeg encodes, when it cannot.
    pub webp: bool,
}

/// SHA-256 of an `ffmpeg -version` output, as stored in `track_quality`.
pub fn decoder_digest(version_output: &[u8]) -> String {
    hex::encode(Sha256::digest(version_output))
}

/// Asks FFmpeg which encoders it has, once at start. `ffmpeg -version` itself
/// is read by `media::check_tool`, which already runs it.
pub async fn probe_webp(ffmpeg: &Path) -> bool {
    let output = tokio::time::timeout(
        PROBE_TIMEOUT,
        Command::new(ffmpeg)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    match output {
        Ok(Ok(output)) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("libwebp")),
        _ => false,
    }
}

/// The first audio stream's rate, channel count and sample format.
struct Stream {
    rate: f64,
    channels: usize,
    float: bool,
}

async fn probe(ffprobe: &Path, path: &Path) -> Option<Stream> {
    let output = tokio::time::timeout(
        PROBE_TIMEOUT,
        crate::media::file_input(
            Command::new(ffprobe).args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=sample_rate,channels,sample_fmt",
                "-of",
                "default=noprint_wrappers=1",
            ]),
            path,
        )
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::trim)
    };
    let rate: f64 = field("sample_rate")?.parse().ok()?;
    let channels: usize = field("channels")?.parse().ok()?;
    let format = field("sample_fmt").unwrap_or("");
    if rate <= 0.0 || channels == 0 {
        return None;
    }
    Some(Stream {
        rate,
        channels,
        float: format.starts_with("flt") || format.starts_with("dbl"),
    })
}

/// Measures one file: ffprobe for its stream, then FFmpeg decoding it to
/// signed 32-bit integers on a pipe, at its own rate, on one thread.
///
/// The child is killed if this future is dropped or the deadline passes. The
/// FFT runs on a blocking thread fed by the pipe, so the async workers only
/// move bytes.
pub async fn measure(
    ffprobe: &Path,
    ffmpeg: &Path,
    path: &Path,
    deadline: Duration,
) -> Measurement {
    let started = Instant::now();
    let Some(stream) = probe(ffprobe, path).await else {
        return Measurement::Unreadable(Failure::Decode);
    };
    let remaining = deadline.saturating_sub(started.elapsed());
    let child = crate::media::file_input(
        Command::new(ffmpeg).args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
        ]),
        path,
    )
    .args([
        "-map",
        "0:a:0",
        "-f",
        "s32le",
        "-acodec",
        "pcm_s32le",
        "pipe:1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn();
    let Ok(mut child) = child else {
        return Measurement::Unreadable(Failure::Decode);
    };
    let Some(mut stdout) = child.stdout.take() else {
        return Measurement::Unreadable(Failure::Decode);
    };

    let (sender, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let channels = stream.channels;
    let rate = stream.rate;
    let meter = tokio::task::spawn_blocking(move || Accumulator::run(rate, channels, receiver));

    let decode = async {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = stdout.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            if sender.send(buffer[..read].to_vec()).await.is_err() {
                break;
            }
        }
        drop(sender);
        child.wait().await
    };
    let status = match tokio::time::timeout(remaining, decode).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return Measurement::Unreadable(Failure::Decode),
        // The child is dropped with the future above and killed with it.
        Err(_) => return Measurement::Unreadable(Failure::Timeout),
    };
    let Ok(accumulated) = meter.await else {
        return Measurement::Unreadable(Failure::Decode);
    };
    if !status.success() || accumulated.samples == 0 {
        return Measurement::Unreadable(Failure::Decode);
    }
    accumulated.into_measurement(stream.float)
}

/// What the blocking side of the pipe gathered.
struct Accumulator {
    meter: bandwidth::BandwidthMeter,
    or_bits: u32,
    samples: u64,
}

impl Accumulator {
    fn run(rate: f64, channels: usize, mut receiver: tokio::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        let mut this = Self {
            meter: bandwidth::BandwidthMeter::new(rate, channels),
            or_bits: 0,
            samples: 0,
        };
        let frame_bytes = 4 * channels;
        let mut pending: Vec<u8> = Vec::new();
        let mut frame = vec![0f32; channels];
        while let Some(chunk) = receiver.blocking_recv() {
            pending.extend_from_slice(&chunk);
            let whole = pending.len() / frame_bytes * frame_bytes;
            for bytes in pending[..whole].chunks_exact(frame_bytes) {
                for (slot, sample) in frame.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    let value = i32::from_le_bytes(*sample);
                    this.or_bits |= value as u32;
                    *slot = value as f32 / 2_147_483_648.0;
                }
                this.meter.push_frame(&frame);
                this.samples += 1;
            }
            pending.drain(..whole);
        }
        this
    }

    fn into_measurement(self, float: bool) -> Measurement {
        let effective_bits = effective_bits(self.or_bits, float);
        match self.meter.finish_with_spectrum() {
            Some((wall, db)) => Measurement::Measured {
                cutoff_hz: wall.cutoff_hz,
                wall_db: wall.wall_db,
                effective_bits,
                spectrum: quantise_spectrum(&db),
            },
            None => Measurement::Silent { effective_bits },
        }
    }
}

/// The bits the samples use, from the OR of every sample as a signed 32-bit
/// integer: everything above the lowest bit any sample sets.
///
/// `None` for a floating-point source, whose bits do not mean depth, and for
/// a file whose every sample is zero.
pub fn effective_bits(or_bits: u32, float: bool) -> Option<i64> {
    (!float && or_bits != 0).then(|| 32 - i64::from(or_bits.trailing_zeros()))
}

/// The averaged spectrum as `track_quality` stores it: [`SPECTRUM_BANDS`]
/// bands from 0 Hz to Nyquist, each the mean power of its FFT bins, in
/// half-decibels under the loudest band, down to −127.5 dB.
pub fn quantise_spectrum(db: &[f64]) -> Vec<u8> {
    let bins = db.len().max(1);
    let bands: Vec<f64> = (0..SPECTRUM_BANDS)
        .map(|band| {
            let lo = band * bins / SPECTRUM_BANDS;
            let hi = ((band + 1) * bins / SPECTRUM_BANDS).max(lo + 1).min(bins);
            let power: f64 = db[lo..hi].iter().map(|v| 10f64.powf(v / 10.0)).sum();
            10.0 * (power / (hi - lo) as f64).max(1e-30).log10()
        })
        .collect();
    let loudest = bands.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    bands
        .iter()
        .map(|level| ((loudest - level) * 2.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// The stored spectrum back in dB under the loudest band.
pub fn spectrum_db(stored: &[u8]) -> Vec<f64> {
    stored.iter().map(|&step| -f64::from(step) / 2.0).collect()
}

/// A spectrogram, drawn by FFmpeg on demand (decision 8).
pub struct Picture {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

/// Draws a file's spectrogram: WebP when FFmpeg has libwebp, JPEG otherwise.
/// `None` when FFmpeg fails or runs past `deadline`.
pub async fn spectrogram(
    ffmpeg: &Path,
    path: &Path,
    webp: bool,
    deadline: Duration,
) -> Option<Picture> {
    let mut command = Command::new(ffmpeg);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-threads",
        "1",
    ]);
    crate::media::file_input(&mut command, path).args([
        "-filter_complex",
        SPECTROGRAM_FILTER,
        "-frames:v",
        "1",
    ]);
    let content_type = if webp {
        command.args(["-c:v", "libwebp", "-quality", "70", "-f", "webp", "pipe:1"]);
        "image/webp"
    } else {
        command.args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "pipe:1"]);
        "image/jpeg"
    };
    let output = tokio::time::timeout(
        deadline,
        command
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    (output.status.success() && !output.stdout.is_empty()).then_some(Picture {
        bytes: output.stdout,
        content_type,
    })
}

#[cfg(test)]
mod tests {
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
}
