//! Placing a canvas on a track and removing it, and releasing a blob nothing
//! references any more.

use super::*;

impl DomainServices {
    /// Attaches a loop to a track, replacing whatever it had.
    ///
    /// The bytes are held in memory because the route's own body ceiling is
    /// `WAVEFLOW_CANVAS_MAX_BYTES`: a few megabytes at most, which is the whole
    /// reason this needs none of RFC-008's resumption machinery.
    pub async fn place_canvas(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        bytes: &[u8],
        origin_device_id: Option<Uuid>,
    ) -> Result<CanvasBlob, ServiceError> {
        let byte_size = i64::try_from(bytes.len()).map_err(|_| ServiceError::Invalid)?;
        if byte_size == 0 || byte_size > self.canvas.max_bytes {
            return Err(ServiceError::Invalid);
        }
        // Refused before a byte is written, so an unauthorised caller never
        // costs the operator a staging file or an `ffprobe`. It is not the
        // decision that counts, though — that one is re-read under the gate
        // below, where a role revoked while this call was deciding cannot
        // commit between the check and the write.
        self.canvas_library_for_track(user_id, track_id).await?;

        tokio::fs::create_dir_all(&self.canvas_dir)
            .await
            .map_err(|error| {
                tracing::error!(%error, "cannot create the canvas store");
                ServiceError::Unavailable
            })?;
        // Staged under a name nothing else can collide with, because the final
        // name is not known until the bytes have been read: it is the hash, and
        // the extension is whatever the probe says the container is.
        let staging = self.canvas_dir.join(format!("{}.part", Uuid::new_v4()));
        tokio::fs::write(&staging, bytes).await.map_err(|error| {
            tracing::error!(%error, "cannot stage a canvas");
            ServiceError::Unavailable
        })?;

        let outcome = self
            .identify_and_place(user_id, track_id, &staging, origin_device_id)
            .await;
        // Whatever happened, nothing of this attempt is left under a name only
        // this call knows. Two names: what arrived, and what it became once the
        // soundtrack was taken out — the second exists only sometimes, and on a
        // failing path it is exactly the one that would be left behind.
        for leftover in [staging.clone(), staging.with_extension("silent")] {
            if tokio::fs::try_exists(&leftover).await.unwrap_or(false) {
                if let Err(error) = tokio::fs::remove_file(&leftover).await {
                    tracing::warn!(%error, "a staged canvas could not be removed");
                }
            }
        }
        outcome
    }

