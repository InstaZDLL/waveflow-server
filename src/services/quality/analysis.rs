//! The background analysis: finding the files due a measurement, measuring
//! them, and writing what was measured.

use super::*;

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
               t.duration_ms, t.created_at, l.follow_links, (SELECT json_group_array(o.root_path) FROM library o WHERE o.id <> l.id) AS other_roots \
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
                    link_policy: crate::link_roots::LinkPolicy::from_columns(
                        row.try_get("follow_links")?,
                        row.try_get::<Option<String>, _>("other_roots")?.as_deref(),
                    ),
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
            &self.link_roots,
            &pending.link_policy,
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
