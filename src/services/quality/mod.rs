//! The real quality of a file (RFC-016): which files are measured and when,
//! what a measurement means to a reader, and who may read it.
//!
//! **Never on the answering path** (decision 4). Every read answers from what
//! `track_quality` holds, and a file with nothing current there is `pending`.
//! The background pass measures the available lossless files with nothing
//! current, oldest first, at most `WAVEFLOW_QUALITY_PARALLELISM` at a time, and
//! starts no file while a scan runs. It wakes at boot, when a scan finishes,
//! and when an owner or manager asks for a library to be analysed again.
//!
//! **The verdict is computed where it is read** (decision 3), in one SQL
//! expression, [`quality_verdict_sql!`]. It reuses the core's
//! `lossless_track_sql!` and `probable_fake_lossless_sql!` verbatim, so the
//! server and the desktop cannot disagree about a fake lossless file.
//!
//! **A measurement is of a content, by a version, through a decoder**
//! (decision 5). Rows written by another `QUALITY_VERSION` or another FFmpeg
//! are deleted at boot, before anything reads them, which is what lets every
//! query here call a row current on its `full_hash` alone.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};

use futures_util::{stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, Semaphore};

use super::credit_decisions::curated_library;
use super::*;
use crate::quality::{self, Measurement};

/// Whether the track aliased `t` is in a lossy format: the formats the
/// scanner names that throw something away. AAC with a bit depth is ALAC,
/// which the core counts as lossless.
macro_rules! lossy_track_sql {
    () => {
        "(t.codec IN ('MP3', 'Vorbis', 'Opus', 'Speex') \
          OR (t.codec = 'AAC' AND t.bit_depth IS NULL))"
    };
}

/// Every current measurement of the track aliased `t`, aliased `ta`. A row
/// whose hash is not the track's describes a file that has gone.
macro_rules! quality_join {
    () => {
        " LEFT JOIN track_quality ta ON ta.track_id = t.id AND ta.full_hash = t.full_hash"
    };
}

/// The verdict of the track aliased `t`, over [`quality_join!`]. The first
/// that applies, in the order of RFC-016 decision 3.
///
/// `upsampled` stops at 30 kHz rather than at a 48 kHz source's 24 kHz
/// Nyquist: a resampler's filter does not stop dead there. Measured on
/// FFmpeg's own, the wall stands at 21.5 to 23.8 kHz with a steep filter and
/// at 25.7 kHz (from 44.1 kHz) or 28.0 kHz (from 48 kHz) with its default one.
macro_rules! quality_verdict_sql {
    () => {
        concat!(
            "(CASE WHEN ",
            lossy_track_sql!(),
            " THEN 'lossy' WHEN NOT ",
            waveflow_core::lossless_track_sql!(),
            " THEN 'not_measured' \
              WHEN ta.track_id IS NULL THEN 'pending' \
              WHEN ta.outcome = 'unreadable' THEN 'unreadable' \
              WHEN ta.outcome = 'silent' THEN 'silent' \
              WHEN ",
            waveflow_core::probable_fake_lossless_sql!(),
            " THEN 'lossy_source' \
              WHEN t.sample_rate > 48000 AND ta.cutoff_hz IS NOT NULL \
                AND ta.cutoff_hz <= 30000 THEN 'upsampled' \
              WHEN ta.effective_bits IS NOT NULL AND t.bit_depth IS NOT NULL \
                AND ta.effective_bits < t.bit_depth THEN 'padded' \
              ELSE 'consistent' END)"
        )
    };
}

/// Worst first: the order the page lists in (decision 10).
macro_rules! verdict_rank_sql {
    ($column:literal) => {
        concat!(
            "(CASE ",
            $column,
            " WHEN 'lossy_source' THEN 0 WHEN 'upsampled' THEN 1 WHEN 'padded' THEN 2 \
              WHEN 'unreadable' THEN 3 WHEN 'silent' THEN 4 WHEN 'pending' THEN 5 \
              WHEN 'consistent' THEN 6 WHEN 'lossy' THEN 7 ELSE 8 END)"
        )
    };
}