    /// The half of [`Self::place_canvas`] that runs with a staged file in hand.
    async fn identify_and_place(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        staging: &std::path::Path,
        origin_device_id: Option<Uuid>,
    ) -> Result<CanvasBlob, ServiceError> {
        // Outside every lock: probing spawns a subprocess and reads a file, and
        // neither belongs under a mutex the rest of the server is waiting on.
        let probed = self.probe_canvas(staging).await?;
        if probed.duration_secs > f64::from(self.canvas.max_duration_secs) {
            return Err(ServiceError::Invalid);
        }
        // RFC-009 decision 9. A canvas plays over a track that is already
        // playing, so the desktop's `<video>` carries a hard-coded `muted` —
        // there is no client that could choose otherwise. Keeping the stream
        // would spend the ceiling on bytes nobody can hear, at the expense of
        // the picture those bytes could have bought.
        //
        // Stripped rather than refused: an mp4 that is otherwise perfectly good
        // must not be rejected over a stream the sender did not necessarily
        // choose to include, and plenty of encoders write an empty track by
        // default.
        let staging = if probed.has_audio {
            &self.strip_canvas_audio(staging, &probed.format).await?
        } else {
            staging
        };
        // Hashed through the scanner's own function rather than a second
        // implementation, so a canvas is named the way everything else in this
        // project is named.
        let hash = {
            let path = staging.to_path_buf();
            tokio::task::spawn_blocking(move || waveflow_core::scanner::hash_file_full(&path))
                .await
                .map_err(|_| ServiceError::Unavailable)?
                .map_err(|error| {
                    tracing::error!(%error, "cannot hash a staged canvas");
                    ServiceError::Unavailable
                })?
        };
        // What the quota is charged is what the store holds, which is not what
        // arrived when the audio has been taken out of it.
        let byte_size = match tokio::fs::metadata(staging).await {
            Ok(meta) => i64::try_from(meta.len()).map_err(|_| ServiceError::Invalid)?,
            Err(error) => {
                tracing::error!(%error, "cannot measure a staged canvas");
                return Err(ServiceError::Unavailable);
            }
        };
        let blob = CanvasBlob {
            hash,
            format: probed.format,
            byte_size,
        };

        // Scoped so the guard and this call's own `Arc` are both gone before
        // the entry is offered back to the map, on the failing paths as well as
        // the succeeding one.
        let linked = {
            let lock = self.canvas_lock(&blob.hash);
            let _held = lock.lock().await;
            async {
                // The file first. Already there means another track holds the
                // same loop, and the bytes are identical by construction —
                // rewriting them would be a write for nothing, over a file a
                // live link may be being served from.
                let final_path = self.canvas_path(&blob);
                if !tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
                    tokio::fs::rename(staging, &final_path)
                        .await
                        .map_err(|error| {
                            tracing::error!(%error, "cannot move a canvas into the store");
                            ServiceError::Unavailable
                        })?;
                }

                match self
                    .link_canvas(user_id, track_id, &blob, origin_device_id)
                    .await
                {
                    Ok(replaced) => Ok(replaced),
                    Err(error) => {
                        // The row did not happen, so the bytes must not stay
                        // unless something else already named them. Still under
                        // the lock, so no concurrent placement can have named
                        // them since.
                        self.discard_unreferenced_blob(&blob, &final_path).await;
                        Err(error)
                    }
                }
            }
            .await
        };
        self.release_canvas_lock(&blob.hash);
        let replaced = linked?;

