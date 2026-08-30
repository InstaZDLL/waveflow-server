//! The short video loop a member attaches to a track. RFC-009.
//!
//! Three things make this different from receiving a file, and each of them
//! removes machinery rather than adding it. A canvas fits in one request, so
//! there is no negotiation, no fragment and no session. It is server-owned, so
//! it goes to a content-addressed store beside the artwork rather than into the
//! operator's collection. And it is given by a human, so no scan can ever find
//! one, which is why the link sits beside the track instead of on it.
//!
//! What it does share is the sentence that governs RFC-008: accepting one is
//! spending somebody else's disk, permanently.
//!
//! # The order of writes
//!
//! The file first and the row second when placing; the row first and the file
//! second when removing. Both orders leave the same failure, and it is the
//! recoverable *kind* — bytes nothing names, in a store that is
//! content-addressed and therefore enumerable, so a sweep can find them.
//! [`DomainServices::sweep_canvas_store`] is that sweep; `artwork_dir`, which
//! had left the same question open since the beginning, has its own in
//! [`super::artwork`]. A row naming an absent file is the failure that could
//! not be recovered even in principle, because it is a dead link every read
//! runs into.
//!
//! Between a commit and the unlink that follows it there is a window, and it is
//! not theoretical: a `PUT` of the same content lands in it, finds no row,
//! writes its bytes and inserts its own — and the unlink carries off the file of
//! a live link. The lock in [`DomainServices::canvas_lock`] closes it. It is
//! keyed by hash rather than being the writer gate, because the race is per
//! blob and because file I/O has no business happening while the process-wide
//! gate is held — the rule `upload_locks` already follows.

use std::{path::PathBuf, str::FromStr, sync::Arc};

use sqlx::Row;
use uuid::Uuid;

use super::{parse_uuid, CanvasBlob, DomainServices, ServiceError};
use crate::authentication::now_ms;

/// The containers a canvas may arrive in.
///
/// Two, and both are what a browser and a desktop client can already play
/// without a decoder anyone has to ship. The list is short on purpose: every
/// entry is a format the server promises to serve forever, since the bytes
/// behind a hash never change.
const ACCEPTED_FORMATS: [&str; 2] = ["mp4", "webm"];

/// How long `ffprobe` gets to read a canvas.
///
/// The same five seconds `check_tool` allows the version probe. A loop of a few
/// seconds and a few hundred kilobytes is read in milliseconds, so this is not
/// a budget anything legitimate spends — it is the bound on something that will
/// never finish.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `ffprobe` had to say about the bytes offered.
struct Probed {
    format: String,
    duration_secs: f64,
    /// Whether the file carries sound nobody will ever hear. See
    /// [`DomainServices::strip_canvas_audio`].
    has_audio: bool,
}

/// How long a working file may sit before the sweep treats it as abandoned.
///
/// A `.part` or `.silent` belongs to one in-flight request, which the router's
/// own timeout already bounds in seconds. An hour is far past anything a live
/// placement can still be doing, and it is the margin that keeps this from
/// racing a request whose clock and this one's disagree.
const STAGING_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How often the store is walked.
///
/// Orphans are made by failures, not by use, so this is a repair pass rather
/// than housekeeping: daily costs one directory listing and finds nothing on a
/// server that has not crashed.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// What one pass of the store sweep found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CanvasSweep {
    /// Blobs no row named any more, and whose bytes are now gone.
    pub blobs_removed: usize,
    /// Working files a failed placement left behind.
    pub staging_removed: usize,
    /// Rows naming a file that is not there.
    ///
    /// Counted and never repaired. RFC-009 calls this the failure that cannot
    /// be recovered — a dead link every read runs into — and the repair would
    /// be to delete a member's canvas, which is a worse answer than telling the
    /// operator the truth.
    pub dead_links: usize,
    /// Names the sweep did not recognise, left exactly where they were.
    pub unknown: usize,
}