/// What the audio says about a file, against what its container claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QualityVerdict {
    /// A wall where a lossy encoder puts one: probably made from a lossy file.
    LossySource,
    /// Above 48 kHz, with a wall where a resampler from 44.1 or 48 kHz leaves
    /// one.
    Upsampled,
    /// Fewer bits used than the container declares.
    Padded,
    /// FFmpeg could not read it, or ran past its deadline.
    Unreadable,
    /// Nothing louder than −60 dBFS.
    Silent,
    /// Lossless, with no measurement of this content yet.
    Pending,
    /// No sign of a lossy source. Not "genuine": an encoder that keeps the
    /// full band leaves nothing to find.
    Consistent,
    /// A lossy format, which is not measured.
    Lossy,
    /// DSD, or a format neither list knows.
    NotMeasured,
}

impl QualityVerdict {
    /// Worst first, as the page lists them.
    pub const ALL: [Self; 9] = [
        Self::LossySource,
        Self::Upsampled,
        Self::Padded,
        Self::Unreadable,
        Self::Silent,
        Self::Pending,
        Self::Consistent,
        Self::Lossy,
        Self::NotMeasured,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LossySource => "lossy_source",
            Self::Upsampled => "upsampled",
            Self::Padded => "padded",
            Self::Unreadable => "unreadable",
            Self::Silent => "silent",
            Self::Pending => "pending",
            Self::Consistent => "consistent",
            Self::Lossy => "lossy",
            Self::NotMeasured => "not_measured",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|verdict| verdict.as_str() == value)
    }
}

/// How many files of a library carry one verdict.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QualityVerdictCount {
    pub verdict: QualityVerdict,
    pub count: i64,
}

/// A library's quality at a glance (decision 10's header).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QualitySummary {
    /// Whether this server measures at all; `false` when the operator set
    /// `WAVEFLOW_QUALITY_PARALLELISM=0`.
    pub enabled: bool,
    /// Available lossless files, the ones the analysis is about.
    pub lossless: i64,
    /// Of those, how many have a current measurement.
    pub measured: i64,
    /// Every verdict, worst first, zero counts included.
    pub verdicts: Vec<QualityVerdictCount>,
    /// Where the analysis stands for this library.
    pub analysis: QualityAnalysis,
}

/// What the analyser is doing about one library's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisState {
    /// The operator turned the analysis off.
    Off,
    /// Nothing of this library is waiting, and nothing is being measured.
    Idle,
    /// Files are waiting and no pass is running yet: the analyser is between
    /// passes, and the next wake reaches them.
    Queued,
    /// Files are waiting, and a scan the caller can see holds the analysis
    /// back until it finishes.
    WaitingForScan,
    /// A pass is running. One held back by a scan the caller cannot see is
    /// still `running` to them: that scan is not theirs to know about.
    Running,
}

/// The analyser's state, as one library's member may read it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QualityAnalysis {
    pub state: AnalysisState,
    /// How many of this library's files are being decoded right now.
    pub in_flight: i64,
    /// When the running pass began; `null` unless one is running.
    pub pass_started_at: Option<i64>,
    /// When a file of this library was last measured; `null` before any was.
    pub last_measured_at: Option<i64>,
}

/// One file's verdict and the numbers behind it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackQuality {
    pub verdict: QualityVerdict,
    /// Where the wall stands, in hertz; `null` with no wall or no measurement.
    pub cutoff_hz: Option<f64>,
    /// How far the level falls across the steepest candidate, in dB.
    pub wall_db: Option<f64>,
    /// Half the sample rate: the most the file could hold.
    pub nyquist_hz: Option<f64>,
    /// How many bits the samples use; `null` for floating point.
    pub effective_bits: Option<i64>,
    pub analyzed_at: Option<i64>,
}

/// A song item of the quality list, with its verdict.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QualityTrack {
    #[serde(flatten)]
    pub song: SongItem,
    pub quality: TrackQuality,
}

/// One track's quality, with the averaged spectrum it was judged on.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackQualityDetail {
    #[serde(flatten)]
    pub quality: TrackQuality,
    /// 256 bands from 0 Hz to Nyquist, in dB under the loudest one; `null`
    /// when nothing was measured or the file was silent.
    pub spectrum: Option<Vec<f64>>,
}

/// What the spectrogram route answers.
pub enum Spectrogram {
    /// The caller's copy is the picture this file would draw.
    NotModified { etag: String },
    Picture {
        etag: String,
        bytes: Vec<u8>,
        content_type: &'static str,
    },
}