        // A replacement is a removal too: the loop this track used to carry has
        // lost a reference, and may have lost its last. Taken after the link is
        // committed and under the old blob's own lock, never both locks at once
        // — two placements swapping each other's canvases would deadlock.
        if let Some(previous) = replaced.filter(|previous| *previous != blob.hash) {
            self.release_canvas_blob(&previous).await;
        }
        Ok(blob)
    }

    /// Writes the link and the blob row, inside one transaction under the gate.
    ///
    /// Returns the hash this track carried before, when it carried one.
    async fn link_canvas(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        blob: &CanvasBlob,
        origin_device_id: Option<Uuid>,
    ) -> Result<Option<String>, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;

        // Authoritative, unlike the check that let this call get this far.
        let row = sqlx::query(
            "SELECT t.library_id, t.full_hash, l.accepts_canvas, m.role FROM track t \
             JOIN library l ON l.id=t.library_id \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE t.id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_upload() {
            return Err(ServiceError::Forbidden);
        }
        if row.try_get::<i64, _>("accepts_canvas")? == 0 {
            return Err(ServiceError::Forbidden);
        }
        // Derived from the track inside the writing transaction and never taken
        // from the request, so nobody chooses which library they attach a track
        // to. The same thing `track_override` does.
        let library_id: String = row.try_get("library_id")?;
        // Read here rather than in a second query: it is what the event carries,
        // and it has to be the value this transaction saw.
        let full_hash: String = row.try_get("full_hash")?;

        let previous: Option<String> =
            sqlx::query_scalar("SELECT canvas_hash FROM track_canvas WHERE track_id=?")
                .bind(track_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;

        // Distinct blobs the library references, never links: an album's shared
        // loop is billed once however many tracks name it. Charging links would
        // make the price of a canvas depend on how many tracks it is attached
        // to, and a member would pay twelve times for bytes written once.
        //
        // A blob this library already holds costs nothing to attach again,
        // which is the whole of the deduplication showing up in the price.
        let already_held: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM track_canvas WHERE library_id=? AND canvas_hash=? \
             AND track_id<>?",
        )
        .bind(&library_id)
        .bind(&blob.hash)
        .bind(track_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if already_held == 0 {
            let used: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(c.byte_size), 0) FROM canvas c \
                 WHERE c.hash IN (SELECT canvas_hash FROM track_canvas WHERE library_id=?)",
            )
            .bind(&library_id)
            .fetch_one(&mut *tx)
            .await?;
            // What this track is about to stop referencing does not pay for
            // what it is about to start referencing: the old blob may still be
            // held by another track of the same library, and deciding that here
            // would be counting it twice. Placing over one's own canvas is
            // therefore charged as if it were new, which is the conservative
            // direction — the operator is never surprised by a library that
            // grew past its quota.
            let after = used
                .checked_add(blob.byte_size)
                .ok_or(ServiceError::Invalid)?;
            if after > self.canvas.library_quota_bytes {
                return Err(ServiceError::Conflict);
            }
        }

        sqlx::query(
            "INSERT INTO canvas (hash, format, byte_size, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(hash) DO NOTHING",
        )
        .bind(&blob.hash)
        .bind(&blob.format)
        .bind(blob.byte_size)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO track_canvas (track_id, library_id, canvas_hash, created_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(track_id) DO UPDATE SET canvas_hash=excluded.canvas_hash, \
               created_at=excluded.created_at",
        )
        .bind(track_id.to_string())
        .bind(&library_id)
        .bind(&blob.hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        self.announce_canvas_change(
            &mut tx,
            &library_id,
            track_id,
            &full_hash,
            origin_device_id,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(previous)
    }

    /// Tells the library feed that a track changed, in the same transaction.
    ///
    /// RFC-009 decision 7: the blob itself is content-addressed, and the bytes
    /// behind a hash never change, so there is no event for it. Only the
    /// **link** is a change, and a link is part of the track — so it travels in
    /// the track's own `upsert`, by the same path a tag correction takes.
    ///
    /// The payload does not grow either. It carries `full_hash` because nothing
    /// else does: a track keeps its id while its bytes move, and the event is
    /// the only witness. A canvas link is not in that position — it is read off
    /// the track, which the client refetches on receiving this anyway. Adding a
    /// field would make the payload a partial projection of the track, a second
    /// model to keep in agreement with the first.
    async fn announce_canvas_change(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        library_id: &str,
        track_id: Uuid,
        full_hash: &str,
        origin_device_id: Option<Uuid>,
        now: i64,
    ) -> Result<(), ServiceError> {
        crate::catalog::record_library_event(
            tx,
            crate::catalog::LibraryChange {
                library_id: parse_uuid(library_id.to_owned())?,
                entity_type: "track",
                entity_id: track_id,
                action: "upsert",
                payload: serde_json::json!({ "full_hash": full_hash }),
                changed_at: now,
                // So the client that just placed the canvas does not read it
                // back off the feed as something it has to go and fetch.
                origin_device_id,
            },
        )
        .await?;
        Ok(())
    }

    /// Removes the loop a track carries.
    pub async fn remove_canvas(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        origin_device_id: Option<Uuid>,
    ) -> Result<(), ServiceError> {
        let removed = {
            let now = now_ms();
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let row = sqlx::query(
                "SELECT t.library_id, t.full_hash, m.role FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE t.id=? AND m.user_id=?",
            )
            .bind(track_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ServiceError::NotFound)?;
            let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
                .map_err(|_| ServiceError::Invalid)?;
            if !role.may_upload() {
                return Err(ServiceError::Forbidden);
            }
            let library_id: String = row.try_get("library_id")?;
            let full_hash: String = row.try_get("full_hash")?;
            // Deliberately not gated on `accepts_canvas`. Closing a library to
            // new files must not strand what it already holds, and taking
            // something away never spends the operator's disk.
            let hash: Option<String> = sqlx::query_scalar(
                "DELETE FROM track_canvas WHERE track_id=? RETURNING canvas_hash",
            )
            .bind(track_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            // Announced only when something was actually taken away. A removal
            // that found nothing changed nothing, and an event for it would
            // send every client to refetch a track that is as it was.
            let hash = hash.ok_or(ServiceError::NotFound)?;
            self.announce_canvas_change(
                &mut tx,
                &library_id,
                track_id,
                &full_hash,
                origin_device_id,
                now,
            )
            .await?;
            tx.commit().await?;
            hash
        };
        self.release_canvas_blob(&removed).await;
        Ok(())
    }

    /// Erases a blob once nothing references it any more.
    ///
    /// The count, the row and the unlink are one decision taken under the
    /// blob's lock, so two removals cannot each conclude that a reference
    /// remains and leave bytes nothing names.
    ///
    /// Failing here is not an error the caller can act on: the link is already
    /// gone, which is what they asked for. What it does leave is bytes nothing
    /// names, and [`DomainServices::sweep_canvas_store`] is what collects those
    /// — the store is content-addressed and therefore enumerable, which is what
    /// made the sweep writable at all.
    pub(in crate::services) async fn release_canvas_blob(&self, hash: &str) {
        // Scoped for the same reason as the placing path: the entry goes back
        // to the map only once this call holds nothing.
        {
            let lock = self.canvas_lock(hash);
            let _held = lock.lock().await;
            async {
                let doomed = match self.forget_unreferenced_blob(hash).await {
                    Ok(doomed) => doomed,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "cannot decide whether a canvas is still referenced"
                        );
                        return;
                    }
                };
                let Some(blob) = doomed else { return };
                let path = self.canvas_path(&blob);
                // After the commit, never before: the other order leaves a row
                // naming an absent file the moment the transaction fails.
                if let Err(error) = tokio::fs::remove_file(&path).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(%error, "an unreferenced canvas is left in the store");
                    }
                }
            }
            .await;
        }
        self.release_canvas_lock(hash);
    }

    /// Drops the `canvas` row when the last link to it is gone, and says which
    /// bytes that leaves to erase.
    async fn forget_unreferenced_blob(
        &self,
        hash: &str,
    ) -> Result<Option<CanvasBlob>, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM track_canvas WHERE canvas_hash=?")
                .bind(hash)
                .fetch_one(&mut *tx)
                .await?;
        if remaining > 0 {
            return Ok(None);
        }
        // The row describes a blob; once nothing names the blob it lies, so it
        // goes rather than being marked. A "describes a file that is gone"
        // state would propagate into every read for the benefit of none.
        let row = sqlx::query("DELETE FROM canvas WHERE hash=? RETURNING hash, format, byte_size")
            .bind(hash)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok::<_, sqlx::Error>(CanvasBlob {
                hash: row.try_get("hash")?,
                format: row.try_get("format")?,
                byte_size: row.try_get("byte_size")?,
            })
        })
        .transpose()
        .map_err(Into::into)
    }

    /// Removes bytes a failed placement wrote, unless something else names them.
    ///
    /// Only ever called while holding the blob's lock, which is what makes the
    /// question answerable at all: without it, the answer could change between
    /// asking and acting.
    async fn discard_unreferenced_blob(&self, blob: &CanvasBlob, path: &std::path::Path) {
        let referenced =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM track_canvas WHERE canvas_hash=?")
                .bind(&blob.hash)
                .fetch_one(self.db.pool())
                .await;
        match referenced {
            Ok(0) => {
                if let Err(error) = tokio::fs::remove_file(path).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(%error, "a rejected canvas is left in the store");
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "cannot decide whether a rejected canvas is referenced");
            }
        }
    }
}
