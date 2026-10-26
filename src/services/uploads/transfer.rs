//! Moving the bytes: a session's state, receiving its fragments, and
//! committing the file it became.

use super::sweep::staging_is_gone;
use super::*;

/// What a session looks like to the code that has to act on it.
struct SessionTarget {
    library_id: Uuid,
    root: std::path::PathBuf,
    declared_hash: String,
    declared_size: i64,
    extension: String,
    next_chunk: i64,
    received_bytes: i64,
    expires_at: i64,
}

impl SessionTarget {
    /// Where the fragments accumulate: the destination directory itself, under
    /// a name carrying no audio extension.
    ///
    /// Both halves matter. Same directory means same filesystem, so the rename
    /// at commit is atomic rather than a copy with a window during which a
    /// truncated file exists. And the walk skips it not because it is hidden —
    /// nothing about the walk skips hidden directories — but because it filters
    /// on extension, which is a rule that already exists and cannot be broken
    /// by someone rewriting the traversal.
    fn staging_path(&self, session_id: Uuid) -> std::path::PathBuf {
        self.root
            .join(MANAGED_DIR)
            .join(format!("{session_id}.part"))
    }

    /// The name the file keeps, once the bytes are known to be what was
    /// promised.
    ///
    /// By hash rather than by tags: tags move — a correction, a retag, a later
    /// re-identification — and a path built from them would leave the server
    /// choosing between moving the file and keeping a name that lies. The
    /// project already answered that question in the other direction, where a
    /// correction never rewrites the file. It keeps its extension because the
    /// walk recognises a file by nothing else.
    fn relative_final(&self, hash: &str) -> String {
        format!("{MANAGED_DIR}/{hash}.{}", self.extension)
    }
}

