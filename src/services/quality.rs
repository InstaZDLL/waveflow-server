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

impl DomainServices {
    /// Places the FFmpeg this process measures with, and deletes every
    /// measurement another generation or another FFmpeg made (decision 5).
    /// Runs once at boot, before anything reads `track_quality`.
    pub async fn use_quality_decoder(
        &self,
        decoder: quality::Decoder,
    ) -> Result<u64, ServiceError> {
        let removed = {
            let _writer = self.db.writer_guard().await;
            sqlx::query("DELETE FROM track_quality WHERE version <> ? OR decoder <> ?")
                .bind(quality::QUALITY_VERSION)
                .bind(&decoder.digest)
                .execute(self.db.pool())
                .await?
                .rows_affected()
        };
        *self
            .quality
            .0
            .decoder
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(decoder);
        if removed > 0 {
            tracing::info!(
                removed,
                "quality measurements from another analyser or FFmpeg will be taken again"
            );
        }
        Ok(removed)
    }

    /// Starts the background pass, unless the operator turned it off.
    pub fn spawn_quality_analyzer(&self) {
        if self.quality.0.parallelism == 0 {
            return;
        }
        let services = self.clone();
        tokio::spawn(async move {
            let mut scans = services.scanner.scans_in_flight();
            loop {
                if let Err(error) = services.analyze_pending_quality().await {
                    tracing::warn!(error = %error, "the quality analysis stopped early");
                }
                tokio::select! {
                    () = services.quality.0.wake.notified() => {}
                    () = scan_finished(&mut scans) => {}
                }
            }
        });
    }

    /// Measures every available lossless file with nothing current, then
    /// returns how many it measured. The background pass is this in a loop; a
    /// test calls it directly.
    pub async fn analyze_pending_quality(&self) -> Result<usize, ServiceError> {
        let parallelism = self.quality.0.parallelism;
        let Some(decoder) = self.quality.decoder() else {
            return Ok(0);
        };
        if parallelism == 0 {
            return Ok(0);
        }
        let _passing = Passing::begin(&self.quality);
        let mut measured = 0;
        // Keyset rather than offset: rows written behind the cursor drop out
        // of the query, and a file that keeps failing to be written cannot
        // hold the pass in place.
        let mut after: (i64, String) = (i64::MIN, String::new());
        loop {
            let page = self.pending_quality(&after).await?;
            let Some(last) = page.last() else {
                break;
            };
            after = (last.created_at, last.track_id.clone());
            // Each measurement is written as soon as it finishes, so the count
            // a reader watches moves file by file and a restart loses at most
            // the files in flight. One that cannot be written costs that file
            // and not the pass: it is still pending, and the next pass asks
            // for it again.
            let mut finished = stream::iter(page)
                .map(|pending| self.measure_one(pending))
                .buffer_unordered(parallelism);
            while let Some(done) = finished.next().await {
                let Some((done, _measuring)) = done else {
                    continue;
                };
                match self.write_measurements(&decoder, vec![done]).await {
                    Ok(()) => measured += 1,
                    Err(error) => {
                        tracing::warn!(error = %error, "a quality measurement was not written");
                    }
                }
            }
        }
        Ok(measured)
    }

