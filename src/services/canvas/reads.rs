//! What a canvas reads as: the blob a track or an account is shown, and the
//! file it is served from.

use super::*;

impl DomainServices {
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