impl DomainServices {
    /// Reads a session and re-decides, from scratch, whether it may continue.
    ///
    /// Every step re-asks: a session is not an authorisation a client carries
    /// away with it. The server already holds this rule for playback, where a
    /// ticket re-checks membership on each redemption so revoking access takes
    /// effect immediately rather than when the ticket expires. A transfer lasts
    /// far longer than a ticket and costs far more, so a member removed or a
    /// library closed mid-transfer must stop writing at the next request.
    async fn session_target(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        now: i64,
    ) -> Result<SessionTarget, ServiceError> {
        let row = sqlx::query(
            "SELECT s.library_id, s.declared_hash, s.declared_size, s.extension, \
                    s.next_chunk, s.received_bytes, s.expires_at, \
                    l.root_path, l.accepts_uploads, m.role \
             FROM upload_session s \
             JOIN library l ON l.id = s.library_id \
             JOIN library_member m ON m.library_id = s.library_id AND m.user_id = s.user_id \
             WHERE s.id = ? AND s.user_id = ?",
        )
        .bind(session_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_upload() {
            // Blurred onto 404: someone who may no longer upload learns nothing
            // about a session they may no longer act on.
            return Err(ServiceError::Forbidden);
        }
        if row.try_get::<i64, _>("expires_at")? <= now {
            return Err(ServiceError::NotFound);
        }
        if row.try_get::<i64, _>("accepts_uploads")? == 0 {
            // Not a 404. The caller is still entitled to be here — it is the
            // library that stopped taking files, and saying so is what lets a
            // client stop rather than retry.
            return Err(ServiceError::Conflict);
        }
        Ok(SessionTarget {
            library_id: parse_uuid(row.try_get("library_id")?)?,
            root: std::path::PathBuf::from(row.try_get::<String, _>("root_path")?),
            declared_hash: row.try_get("declared_hash")?,
            declared_size: row.try_get("declared_size")?,
            extension: row.try_get("extension")?,
            next_chunk: row.try_get("next_chunk")?,
            received_bytes: row.try_get("received_bytes")?,
            expires_at: row.try_get("expires_at")?,
        })
    }

    fn state_of(&self, session_id: Uuid, target: &SessionTarget) -> UploadSessionState {
        UploadSessionState {
            session_id,
            next_chunk: target.next_chunk,
            received_bytes: target.received_bytes,
            chunk_bytes: self.uploads.chunk_bytes,
            expires_at: target.expires_at,
        }
    }

    /// Where a transfer stands, so a client that restarted can ask instead of
    /// assuming.
    pub async fn upload_session_state(
        &self,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<UploadSessionState, ServiceError> {
        let target = self.session_target(user_id, session_id, now_ms()).await?;
        Ok(self.state_of(session_id, &target))
    }

    /// Takes one fragment.
    ///
    /// Three cases, and only one is an error. A fragment already written is
    /// answered idempotently rather than rejected: an acknowledgement lost
    /// after the write is what a dropped link ordinarily produces, and treating
    /// the client's honest retry as a fault would make the protocol fragile
    /// exactly where it exists to absorb interruptions. A fragment from the
    /// future is refused, because the gap it would leave is one only the final
    /// hash could reveal, far too late.
    pub async fn receive_chunk(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        index: i64,
        bytes: &[u8],
    ) -> Result<UploadSessionState, ServiceError> {
        // Resolved before any lock exists for it. `session_lock` inserts on
        // lookup, so locking first would let a caller mint a map entry for
        // every uuid they can type; resolving first bounds the entries to
        // sessions that are real and theirs.
        self.session_target(user_id, session_id, now_ms()).await?;
        let lock = self.session_lock(session_id);
        let _held = lock.lock().await;
        // Read again under the lock. The check above decided whether to take
        // the lock; this one is what the rest of the call acts on, and only it
        // is protected from a sweep or another fragment moving underneath.
        let now = now_ms();
        let target = self.session_target(user_id, session_id, now).await?;
        if index < 0 {
            return Err(ServiceError::Invalid);
        }
        if index < target.next_chunk {
            return Ok(self.state_of(session_id, &target));
        }
        if index > target.next_chunk {
            return Err(ServiceError::Conflict);
        }
        // Exactly what this position calls for, not merely no more than it. A
        // short fragment anywhere but the end would put every later one at the
        // wrong offset, and nothing would notice until the hash did.
        let remaining = target
            .declared_size
            .saturating_sub(target.received_bytes)
            .max(0);
        let expected = remaining.min(self.uploads.chunk_bytes);
        if expected == 0 || i64::try_from(bytes.len()).unwrap_or(i64::MAX) != expected {
            return Err(ServiceError::Invalid);
        }

        let staging = target.staging_path(session_id);
        if let Some(parent) = staging.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                tracing::error!(%error, "cannot create the managed directory");
                ServiceError::Unavailable
            })?;
        }
        // Written before the row moves. A fragment written twice at the same
        // offset is the same file; a row that moved without its bytes is not.
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&staging)
            .await
            .map_err(|error| {
                tracing::error!(%error, "cannot open a staging file");
                ServiceError::Unavailable
            })?;
        {
            use tokio::io::AsyncSeekExt;
            use tokio::io::AsyncWriteExt;
            file.seek(std::io::SeekFrom::Start(
                u64::try_from(target.received_bytes).unwrap_or(0),
            ))
            .await
            .map_err(|_| ServiceError::Unavailable)?;
            file.write_all(bytes)
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            file.sync_data()
                .await
                .map_err(|_| ServiceError::Unavailable)?;
        }

        let _writer = self.db.writer_guard().await;
        let received = target.received_bytes + expected;
        // Guarded on the position this call claimed: if anything moved the
        // session while the bytes were being written, this updates nothing and
        // the client is told where the session actually stands.
        let moved = sqlx::query(
            "UPDATE upload_session SET next_chunk = ?, received_bytes = ? \
             WHERE id = ? AND next_chunk = ?",
        )
        .bind(index + 1)
        .bind(received)
        .bind(session_id.to_string())
        .bind(target.next_chunk)
        .execute(self.db.pool())
        .await?
        .rows_affected()
            == 1;
        drop(_writer);
        if !moved {
            return Err(ServiceError::Conflict);
        }
        Ok(UploadSessionState {
            session_id,
            next_chunk: index + 1,
            received_bytes: received,
            chunk_bytes: self.uploads.chunk_bytes,
            expires_at: target.expires_at,
        })
    }

    /// Turns a completed transfer into a track.
    ///
    /// SQLite and the filesystem share no transaction, so the order below is a
    /// decision rather than a detail. A crash between the rename and the
    /// transaction leaves a file with no catalogue row — which is precisely
    /// what the next scan collects, since the file already carries its final
    /// name. The other order leaves a row pointing at nothing, a state nothing
    /// repairs on its own and every read runs into.
    pub async fn commit_upload(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        origin_device_id: Option<Uuid>,
    ) -> Result<CommittedUpload, ServiceError> {
        // Resolved before the lock, for the same reason as above.
        self.session_target(user_id, session_id, now_ms()).await?;
        let lock = self.session_lock(session_id);
        let _held = lock.lock().await;
        let now = now_ms();
        let target = self.session_target(user_id, session_id, now).await?;
        let staging = target.staging_path(session_id);
        if target.received_bytes != target.declared_size {
            return Err(ServiceError::Invalid);
        }

        // Recomputed, never taken on trust. The declared hash exists to avoid a
        // transfer; letting it establish an identity would let any authorised
        // member pass one file off as another, and turn the deduplication into
        // a means of substitution.
        let hashed = {
            let path = staging.clone();
            tokio::task::spawn_blocking(move || waveflow_core::scanner::hash_file_full(&path))
                .await
                .map_err(|_| ServiceError::Unavailable)?
        };
        let Ok(hash) = hashed else {
            self.abandon(session_id, &staging).await;
            return Err(ServiceError::Unavailable);
        };
        if hash != target.declared_hash {
            // What arrived is not what was promised, and nothing about it is
            // worth keeping.
            self.abandon(session_id, &staging).await;
            return Err(ServiceError::Invalid);
        }

        let relative = target.relative_final(&hash);
        let final_path = target.root.join(&relative);
        tokio::fs::rename(&staging, &final_path)
            .await
            .map_err(|error| {
                tracing::error!(%error, "cannot move a received file into place");
                ServiceError::Unavailable
            })?;

        // The extension was never proof — anything at all can be called .flac.
        // Reading the file is, and reading it through the scan's own extractor
        // is what makes what lands here identical to what a scan would have
        // written. A file that will not open is removed rather than left to
        // occupy a disk the catalogue can never show.
        let input = match self.scanner.read_track_input(&target.root, &relative).await {
            Ok(input) => input,
            Err(error) => {
                tracing::warn!(%error, "a received file is not one the catalogue can read");
                // Put back under the name this session owns, before anything
                // else is decided.
                //
                // Everything that cleans up after a session — the sweep, and
                // the rule that keeps a session whose file would not go — knows
                // one name: the staging one. A file left under the final name
                // is outside all of it, and the sweep would then find nothing,
                // call it already gone, drop the row, and leave the file with
                // nothing that remembers it.
                //
                // Teaching the sweep the final name is the obvious repair and
                // the wrong one: that name is the hash, and two members may
                // hold sessions for the same hash in one library — the unique
                // index is per member. A sweep that removed it could delete a
                // live track's file to clean up a stranger's failure. Moving
                // the file back needs no such judgement.
                let put_back = tokio::fs::rename(&final_path, &staging).await.is_ok();
                if !put_back {
                    if let Err(error) = tokio::fs::remove_file(&final_path).await {
                        tracing::error!(
                            %error,
                            file = %relative,
                            "a file the catalogue cannot read is stuck in the library"
                        );
                        // Neither moved nor removed. The session is kept, and
                        // the sweep reaches the final name too — under guards,
                        // since that name is the hash rather than this
                        // session's own — so the file does not outlive the row
                        // that remembers it.
                        return Err(ServiceError::Unavailable);
                    }
                }
                // Removes the staging file if it is back, and keeps the session
                // if it will not go — the same rule as everywhere else, which
                // is the point of putting the file back first.
                self.abandon(session_id, &staging).await;
                return Err(ServiceError::Invalid);
            }
        };

        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // A track may already sit at this name — the same bytes uploaded again
        // after their track went unavailable. Reviving it is what keeps a
        // second row from claiming a path the schema makes unique anyway.
        let existing_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND relative_path=?")
                .bind(target.library_id.to_string())
                .bind(&relative)
                .fetch_optional(&mut *tx)
                .await?;
        let decisions = crate::catalog::credit_decisions(&mut tx, target.library_id).await?;
        crate::database::Database::apply_catalog_track_in_transaction(
            &mut tx,
            self.db.pid(),
            target.library_id,
            // No scan walked for this file. The column says so rather than
            // borrowing an unrelated job's identity, and the end-of-scan sweep
            // knows what that means.
            None,
            &crate::catalog::CatalogApply {
                input,
                existing_id: existing_id.map(parse_uuid).transpose()?,
                moved: false,
                // So the client that sent this file does not read it back off
                // the feed as a track it has just discovered.
                origin_device_id,
            },
            &decisions,
            now,
        )
        .await?;
        // Deleted in the same transaction that inserts the track: the space the
        // session was holding becomes the space the file occupies, with no
        // interval during which it is neither. Releasing it and recounting
        // would leave exactly that interval, and two commits would race through
        // it.
        // Read back rather than derived: the apply owns whether this was a new
        // row or a revived one, and the path is what identifies it either way.
        let track_id: String =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND relative_path=?")
                .bind(target.library_id.to_string())
                .bind(&relative)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM upload_session WHERE id=?")
            .bind(session_id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.upload_locks.remove(&session_id);

        // The apply announced the track on the library feed itself, which is
        // the point of going through it: a received file is a catalogue change
        // like any other, and nothing here had to remember to say so.
        Ok(CommittedUpload {
            track_id: parse_uuid(track_id)?,
            full_hash: hash,
        })
    }

    /// Drops a session and whatever it had accumulated.
    ///
    /// File first, row second, like the sweep: dying in between leaves a row
    /// whose file is gone, which the next sweep repeats harmlessly.
    async fn abandon(&self, session_id: Uuid, staging: &std::path::Path) {
        if !staging_is_gone(staging, session_id).await {
            // Kept for the sweep, for the same reason it is kept there: the row
            // is what remembers the file. The session expires on its own, and
            // its reservation keeps counting the space the file is still using
            // until it does.
            return;
        }
        let _writer = self.db.writer_guard().await;
        if let Err(error) = sqlx::query("DELETE FROM upload_session WHERE id=?")
            .bind(session_id.to_string())
            .execute(self.db.pool())
            .await
        {
            tracing::warn!(%error, session = %session_id, "cannot close an abandoned session");
        }
        drop(_writer);
        self.upload_locks.remove(&session_id);
    }
}