impl DomainServices {
    /// The lock that serialises everything touching one blob.
    fn canvas_lock(&self, hash: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.canvas_locks
                .entry(hash.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .value(),
        )
    }

    /// Drops the entry once the last holder has let go.
    ///
    /// Without this the map keeps a mutex for every hash the server has ever
    /// touched, which is a leak measured in the size of the catalogue rather
    /// than in anything bounded — `upload_locks` removes its entry after a
    /// commit for the same reason.
    ///
    /// **Call this only after the caller's own `Arc` has been dropped**, so
    /// `strong_count == 1` means the map holds the last one. The count is read
    /// under the shard lock, and [`Self::canvas_lock`] takes that same lock to
    /// insert, so a concurrent holder either arrives first and is counted, or
    /// arrives after and inserts a fresh mutex. Either order is correct: the
    /// entry is a rendezvous point, not state.
    fn release_canvas_lock(&self, hash: &str) {
        self.canvas_locks
            .remove_if(hash, |_, lock| Arc::strong_count(lock) == 1);
    }

    fn canvas_path(&self, blob: &CanvasBlob) -> PathBuf {
        self.canvas_dir.join(blob.file_name())
    }

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
        let mut tx = self.db.pool().begin().await?;

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
            let mut tx = self.db.pool().begin().await?;
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
    async fn release_canvas_blob(&self, hash: &str) {
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
        let mut tx = self.db.pool().begin().await?;
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

    /// The loop a track carries, for somebody entitled to see that track.
    pub async fn canvas_for_track(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Option<CanvasBlob>, ServiceError> {
        // Membership in the track's library, re-read on this request as on
        // every other. A track id is guessed no worse than a hash, so the alias
        // is no more an authorisation than the fingerprint is.
        let row = sqlx::query(
            "SELECT c.hash, c.format, c.byte_size FROM track_canvas tc \
             JOIN canvas c ON c.hash=tc.canvas_hash \
             JOIN library_member m ON m.library_id=tc.library_id \
             WHERE tc.track_id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?;
        blob_from_row(row)
    }

    /// The blob behind a hash, for somebody whose libraries reference it.
    ///
    /// Knowing a hash establishes nothing: two accounts that are strangers to
    /// each other can hold the same loop, so the fingerprint identifies content
    /// and never proves access. Resolved the way `artwork_for_user` resolves a
    /// cover, and a hash nobody reachable references is indistinguishable from
    /// one that does not exist.
    pub async fn canvas_for_user(
        &self,
        user_id: Uuid,
        hash: &str,
    ) -> Result<Option<CanvasBlob>, ServiceError> {
        let row = sqlx::query(
            "SELECT c.hash, c.format, c.byte_size FROM canvas c \
             WHERE c.hash=? AND EXISTS ( \
               SELECT 1 FROM track_canvas tc \
               JOIN library_member m ON m.library_id=tc.library_id \
               WHERE tc.canvas_hash=c.hash AND m.user_id=?)",
        )
        .bind(hash)
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?;
        blob_from_row(row)
    }

    /// Walks the canvas store and collects what a failure left in it.
    ///
    /// RFC-009 decision 6 says when a blob stops being referenced and by which
    /// transaction; it does not say who picks up the bytes a crash left behind.
    /// This does. Two sources, both named there: a failure between the commit
    /// and the unlink, and a placement that died between writing its file and
    /// inserting its row.
    ///
    /// The shape is `spawn_upload_sweeper`'s, deliberately — boot first, then a
    /// pass per interval.
    pub fn spawn_canvas_sweeper(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            services.sweep_canvas_now().await;
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_canvas_now().await;
            }
        });
    }

    async fn sweep_canvas_now(&self) {
        match self.sweep_canvas_store().await {
            Ok(swept) if swept == CanvasSweep::default() => {}
            Ok(swept) => tracing::info!(
                blobs = swept.blobs_removed,
                staging = swept.staging_removed,
                dead_links = swept.dead_links,
                unknown = swept.unknown,
                "canvas store swept"
            ),
            Err(error) => tracing::warn!(%error, "could not sweep the canvas store"),
        }
    }

