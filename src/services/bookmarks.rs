//! Per-track playback bookmarks.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Bookmarks the user has set, most recently changed first.
    pub async fn bookmarks(&self, user_id: Uuid) -> Result<Vec<BookmarkItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.bookmarks_on(&mut connection, user_id).await
    }

    pub(super) async fn bookmarks_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<BookmarkItem>, ServiceError> {
        // Joined against `song_select!` so a bookmark on a track that has become
        // unavailable, or on a library the account has lost, simply stops being
        // listed rather than being returned pointing at nothing.
        let rows = sqlx::query(concat!(
            "SELECT b.position_ms, b.comment AS bookmark_comment, \
                    b.created_at AS bookmark_created_at, b.updated_at AS bookmark_updated_at, \
                    song.* FROM bookmark b JOIN (",
            song_select!(),
            ") AS song ON song.id=b.track_id \
             WHERE b.user_id=? ORDER BY b.updated_at DESC, song.id"
        ))
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut bookmarks = Vec::with_capacity(rows.len());
        for row in rows {
            bookmarks.push(BookmarkItem {
                position_ms: row.try_get("position_ms")?,
                comment: row.try_get("bookmark_comment")?,
                created_at: row.try_get("bookmark_created_at")?,
                updated_at: row.try_get("bookmark_updated_at")?,
                song: song_from_row(row)?,
            });
        }
        let mut songs = bookmarks
            .iter()
            .map(|bookmark| bookmark.song.clone())
            .collect::<Vec<_>>();
        attach_song_relations(&mut *connection, user_id, &mut songs).await?;
        for (bookmark, song) in bookmarks.iter_mut().zip(songs) {
            bookmark.song = song;
        }
        Ok(bookmarks)
    }

    pub async fn set_bookmark(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        position_ms: i64,
        comment: Option<&str>,
    ) -> Result<(), ServiceError> {
        self.set_bookmark_with_context(
            user_id,
            track_id,
            position_ms,
            comment,
            MutationContext::server_generated(),
        )
        .await
    }

    /// Sets, or moves, the bookmark on one track.
    ///
    /// A bookmark answers "where did I stop in this file", so there is one per
    /// account and track and a second call moves it rather than adding another.
    pub async fn set_bookmark_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        position_ms: i64,
        comment: Option<&str>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        if position_ms < 0 {
            return Err(ServiceError::Invalid);
        }
        let intent = MutationIntent::new(
            "set-bookmark",
            &format!("bookmark:{track_id}"),
            &serde_json::json!({ "position_ms": position_ms, "comment": comment }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "bookmark")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO bookmark (user_id, track_id, position_ms, comment, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, track_id) DO UPDATE SET position_ms=excluded.position_ms, \
               comment=excluded.comment, updated_at=excluded.updated_at",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(position_ms)
        .bind(comment)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "bookmark",
                track_id,
                "upsert",
                &serde_json::json!({
                    "track_id": track_id,
                    "position_ms": position_ms,
                    "comment": comment,
                }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn delete_bookmark(&self, user_id: Uuid, track_id: Uuid) -> Result<(), ServiceError> {
        self.delete_bookmark_with_context(user_id, track_id, MutationContext::server_generated())
            .await
    }

    /// Removes the bookmark on one track.
    ///
    /// Removing one that is not there succeeds: the caller asked for the track
    /// to carry no bookmark, and it does not. Reporting not-found would also
    /// answer a question about another account's catalogue, which the rest of
    /// the surface refuses to do.
    pub async fn delete_bookmark_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "delete-bookmark",
            &format!("bookmark:{track_id}"),
            &serde_json::json!({}),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "bookmark")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        sqlx::query("DELETE FROM bookmark WHERE user_id=? AND track_id=?")
            .bind(user_id.to_string())
            .bind(track_id.to_string())
            .execute(&mut *tx)
            .await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "bookmark",
                track_id,
                "delete",
                &serde_json::json!({ "track_id": track_id }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }
}