/// The analysis's own state, shared by every clone of [`DomainServices`].
#[derive(Clone)]
pub(super) struct QualityRuntime(Arc<QualityInner>);

struct QualityInner {
    parallelism: usize,
    /// How long a spectrogram may take, kept under the request timeout.
    spectrogram_deadline: Duration,
    /// Placed at boot by [`DomainServices::use_quality_decoder`]; nothing is
    /// measured before it is.
    decoder: RwLock<Option<quality::Decoder>>,
    wake: Notify,
    /// Spectrograms in flight across the server: the analysis's own bound plus
    /// one, so a reader can have one drawn while the background pass runs.
    spectrograms: Arc<Semaphore>,
    /// One spectrogram per account at a time.
    spectrogram_users: dashmap::DashMap<Uuid, Arc<Semaphore>>,
    /// When the running pass began, if one is running, and when the last one
    /// ended.
    pass: Mutex<PassState>,
    /// How many measurements are held back by a running scan.
    waiting: AtomicUsize,
    /// How many files are being decoded right now, by library.
    measuring: dashmap::DashMap<Uuid, usize>,
}

#[derive(Default)]
struct PassState {
    started_at: Option<i64>,
}

impl QualityRuntime {
    pub(super) fn new(parallelism: usize, request_timeout: Duration) -> Self {
        Self(Arc::new(QualityInner {
            parallelism,
            spectrogram_deadline: quality::spectrogram_deadline(request_timeout),
            decoder: RwLock::new(None),
            wake: Notify::new(),
            spectrograms: Arc::new(Semaphore::new(parallelism + 1)),
            spectrogram_users: dashmap::DashMap::new(),
            pass: Mutex::new(PassState::default()),
            waiting: AtomicUsize::new(0),
            measuring: dashmap::DashMap::new(),
        }))
    }

    fn pass(&self) -> std::sync::MutexGuard<'_, PassState> {
        self.0
            .pass
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn decoder(&self) -> Option<quality::Decoder> {
        self.0
            .decoder
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

/// Marks a pass as running for as long as it lives.
struct Passing(QualityRuntime);

impl Passing {
    fn begin(runtime: &QualityRuntime) -> Self {
        runtime.pass().started_at = Some(now_ms());
        Self(runtime.clone())
    }
}

impl Drop for Passing {
    fn drop(&mut self) {
        self.0.pass().started_at = None;
    }
}

/// Counts one thing for as long as it lives: a file of a library being
/// decoded, or a measurement held back by a scan.
enum Counting {
    Measuring(QualityRuntime, Uuid),
    Waiting(QualityRuntime),
}

impl Counting {
    fn measuring(runtime: &QualityRuntime, library_id: Uuid) -> Self {
        *runtime.0.measuring.entry(library_id).or_insert(0) += 1;
        Self::Measuring(runtime.clone(), library_id)
    }

    fn waiting(runtime: &QualityRuntime) -> Self {
        runtime.0.waiting.fetch_add(1, Ordering::SeqCst);
        Self::Waiting(runtime.clone())
    }
}

impl Drop for Counting {
    fn drop(&mut self) {
        match self {
            Self::Measuring(runtime, library_id) => {
                runtime.0.measuring.remove_if_mut(library_id, |_, count| {
                    *count = count.saturating_sub(1);
                    *count == 0
                });
            }
            Self::Waiting(runtime) => {
                runtime.0.waiting.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

/// A file waiting for its measurement.
struct Pending {
    track_id: String,
    library_id: Uuid,
    full_hash: String,
    root_path: String,
    relative_path: String,
    link_policy: crate::library::link_roots::LinkPolicy,
    duration_ms: i64,
    created_at: i64,
}

/// A file measured, waiting for its row.
struct Measured {
    track_id: String,
    full_hash: String,
    measurement: Measurement,
}

/// How many files the pass asks for at a time.
const PAGE: i64 = 64;

/// The longest one file may take, whatever it says about itself. The deadline
/// is a minute plus the track's duration, and the duration is read from the
/// file's own header: without this ceiling, a file claiming ten hours that
/// hangs FFmpeg would hold a slot of the analysis for ten hours. Thirty minutes
/// is a three-hour file decoded at six times real time, a hundred times slower
/// than measured here.
const MAX_DEADLINE: Duration = Duration::from_secs(30 * 60);

mod analysis;
mod reads;
