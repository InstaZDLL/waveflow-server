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
//! recoverable one — bytes nothing names, in a store that can be enumerated. A
//! row naming an absent file is the one that cannot be recovered, because it is
//! a dead link every read runs into.
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

use super::{parse_uuid, DomainServices, ServiceError};
use crate::authentication::now_ms;

/// The containers a canvas may arrive in.
///
/// Two, and both are what a browser and a desktop client can already play
/// without a decoder anyone has to ship. The list is short on purpose: every
/// entry is a format the server promises to serve forever, since the bytes
/// behind a hash never change.
const ACCEPTED_FORMATS: [&str; 2] = ["mp4", "webm"];

/// One blob of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasBlob {
    pub hash: String,
    pub format: String,
    pub byte_size: i64,
}

impl CanvasBlob {
    /// The name the bytes carry on disk. The hash is the name, so this is a
    /// pure function of the row rather than something stored twice.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.hash, self.format)
    }
}

/// What `ffprobe` had to say about the bytes offered.
struct Probed {
    format: String,
    duration_secs: f64,
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
            .identify_and_place(user_id, track_id, byte_size, &staging)
            .await;
        // Whatever happened, nothing of this attempt is left under a name only
        // this call knows.
        if tokio::fs::try_exists(&staging).await.unwrap_or(false) {
            if let Err(error) = tokio::fs::remove_file(&staging).await {
                tracing::warn!(%error, "a staged canvas could not be removed");
            }
        }
        outcome
    }

    /// The half of [`Self::place_canvas`] that runs with a staged file in hand.
    async fn identify_and_place(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        byte_size: i64,
        staging: &std::path::Path,
    ) -> Result<CanvasBlob, ServiceError> {
        // Outside every lock: probing spawns a subprocess and reads a file, and
        // neither belongs under a mutex the rest of the server is waiting on.
        let probed = self.probe_canvas(staging).await?;
        if probed.duration_secs > f64::from(self.canvas.max_duration_secs) {
            return Err(ServiceError::Invalid);
        }
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
        let blob = CanvasBlob {
            hash,
            format: probed.format,
            byte_size,
        };

        let lock = self.canvas_lock(&blob.hash);
        let _held = lock.lock().await;

        // The file first. Already there means another track holds the same
        // loop, and the bytes are identical by construction — rewriting them
        // would be a write for nothing, over a file a live link may be being
        // served from.
        let final_path = self.canvas_path(&blob);
        if !tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
            tokio::fs::rename(staging, &final_path)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "cannot move a canvas into the store");
                    ServiceError::Unavailable
                })?;
        }

        let replaced = match self.link_canvas(user_id, track_id, &blob).await {
            Ok(replaced) => replaced,
            Err(error) => {
                // The row did not happen, so the bytes must not stay unless
                // something else already named them. Still under the lock, so
                // no concurrent placement can have named them since.
                self.discard_unreferenced_blob(&blob, &final_path).await;
                return Err(error);
            }
        };
        drop(_held);

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
    ) -> Result<Option<String>, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;

        // Authoritative, unlike the check that let this call get this far.
        let row = sqlx::query(
            "SELECT t.library_id, l.accepts_uploads, m.role FROM track t \
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
        if row.try_get::<i64, _>("accepts_uploads")? == 0 {
            return Err(ServiceError::Forbidden);
        }
        // Derived from the track inside the writing transaction and never taken
        // from the request, so nobody chooses which library they attach a track
        // to. The same thing `track_override` does.
        let library_id: String = row.try_get("library_id")?;

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
        tx.commit().await?;
        Ok(previous)
    }

    /// Removes the loop a track carries.
    pub async fn remove_canvas(&self, user_id: Uuid, track_id: Uuid) -> Result<(), ServiceError> {
        let removed = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.pool().begin().await?;
            let row = sqlx::query(
                "SELECT m.role FROM track t \
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
            // Deliberately not gated on `accepts_uploads`. Closing a library to
            // new files must not strand what it already holds, and taking
            // something away never spends the operator's disk.
            let hash: Option<String> = sqlx::query_scalar(
                "DELETE FROM track_canvas WHERE track_id=? RETURNING canvas_hash",
            )
            .bind(track_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            hash.ok_or(ServiceError::NotFound)?
        };
        self.release_canvas_blob(&removed).await;
        Ok(())
    }

    /// Erases a blob once nothing references it any more.
    ///
    /// The count, the row and the unlink are one decision taken under the
    /// blob's lock, so two removals cannot each conclude that a reference
    /// remains and leave bytes nothing names. Failing here is not an error the
    /// caller can act on: the link is already gone, which is what they asked
    /// for, and orphaned bytes are what the store sweep exists to collect.
    async fn release_canvas_blob(&self, hash: &str) {
        let lock = self.canvas_lock(hash);
        let _held = lock.lock().await;
        let doomed = match self.forget_unreferenced_blob(hash).await {
            Ok(doomed) => doomed,
            Err(error) => {
                tracing::warn!(%error, "cannot decide whether a canvas is still referenced");
                return;
            }
        };
        let Some(blob) = doomed else { return };
        let path = self.canvas_path(&blob);
        // After the commit, never before: the other order leaves a row naming
        // an absent file the moment the transaction fails.
        if let Err(error) = tokio::fs::remove_file(&path).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, "an unreferenced canvas is left in the store");
            }
        }
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
            "SELECT t.library_id, l.accepts_uploads, m.role FROM track t \
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
        if !role.may_upload() || row.try_get::<i64, _>("accepts_uploads")? == 0 {
            return Err(ServiceError::Forbidden);
        }
        parse_uuid(row.try_get::<String, _>("library_id")?).map_err(Into::into)
    }

    /// Reads the container and the duration out of the bytes themselves.
    ///
    /// Never from a `Content-Type` or an extension: anything at all can be
    /// called `.mp4`. `ffprobe` is already a dependency and already on the
    /// `PATH` of every deployment, and reading the file is the only thing that
    /// is evidence.
    async fn probe_canvas(&self, path: &std::path::Path) -> Result<Probed, ServiceError> {
        let output = tokio::process::Command::new(&self.ffprobe_path)
            .arg("-v")
            .arg("error")
            .arg("-print_format")
            .arg("json")
            .arg("-show_format")
            .arg("-show_streams")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
            .await
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
        })
    }
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