    /// One page of the files with nothing current, oldest first.
    async fn pending_quality(&self, after: &(i64, String)) -> Result<Vec<Pending>, ServiceError> {
        let rows = sqlx::query(concat!(
            "SELECT t.id, t.library_id, t.full_hash, l.root_path, t.relative_path, \
               t.duration_ms, t.created_at \
             FROM track t JOIN library l ON l.id = t.library_id",
            quality_join!(),
            " WHERE t.is_available = 1 AND ",
            waveflow_core::lossless_track_sql!(),
            " AND ta.track_id IS NULL AND (t.created_at, t.id) > (?, ?) \
             ORDER BY t.created_at, t.id LIMIT ?"
        ))
        .bind(after.0)
        .bind(&after.1)
        .bind(PAGE)
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                let library_id: String = row.try_get("library_id")?;
                Ok(Pending {
                    track_id: row.try_get("id")?,
                    library_id: Uuid::parse_str(&library_id)
                        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
                    full_hash: row.try_get("full_hash")?,
                    root_path: row.try_get("root_path")?,
                    relative_path: row.try_get("relative_path")?,
                    duration_ms: row.try_get("duration_ms")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(ServiceError::from)
    }

    /// Measures one file once no scan is running. `None` when the file cannot
    /// be reached at all: a drive that is not mounted says nothing about the
    /// file, so nothing is recorded and the next pass asks again.
    ///
    /// The file counts as in flight for its library until the caller drops
    /// the guard handed back with it, which is once its row is written.
    async fn measure_one(&self, pending: Pending) -> Option<(Measured, Counting)> {
        // In flight from the moment the pass picks the file, before any wait:
        // a scan the caller cannot see must change nothing they are shown,
        // and a file that stopped counting while it waited would say a scan
        // had started somewhere.
        let measuring = Counting::measuring(&self.quality, pending.library_id);
        let mut scans = self.scanner.scans_in_flight();
        if *scans.borrow_and_update() > 0 {
            let _waiting = Counting::waiting(&self.quality);
            let _ = scans.wait_for(|running| *running == 0).await;
        }
        let path = crate::media::secure_track_path(
            std::path::Path::new(&pending.root_path),
            &pending.relative_path,
        )
        .await
        .ok()?;
        let deadline = (Duration::from_secs(60)
            + Duration::from_millis(u64::try_from(pending.duration_ms).unwrap_or(0)))
        .min(MAX_DEADLINE);
        let measurement =
            quality::measure(&self.ffprobe_path, &self.ffmpeg_path, &path, deadline).await;
        Some((
            Measured {
                track_id: pending.track_id,
                full_hash: pending.full_hash,
                measurement,
            },
            measuring,
        ))
    }

    /// Writes measurements in one transaction, under the writer gate. A row
    /// is written only while the track still holds the content it measured.
    async fn write_measurements(
        &self,
        decoder: &quality::Decoder,
        batch: Vec<Measured>,
    ) -> Result<(), ServiceError> {
        if batch.is_empty() {
            return Ok(());
        }
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        for measured in batch {
            let (outcome, cutoff_hz, wall_db, effective_bits, spectrum, failure) =
                match measured.measurement {
                    Measurement::Measured {
                        cutoff_hz,
                        wall_db,
                        effective_bits,
                        spectrum,
                    } => (
                        "measured",
                        cutoff_hz,
                        Some(wall_db),
                        effective_bits,
                        Some(spectrum),
                        None,
                    ),
                    Measurement::Silent { effective_bits } => {
                        ("silent", None, None, effective_bits, None, None)
                    }
                    Measurement::Unreadable(failure) => {
                        ("unreadable", None, None, None, None, Some(failure.as_str()))
                    }
                };
            sqlx::query(
                "INSERT INTO track_quality (track_id, full_hash, version, decoder, outcome, \
                   cutoff_hz, wall_db, effective_bits, spectrum, failure, analyzed_at) \
                 SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ? \
                 WHERE EXISTS (SELECT 1 FROM track WHERE id = ? AND full_hash = ?) \
                 ON CONFLICT (track_id) DO UPDATE SET full_hash = excluded.full_hash, \
                   version = excluded.version, decoder = excluded.decoder, \
                   outcome = excluded.outcome, cutoff_hz = excluded.cutoff_hz, \
                   wall_db = excluded.wall_db, effective_bits = excluded.effective_bits, \
                   spectrum = excluded.spectrum, failure = excluded.failure, \
                   analyzed_at = excluded.analyzed_at",
            )
            .bind(&measured.track_id)
            .bind(&measured.full_hash)
            .bind(quality::QUALITY_VERSION)
            .bind(&decoder.digest)
            .bind(outcome)
            .bind(cutoff_hz)
            .bind(wall_db)
            .bind(effective_bits)
            .bind(spectrum)
            .bind(failure)
            .bind(now)
            .bind(&measured.track_id)
            .bind(&measured.full_hash)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// A library's counts per verdict, and how far the analysis has got. Any
    /// member may read it (decision 7).
    pub async fn quality_summary(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<QualitySummary, ServiceError> {
        self.require_member(user_id, library_id).await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(concat!(
            "SELECT verdict, COUNT(*) FROM (SELECT ",
            quality_verdict_sql!(),
            " AS verdict FROM track t",
            quality_join!(),
            " WHERE t.library_id = ? AND t.is_available = 1) GROUP BY verdict"
        ))
        .bind(library_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let count = |verdict: QualityVerdict| {
            rows.iter()
                .find(|(name, _)| name == verdict.as_str())
                .map_or(0, |(_, count)| *count)
        };
        let not_lossless = count(QualityVerdict::Lossy) + count(QualityVerdict::NotMeasured);
        let total: i64 = rows.iter().map(|(_, count)| count).sum();
        let lossless = total - not_lossless;
        let analysis = self
            .quality_analysis(user_id, library_id, count(QualityVerdict::Pending))
            .await?;
        Ok(QualitySummary {
            enabled: self.quality.0.parallelism > 0,
            lossless,
            measured: lossless - count(QualityVerdict::Pending),
            analysis,
            verdicts: QualityVerdict::ALL
                .into_iter()
                .map(|verdict| QualityVerdictCount {
                    verdict,
                    count: count(verdict),
                })
                .collect(),
        })
    }

    /// One page of a library's files with their verdicts, worst first, then
    /// lowest cutoff first. Any member may read it.
    pub async fn quality_tracks(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        verdict: Option<QualityVerdict>,
        codec: Option<&str>,
        page: BrowsePage,
    ) -> Result<Vec<QualityTrack>, ServiceError> {
        self.require_member(user_id, library_id).await?;
        let verdict = verdict.map(QualityVerdict::as_str);
        let rows = sqlx::query(concat!(
            "SELECT * FROM (SELECT s.*, ",
            quality_verdict_sql!(),
            " AS q_verdict, ta.cutoff_hz AS q_cutoff_hz, ta.wall_db AS q_wall_db, \
               ta.effective_bits AS q_effective_bits, ta.analyzed_at AS q_analyzed_at \
             FROM (",
            song_select!(),
            " AND t.library_id = ?) AS s JOIN track t ON t.id = s.id",
            quality_join!(),
            ") AS q WHERE (? IS NULL OR q.q_verdict = ?) \
               AND (? IS NULL OR q.codec = ? COLLATE NOCASE) \
             ORDER BY ",
            verdict_rank_sql!("q.q_verdict"),
            ", q.q_cutoff_hz NULLS LAST, q.title COLLATE NOCASE, q.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .bind(verdict)
        .bind(verdict)
        .bind(codec)
        .bind(codec)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?;
        let mut qualities = Vec::with_capacity(rows.len());
        let mut songs = Vec::with_capacity(rows.len());
        for row in rows {
            qualities.push(quality_from_row(&row, "q_")?);
            songs.push(song_from_row(row)?);
        }
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs
            .into_iter()
            .zip(qualities)
            .map(|(song, quality)| QualityTrack { song, quality })
            .collect())
    }

    /// One track's verdict, its numbers and its averaged spectrum.
    pub async fn track_quality(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<TrackQualityDetail, ServiceError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            quality_verdict_sql!(),
            " AS q_verdict, ta.cutoff_hz AS q_cutoff_hz, ta.wall_db AS q_wall_db, \
               ta.effective_bits AS q_effective_bits, ta.analyzed_at AS q_analyzed_at, \
               ta.spectrum AS q_spectrum, t.sample_rate \
             FROM track t JOIN library_member m ON m.library_id = t.library_id",
            quality_join!(),
            " WHERE m.user_id = ? AND t.id = ? AND t.is_available = 1"
        ))
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let spectrum: Option<Vec<u8>> = row.try_get("q_spectrum")?;
        Ok(TrackQualityDetail {
            quality: quality_from_row(&row, "q_")?,
            spectrum: spectrum.as_deref().map(quality::spectrum_db),
        })
    }

    /// Asks for a library to be measured again from scratch (decision 5).
    /// Owners and managers only; anyone else gets the `404` of a foreign
    /// library.
    pub async fn request_quality_analysis(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<(), ServiceError> {
        {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            curated_library(&mut tx, user_id, library_id).await?;
            sqlx::query(
                "DELETE FROM track_quality WHERE track_id IN \
                 (SELECT id FROM track WHERE library_id = ?)",
            )
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        self.quality.0.wake.notify_one();
        Ok(())
    }

    /// A track's spectrogram, drawn on demand (decision 8).
    ///
    /// Access is checked first, on every request: before a `304`, so a
    /// browser's copy cannot outlive a membership, and before the bound, so an
    /// account that may not read the track gets `404` and never learns from a
    /// `429` that something is there.
    pub async fn track_spectrogram(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        if_none_match: Option<&str>,
    ) -> Result<Spectrogram, ServiceError> {
        let (root_path, relative_path, full_hash): (String, String, String) = sqlx::query_as(
            "SELECT l.root_path, t.relative_path, t.full_hash FROM track t \
             JOIN library l ON l.id = t.library_id \
             JOIN library_member m ON m.library_id = t.library_id \
             WHERE m.user_id = ? AND t.id = ? AND t.is_available = 1",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let webp = self.quality.decoder().is_some_and(|decoder| decoder.webp);
        let etag = format!(
            "\"{}\"",
            &hex::encode(Sha256::digest(format!(
                "{full_hash}|{}|{webp}",
                quality::SPECTROGRAM_FILTER
            )))[..32]
        );
        if if_none_match.is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim().trim_start_matches("W/") == etag)
        }) {
            return Ok(Spectrogram::NotModified { etag });
        }
        let per_user = self
            .quality
            .0
            .spectrogram_users
            .entry(user_id)
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone();
        let _mine = per_user
            .try_acquire_owned()
            .map_err(|_| ServiceError::Busy)?;
        let _slot = Arc::clone(&self.quality.0.spectrograms)
            .try_acquire_owned()
            .map_err(|_| ServiceError::Busy)?;
        let path =
            crate::media::secure_track_path(std::path::Path::new(&root_path), &relative_path)
                .await
                .map_err(|_| ServiceError::Unavailable)?;
        let picture = quality::spectrogram(
            &self.ffmpeg_path,
            &path,
            webp,
            self.quality.0.spectrogram_deadline,
        )
        .await
        .ok_or(ServiceError::Unavailable)?;
        Ok(Spectrogram::Picture {
            etag,
            bytes: picture.bytes,
            content_type: picture.content_type,
        })
    }

    /// Where the analysis stands for one library, as its member may read it.
    ///
    /// A scan holds every measurement back, of every library, but the page
    /// says "waiting for the scan" only when the caller can see a library being
    /// scanned. A scan of somebody else's library is not something to confirm
    /// to them, so it changes nothing they are shown: the pass still reads as
    /// running, and its file as in flight.
    async fn quality_analysis(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        pending: i64,
    ) -> Result<QualityAnalysis, ServiceError> {
        let last_measured_at: Option<i64> = sqlx::query_scalar(concat!(
            "SELECT MAX(ta.analyzed_at) FROM track t",
            quality_join!(),
            " WHERE t.library_id = ?"
        ))
        .bind(library_id.to_string())
        .fetch_one(self.db.pool())
        .await?;
        let in_flight = self
            .quality
            .0
            .measuring
            .get(&library_id)
            .map_or(0, |count| *count) as i64;
        let pass_started_at = self.quality.pass().started_at;
        let waiting = self.quality.0.waiting.load(Ordering::SeqCst) > 0;
        let state = if self.quality.0.parallelism == 0 {
            AnalysisState::Off
        } else if pending == 0 && in_flight == 0 {
            AnalysisState::Idle
        } else if self.scanner.is_scanning(library_id)
            || (waiting && self.sees_a_scan(user_id).await?)
        {
            AnalysisState::WaitingForScan
        } else if pass_started_at.is_some() {
            // Whatever `waiting` says: a pass held back by a scan the caller
            // cannot see reads exactly as one that is not, so the state never
            // tells them that somebody else's library is being scanned.
            AnalysisState::Running
        } else {
            AnalysisState::Queued
        };
        Ok(QualityAnalysis {
            state,
            in_flight,
            pass_started_at: pass_started_at.filter(|_| state == AnalysisState::Running),
            last_measured_at,
        })
    }

    /// Whether a library the caller belongs to is being scanned.
    async fn sees_a_scan(&self, user_id: Uuid) -> Result<bool, ServiceError> {
        let libraries: Vec<String> =
            sqlx::query_scalar("SELECT library_id FROM library_member WHERE user_id = ?")
                .bind(user_id.to_string())
                .fetch_all(self.db.pool())
                .await?;
        Ok(libraries.iter().any(|library| {
            Uuid::parse_str(library).is_ok_and(|library| self.scanner.is_scanning(library))
        }))
    }

    /// The caller is a member of the library, in any role.
    async fn require_member(&self, user_id: Uuid, library_id: Uuid) -> Result<(), ServiceError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM library_member WHERE user_id = ? AND library_id = ?",
        )
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .map(|_| ())
        .ok_or(ServiceError::NotFound)
    }
}

/// Resolves when the number of running scans falls back to zero after having
/// changed; never, once the scanner is gone.
async fn scan_finished(scans: &mut tokio::sync::watch::Receiver<usize>) {
    loop {
        if scans.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        if *scans.borrow_and_update() == 0 {
            return;
        }
    }
}

fn quality_from_row(
    row: &sqlx::sqlite::SqliteRow,
    prefix: &str,
) -> Result<TrackQuality, sqlx::Error> {
    let verdict: String = row.try_get(format!("{prefix}verdict").as_str())?;
    let sample_rate: Option<i64> = row.try_get("sample_rate")?;
    Ok(TrackQuality {
        verdict: QualityVerdict::parse(&verdict).ok_or_else(|| {
            sqlx::Error::Decode(format!("unknown quality verdict {verdict:?}").into())
        })?,
        cutoff_hz: row.try_get(format!("{prefix}cutoff_hz").as_str())?,
        wall_db: row.try_get(format!("{prefix}wall_db").as_str())?,
        nyquist_hz: sample_rate.map(|rate| rate as f64 / 2.0),
        effective_bits: row.try_get(format!("{prefix}effective_bits").as_str())?,
        analyzed_at: row.try_get(format!("{prefix}analyzed_at").as_str())?,
    })
}
