//! The snapshot a client rebuilds its state from.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn sync_snapshot(
        &self,
        user_id: Uuid,
        history_limit: i64,
    ) -> Result<SyncSnapshotData, ServiceError> {
        let mut tx = self.db.pool().begin().await?;
        // A global watermark, read inside the same transaction as the rows
        // below so nothing committed after it can be missed.
        //
        // Deliberately not this user's MAX: `changes` refuses cursors below the
        // journal's global floor, so a per-user watermark would hand an account
        // with no surviving events a cursor beneath that floor — it would
        // re-snapshot, get the same cursor, be refused again, and loop. Filtering
        // by user still happens in `changes`, so a global watermark only means
        // "everything up to here is already in this snapshot".
        let cursor = sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM sync_event")
            .fetch_one(&mut *tx)
            .await?;
        let playlists = self.playlists_on(&mut tx, user_id).await?;
        let favorites = self.starred_ids_on(&mut tx, user_id).await?;
        let ratings = self.ratings_on(&mut tx, user_id).await?;
        let queue = self.queue_on(&mut tx, user_id).await?;
        let history = self.history_on(&mut tx, user_id, history_limit).await?;
        let shares = self.shares_on(&mut tx, user_id).await?;
        let bookmarks = self.bookmarks_on(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(SyncSnapshotData {
            cursor,
            playlists,
            favorites,
            ratings,
            queue,
            history,
            shares,
            bookmarks,
        })
    }
}