    /// One pass. Public so a test can run it rather than wait a day for it.
    ///
    /// **Nothing is removed that this does not recognise.** The store lives
    /// under the operator's `data/`, and a sweep that deletes what it cannot
    /// name is a sweep nobody should run. A file is only a candidate if it is
    /// `<64 hex>.<format>` for a format the whitelist admits, or a working name
    /// this module writes; everything else is counted and left where it is.
    pub async fn sweep_canvas_store(&self) -> Result<CanvasSweep, ServiceError> {
        let mut swept = CanvasSweep::default();
        let mut entries = match tokio::fs::read_dir(&self.canvas_dir).await {
            Ok(entries) => entries,
            // No store yet is not a failure: no library has taken a canvas.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(swept);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read the canvas store");
                return Err(ServiceError::Unavailable);
            }
        };

        let mut seen = std::collections::HashSet::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            tracing::warn!(%error, "cannot walk the canvas store");
            ServiceError::Unavailable
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            match classify_store_entry(&name) {
                StoreEntry::Blob { hash, .. } => {
                    seen.insert(hash.clone());
                    if self.discard_orphan_blob(&hash, &entry.path()).await {
                        swept.blobs_removed += 1;
                    }
                }
                StoreEntry::Working => {
                    if self.discard_stale_working_file(&entry.path()).await {
                        swept.staging_removed += 1;
                    }
                }
                StoreEntry::Unknown => {
                    tracing::warn!(file = %name, "unrecognised file left in the canvas store");
                    swept.unknown += 1;
                }
            }
        }

        // The other direction, which is reported and never repaired: a row is
        // somebody's canvas, and deleting it to tidy the store would answer a
        // dead link by making it a missing one.
        let named: Vec<String> = sqlx::query_scalar("SELECT hash FROM canvas")
            .fetch_all(self.db.pool())
            .await?;
        for hash in named {
            if !seen.contains(&hash) {
                tracing::warn!(%hash, "a canvas row names a file the store does not hold");
                swept.dead_links += 1;
            }
        }
        Ok(swept)
    }

    /// Removes a blob nothing references, under the lock that makes the
    /// question answerable.
    ///
    /// Without the lock this is the sweep that creates the damage it exists to
    /// repair: a placement writes its file, and between that write and its row
    /// the sweep sees bytes no row names and takes them — leaving the placement
    /// to commit a row whose file is gone. Holding the blob's lock makes the
    /// two mutually exclusive, and the count is re-read inside it rather than
    /// trusted from the walk.
    ///
    /// **Both halves are covered now.** Removing the count fails a test — a
    /// referenced blob is taken. Removing the lock fails
    /// `tests::a_placement_that_commits_while_the_sweep_waits_keeps_its_bytes`,
    /// which does not race for the interleaving but arranges it: the test holds
    /// this lock, which is what a placement holds between its bytes and its
    /// row, and the sweep has to wait. That needed a unit test rather than an
    /// integration one — the lock is private, and reaching it is the whole
    /// trick.
    async fn discard_orphan_blob(&self, hash: &str, path: &std::path::Path) -> bool {
        let removed = {
            let lock = self.canvas_lock(hash);
            let _held = lock.lock().await;
            let referenced: Result<i64, _> =
                sqlx::query_scalar("SELECT COUNT(*) FROM canvas WHERE hash=?")
                    .bind(hash)
                    .fetch_one(self.db.pool())
                    .await;
            match referenced {
                Ok(0) => match tokio::fs::remove_file(path).await {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => {
                        tracing::warn!(%error, "cannot remove an orphaned canvas");
                        false
                    }
                },
                Ok(_) => false,
                Err(error) => {
                    tracing::warn!(%error, "cannot decide whether a canvas is referenced");
                    false
                }
            }
        };
        self.release_canvas_lock(hash);
        removed
    }

    /// Removes a working file older than any request could still be using.
    ///
    /// Age rather than a lock, because these names belong to one call and
    /// nothing else can look them up: `<uuid>.part` and `<uuid>.silent` are
    /// known only to the placement that made them, so there is no lock to take
    /// and no row to consult. The grace period is what stands in for both.
    async fn discard_stale_working_file(&self, path: &std::path::Path) -> bool {
        let Ok(metadata) = tokio::fs::metadata(path).await else {
            return false;
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STAGING_GRACE);
        if !stale {
            return false;
        }
        match tokio::fs::remove_file(path).await {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!(%error, "cannot remove an abandoned canvas working file");
                false
            }
        }
    }

    /// Where a blob's bytes are, for a caller that has already been authorised.
    pub fn canvas_file(&self, blob: &CanvasBlob) -> PathBuf {
        self.canvas_path(blob)
    }

    /// The library a track belongs to, if this account may place a canvas on it.
    async fn canvas_library_for_track(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let row = sqlx::query(
            "SELECT t.library_id, l.accepts_canvas, m.role FROM track t \
             JOIN library l ON l.id=t.library_id \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE t.id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_upload() || row.try_get::<i64, _>("accepts_canvas")? == 0 {
            return Err(ServiceError::Forbidden);
        }
        parse_uuid(row.try_get::<String, _>("library_id")?).map_err(Into::into)
    }

    /// Rewrites a staged canvas without its soundtrack.
    ///
    /// A **remux, never a re-encode**: `-c copy` moves the video packets across
    /// untouched, so this is not what decision 8 refuses. That decision's
    /// argument was that transcoding would make a canvas a compute charge *per
    /// playback*; this is one stream copy, once, at ingestion, and the picture
    /// comes out bit for bit what it went in as.
    ///
    /// `-map_metadata -1` and `-fflags +bitexact` are load-bearing rather than
    /// tidy. The store is content-addressed, so two members offering the same
    /// file must produce the same output or the deduplication of decision 1
    /// quietly stops working and each of them is charged for a blob. Those two
    /// flags drop the creation timestamp and the encoder string that would
    /// otherwise differ between two runs a second apart.
    ///
    /// It holds within one deployment. Two different FFmpeg versions may still
    /// mux the same packets differently, so a server upgrade can give the same
    /// source a second blob — which costs a duplicate, never a wrong answer,
    /// and is the reason this is written down rather than assumed.
    ///
    /// **The suite does not cover those two flags, and does not claim to.**
    /// `canvas` asserts that the same bytes offered twice give the same blob,
    /// and that holds with the flags removed: one FFmpeg build muxing the same
    /// packets twice agrees with itself either way. What the flags defend
    /// against is the version string this muxer writes, which differs between
    /// builds — a difference no test running against a single build can see.
    async fn strip_canvas_audio(
        &self,
        staging: &std::path::Path,
        format: &str,
    ) -> Result<PathBuf, ServiceError> {
        // A name the caller can predict without knowing the container, so its
        // cleanup reaches this file on every path out — including the ones that
        // fail after it exists. The muxer is named with `-f` rather than
        // inferred from an extension, which is what lets the name stay plain.
        let silent = staging.with_extension("silent");
        let output = tokio::time::timeout(
            PROBE_TIMEOUT,
            tokio::process::Command::new(&self.ffmpeg_path)
                .arg("-hide_banner")
                .arg("-loglevel")
                .arg("error")
                .arg("-y")
                .arg("-fflags")
                .arg("+bitexact")
                .arg("-i")
                .arg(staging)
                .arg("-map")
                .arg("0:v")
                .arg("-c")
                .arg("copy")
                .arg("-an")
                .arg("-map_metadata")
                .arg("-1")
                .arg("-fflags")
                .arg("+bitexact")
                .arg("-f")
                .arg(format)
                .arg(&silent)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| {
            tracing::warn!("ffmpeg timed out taking the sound out of a canvas");
            ServiceError::Invalid
        })?
        .map_err(|error| {
            tracing::error!(%error, "cannot run ffmpeg on a canvas");
            ServiceError::Unavailable
        })?;
        if !output.status.success() {
            // The probe already said this is a container we accept, so a remux
            // that fails is about this file rather than about the server.
            tracing::warn!("a canvas could not be stripped of its audio");
            return Err(ServiceError::Invalid);
        }
        Ok(silent)
    }

    /// Reads the container and the duration out of the bytes themselves.
    ///
    /// Never from a `Content-Type` or an extension: anything at all can be
    /// called `.mp4`. `ffprobe` is already a dependency and already on the
    /// `PATH` of every deployment, and reading the file is the only thing that
    /// is evidence.
    ///
    /// Bounded, and killed if this future is dropped. The bytes being read were
    /// chosen by whoever sent them, so a container crafted to make a demuxer
    /// spin is an input this route has to survive: without the bound one
    /// request holds a task forever, and without `kill_on_drop` a client that
    /// hangs up leaves the process behind to finish spinning alone. Both
    /// already exist in `MediaService` — the five-second bound on `check_tool`,
    /// and `kill_on_drop` on the transcode child.
    async fn probe_canvas(&self, path: &std::path::Path) -> Result<Probed, ServiceError> {
        let output = tokio::time::timeout(
            PROBE_TIMEOUT,
            tokio::process::Command::new(&self.ffprobe_path)
                .arg("-v")
                .arg("error")
                .arg("-print_format")
                .arg("json")
                .arg("-show_format")
                .arg("-show_streams")
                .arg(path)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| {
            // The server is fine; this file is not something it will finish
            // reading. Refusing the offer says that without pretending the
            // deployment is broken.
            tracing::warn!("ffprobe timed out on a canvas");
            ServiceError::Invalid
        })?
        .map_err(|error| {
            tracing::error!(%error, "cannot run ffprobe on a canvas");
            ServiceError::Unavailable
        })?;
        if !output.status.success() {
            // Not an outage: what arrived is not something ffprobe can read,
            // which is a verdict on the offer rather than on the server.
            return Err(ServiceError::Invalid);
        }
        let probed: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| ServiceError::Unavailable)?;

        // A container reports a family — "matroska,webm", or the mov/mp4 group
        // — so the whitelist is matched against the members rather than the
        // whole string.
        let format_names = probed
            .get("format")
            .and_then(|format| format.get("format_name"))
            .and_then(serde_json::Value::as_str)
            .ok_or(ServiceError::Invalid)?;
        let format = ACCEPTED_FORMATS
            .iter()
            .find(|accepted| format_names.split(',').any(|name| name == **accepted))
            .ok_or(ServiceError::Invalid)?;

        // A canvas without a video stream is not a canvas, whatever it is.
        let has_video = probed
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|streams| {
                streams.iter().any(|stream| {
                    stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("video")
                })
            });
        if !has_video {
            return Err(ServiceError::Invalid);
        }
        // Reported rather than refused. RFC-009 decision 9: a canvas loops over a
        // track that is already playing and the desktop's `<video>` is muted in
        // its markup, so nobody can hear this — and keeping it would spend the
        // ceiling on bytes that buy nothing. `identify_and_place` remuxes the
        // stream out before the file reaches the store; this only says whether
        // there is one to take out.
        let has_audio = probed
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|streams| {
                streams.iter().any(|stream| {
                    stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("audio")
                })
            });

        // A container that does not say how long it runs cannot be checked
        // against the ceiling, and an unchecked ceiling is not one.
        let duration_secs = probed
            .get("format")
            .and_then(|format| format.get("duration"))
            .and_then(serde_json::Value::as_str)
            .and_then(|duration| duration.parse::<f64>().ok())
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .ok_or(ServiceError::Invalid)?;

        Ok(Probed {
            format: (*format).to_owned(),
            duration_secs,
            has_audio,
        })
    }
}

