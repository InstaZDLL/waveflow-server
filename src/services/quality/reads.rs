//! What an account is shown of the measurements: the summary, the tracks by
//! verdict, one track's detail and its spectrogram.

use super::*;

impl DomainServices {
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
        let (root_path, relative_path, full_hash, follow_links, other_roots): (
            String,
            String,
            String,
            i64,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT l.root_path, t.relative_path, t.full_hash, l.follow_links, (SELECT json_group_array(o.root_path) FROM library o WHERE o.id <> l.id) AS other_roots FROM track t \
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
        let path = crate::media::secure_track_path(
            std::path::Path::new(&root_path),
            &relative_path,
            &self.link_roots,
            &crate::link_roots::LinkPolicy::from_columns(follow_links, other_roots.as_deref()),
        )
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
