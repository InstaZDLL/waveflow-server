//! The upload sweeper: expired sessions, and staged or final files that are
//! gone.

use super::*;

/// Whether the file a session owned is gone — removed now, or already absent.
///
/// `false` means it is still there and the session must be kept: the row is the
/// only thing that remembers the file exists, so dropping it would leave the
/// operator's disk carrying something nothing will ever look for again. Absent
/// is success, and the ordinary case — this runs after crashes, and a session
/// that never received a fragment has no file at all.
pub(super) async fn staging_is_gone(path: &std::path::Path, session_id: Uuid) -> bool {
    match tokio::fs::remove_file(path).await {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            tracing::warn!(
                %error,
                session = %session_id,
                "cannot remove a staging file; keeping the session so the sweep retries"
            );
            false
        }
    }
}

impl DomainServices {
    /// Sweeps abandoned sessions on a schedule, so cleanup does not wait for
    /// somebody to offer another file.
    ///
    /// Without this the sweep only runs inside a negotiation. Nothing breaks —
    /// the quota is decided by that same negotiation, which sweeps first — but
    /// a client that abandons a batch of transfers and never returns leaves its
    /// staging files holding the operator's disk with nothing to signal it.
    ///
    /// Boot first, then one pass per session lifetime: an abandoned file then
    /// lives at most twice the lifetime a session was promised, which is the
    /// same order of magnitude that promise already makes. The shape is the
    /// scanner's `spawn_background`, deliberately.
    pub fn spawn_upload_sweeper(&self) {
        let services = self.clone();
        let interval = services.uploads.session_ttl;
        tokio::spawn(async move {
            services.sweep_now().await;
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_now().await;
            }
        });
    }

    async fn sweep_now(&self) {
        if let Err(error) = self.sweep_expired_sessions(now_ms()).await {
            tracing::warn!(%error, "could not sweep expired upload sessions");
        }
    }

    /// Removes what expired sessions left behind, before anything counts them.
    ///
    /// Each session is swept under its own lock, and that is not tidiness. A
    /// fragment that read its session just before it expired can be writing
    /// while this runs: delete the file and the row without the lock, and that
    /// write recreates the staging file a moment later with no row left to
    /// remember it — a file on the operator's disk nothing will ever sweep
    /// again. The lock makes the two mutually exclusive.
    ///
    /// File first, row second, and that order is deliberate too: dying between
    /// the two leaves a row whose file is already gone, which the next sweep
    /// repeats harmlessly. The other order leaves the orphan.
    ///
    /// Run before the writer gate is taken. File I/O has no business happening
    /// while the process-wide gate is held, and a session that expires during
    /// this sweep is one the next sweep gets.
    pub(super) async fn sweep_expired_sessions(&self, now: i64) -> Result<(), ServiceError> {
        let expired = sqlx::query(
            "SELECT s.id, s.library_id, s.declared_hash, s.extension, l.root_path \
             FROM upload_session s \
             JOIN library l ON l.id = s.library_id \
             WHERE s.expires_at <= ?",
        )
        .bind(now)
        .fetch_all(self.db.pool())
        .await?;
        for row in &expired {
            let session_id = parse_uuid(row.try_get::<String, _>("id")?)?;
            let library_id: String = row.try_get("library_id")?;
            let declared_hash: String = row.try_get("declared_hash")?;
            let extension: String = row.try_get("extension")?;
            let root: String = row.try_get("root_path")?;
            let path = staging_path(std::path::Path::new(&root), session_id);

            let lock = self.session_lock(session_id);
            let held = lock.lock().await;
            // A commit that got as far as renaming and could neither move the
            // file back nor remove it leaves one under the final name. Nothing
            // else would ever look there, so the sweep does — under two guards,
            // because that name is the hash and is not this session's to claim
            // on its own.
            if !self
                .final_file_is_gone(&library_id, &declared_hash, &extension, &root, session_id)
                .await?
            {
                continue;
            }
            if !staging_is_gone(&path, session_id).await {
                // The row stays, and so does the lock entry. Deleting either
                // would discard the only record that this file exists — the
                // order of file-then-row was chosen so a failure could be
                // retried, and a failure that deletes the row anyway is not a
                // retry, it is the orphan the order was avoiding.
                continue;
            }
            {
                let _writer = self.db.writer_guard().await;
                sqlx::query("DELETE FROM upload_session WHERE id=? AND expires_at <= ?")
                    .bind(session_id.to_string())
                    .bind(now)
                    .execute(self.db.pool())
                    .await?;
            }
            // Released before the entry goes, so a caller arriving between the
            // two waits on a mutex nobody holds rather than on one this sweep
            // is still using.
            drop(held);
            self.upload_locks.remove(&session_id);
        }
        Ok(())
    }

    /// Whether nothing of this session survives under the final name.
    ///
    /// The final name is `<hash>.<ext>`, and it is emphatically not this
    /// session's private property: the unique index is per member, so two
    /// members can hold sessions for the same hash in one library, and a
    /// successful commit by either produces exactly that file. Removing it
    /// unguarded would delete a live track's file to clean up a stranger's
    /// failure — which is why the earlier shape of this fix was refused.
    ///
    /// Two guards make it safe, and they are the two ways the file can belong
    /// to somebody else. A track naming that path is using it. Another session
    /// on the same hash may be committing right now, between its rename and its
    /// transaction, in which case there is no track yet and the file is still
    /// not ours. Neither, and the only thing that could have put a file there
    /// is the commit this session gave up on.
    async fn final_file_is_gone(
        &self,
        library_id: &str,
        declared_hash: &str,
        extension: &str,
        root: &str,
        session_id: Uuid,
    ) -> Result<bool, ServiceError> {
        let relative = relative_final(declared_hash, extension);
        let path = std::path::Path::new(root).join(&relative);
        match tokio::fs::try_exists(&path).await {
            Ok(false) => return Ok(true),
            Ok(true) => {}
            // Not the same thing as absent, and reading it as absent would drop
            // the row while the file it names may well be there — the orphan
            // this whole path exists to prevent, one question earlier.
            Err(error) => {
                tracing::warn!(
                    %error,
                    session = %session_id,
                    file = %relative,
                    "cannot tell whether a stuck file is there; keeping the session so the sweep retries"
                );
                return Ok(false);
            }
        }
        // Both guards in one statement, so they read one state of the
        // database. Asked one after the other, a commit landing between them —
        // its track inserted and its session deleted in a single transaction —
        // answered "no track yet" to the first and "no session any more" to the
        // second, and the sweep deleted the file of the track just committed.
        let others: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM track WHERE library_id=? AND relative_path=?) \
                 OR EXISTS(SELECT 1 FROM upload_session \
                           WHERE library_id=? AND declared_hash=? AND id<>?)",
        )
        .bind(library_id)
        .bind(&relative)
        .bind(library_id)
        .bind(declared_hash)
        .bind(session_id.to_string())
        .fetch_one(self.db.pool())
        .await?;
        if others != 0 {
            // Somebody else's, either already or possibly. The row goes anyway:
            // keeping it would hold a reservation against a file this session
            // no longer owns.
            return Ok(true);
        }
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => {
                tracing::warn!(
                    %error,
                    session = %session_id,
                    file = %relative,
                    "cannot remove a stuck file; keeping the session so the sweep retries"
                );
                Ok(false)
            }
        }
    }
}