/// What a name in the store is, if it is anything this module wrote.
enum StoreEntry {
    Blob { hash: String },
    Working,
    Unknown,
}

/// Reads a file name without touching the disk.
///
/// Deliberately strict. A blob is sixty-four lowercase hex characters and a
/// format the whitelist admits — the same two rules that built the name — and
/// anything that does not match both is not something this sweep will delete.
fn classify_store_entry(name: &str) -> StoreEntry {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return StoreEntry::Unknown;
    };
    if ACCEPTED_FORMATS.contains(&extension)
        && stem.len() == 64
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return StoreEntry::Blob {
            hash: stem.to_owned(),
        };
    }
    // The two names a placement writes before it knows the final one, and the
    // stem is checked as strictly as a blob's. It is a UUID this module
    // invented — `place_canvas` writes `<uuid>.part`, and the strip renames it
    // to `<uuid>.silent` — so requiring one to parse costs nothing and closes
    // the asymmetry an earlier version of this function had: sixty-four hex
    // demanded of a blob, and any stem at all accepted before `.part`. The
    // operator's `notes.part` is not this module's file.
    if matches!(extension, "part" | "silent") && Uuid::parse_str(stem).is_ok() {
        return StoreEntry::Working;
    }
    StoreEntry::Unknown
}

fn blob_from_row(row: Option<sqlx::sqlite::SqliteRow>) -> Result<Option<CanvasBlob>, ServiceError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The lock in [`DomainServices::discard_orphan_blob`], demonstrated.
    ///
    /// Its own comment said the suite covered the re-read and not the lock:
    /// removing the count fails a test, removing the lock fails nothing,
    /// because the damage needs a placement and a sweep to interleave at one
    /// instant and no *integration* test can force that ordering.
    ///
    /// A unit test can, because it sits in the module and may hold the lock
    /// itself — which is exactly what a placement holds between writing its
    /// bytes and inserting its row. So the interleaving is not raced for, it is
    /// arranged: the sweep starts, finds the blob, and blocks. The row is
    /// committed while it waits. The lock is released, the sweep re-reads, and
    /// the file it was about to take is now somebody's canvas.
    ///
    /// Removing the lock fails this: the sweep reads no row at once and unlinks
    /// before the placement ever commits, leaving a live row naming nothing.
    #[tokio::test]
    async fn a_placement_that_commits_while_the_sweep_waits_keeps_its_bytes() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = crate::Config::for_data_dir(temp.path().join("data"));
        let state = crate::initialize(&config).await.expect("initialize");
        let services = state.services.clone();

        // Bytes in the store and no row: what a placement looks like from
        // outside, one instant before it commits.
        tokio::fs::create_dir_all(&config.canvas_dir)
            .await
            .expect("canvas store");
        let hash = "c".repeat(64);
        let path = config.canvas_dir.join(format!("{hash}.mp4"));
        tokio::fs::write(&path, b"loop bytes").await.expect("write");

        let lock = services.canvas_lock(&hash);
        let held = lock.lock().await;

        let sweeper = {
            let services = services.clone();
            tokio::spawn(async move { services.sweep_canvas_store().await })
        };

        // Every chance to run and find no row. Without the lock this is where
        // the bytes go.
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            path.exists(),
            "the sweep took a blob whose lock was held: it never waited"
        );

        sqlx::query(
            "INSERT INTO canvas (hash, format, byte_size, created_at) VALUES (?, 'mp4', 10, ?)",
        )
        .bind(&hash)
        .bind(now_ms())
        .execute(services.db.pool())
        .await
        .expect("the placement commits");
        drop(held);

        let swept = sweeper
            .await
            .expect("the sweep finishes")
            .expect("no error");
        assert_eq!(
            swept.blobs_removed, 0,
            "the re-read under the lock is what sees the new row"
        );
        assert!(
            path.exists(),
            "a live row must not be left naming bytes the sweep carried off"
        );
        assert_eq!(swept.dead_links, 0);
    }
}
