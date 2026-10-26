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

mod placement;
mod probe;
mod sweep;

/// The containers a canvas may arrive in.
///
/// Two, and both are what a browser and a desktop client can already play
/// without a decoder anyone has to ship. The list is short on purpose: every
/// entry is a format the server promises to serve forever, since the bytes
/// behind a hash never change.
const ACCEPTED_FORMATS: [&str; 2] = ["mp4", "webm"];

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

    /// The loop a track carries, for somebody entitled to see that track.
    pub async fn canvas_for_track(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Option<CanvasBlob>, ServiceError> {
        // Membership in the track's library, re-read on this request as on
        // every other. A track id is guessed no worse than a hash, so the alias
        // is no more an authorisation than the fingerprint is. And of an
        // account that is not disabled: a canvas ticket is sealed rather than
        // looked up, so this read is what stops it (RFC-023 decision 4).
        let row = sqlx::query(
            "SELECT c.hash, c.format, c.byte_size FROM track_canvas tc \
             JOIN canvas c ON c.hash=tc.canvas_hash \
             JOIN library_member m ON m.library_id=tc.library_id \
             JOIN account a ON a.id=m.user_id \
             WHERE tc.track_id=? AND m.user_id=? AND a.disabled=0",
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
mod tests;
