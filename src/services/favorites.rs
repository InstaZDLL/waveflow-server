//! Favourites and ratings.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn set_star(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
    ) -> Result<(), ServiceError> {
        self.set_star_with_context(
            user_id,
            entity_type,
            entity_id,
            starred,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_star_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if starred { "star" } else { "unstar" },
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "starred": starred }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "favorite")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if starred {
            sqlx::query("INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) VALUES (?, ?, ?, ?) ON CONFLICT DO UPDATE SET starred_at=excluded.starred_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(now_ms())
                .execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM user_star WHERE user_id=? AND entity_type=? AND entity_id=?")
                .bind(user_id.to_string())
                .bind(entity_type)
                .bind(entity_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "favorite",
                entity_id,
                if starred { "upsert" } else { "delete" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "starred": starred,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn entity_kind(
        &self,
        user_id: Uuid,
        entity_id: Uuid,
    ) -> Result<Option<&'static str>, ServiceError> {
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT entity_type FROM (\
               SELECT 'track' AS entity_type FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE t.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'album' FROM album a \
                 JOIN library_member m ON m.library_id=a.library_id \
                 WHERE a.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'artist' FROM artist ar \
                 JOIN library_member m ON m.library_id=ar.library_id \
                 WHERE ar.id=? AND m.user_id=? \
             )",
        )
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        if kinds.len() != 1 {
            return Ok(None);
        }
        Ok(match kinds[0].as_str() {
            "track" => Some("track"),
            "album" => Some("album"),
            "artist" => Some("artist"),
            _ => None,
        })
    }

    pub async fn starred_ids(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.starred_ids_on(&mut connection, user_id).await
    }

    pub(super) async fn starred_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        sqlx::query(
            "SELECT s.entity_type, s.entity_id, s.starred_at FROM user_star s \
             WHERE s.user_id=? AND ( \
               (s.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) \
             ) ORDER BY s.starred_at DESC",
        )
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?
            .into_iter().map(|row| Ok((row.try_get("entity_type")?, parse_uuid(row.try_get("entity_id")?)?, row.try_get("starred_at")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>().map_err(Into::into)
    }

    pub async fn ratings(&self, user_id: Uuid) -> Result<Vec<RatingItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.ratings_on(&mut connection, user_id).await
    }

    pub(super) async fn ratings_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<RatingItem>, ServiceError> {
        sqlx::query(
            "SELECT r.entity_type, r.entity_id, r.rating, r.updated_at FROM user_rating r \
             WHERE r.user_id=? AND ( \
               (r.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) \
             ) ORDER BY r.updated_at DESC, r.entity_type, r.entity_id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(RatingItem {
                entity_type: row.try_get("entity_type")?,
                entity_id: parse_uuid(row.try_get("entity_id")?)?,
                rating: row.try_get("rating")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn set_rating(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
    ) -> Result<(), ServiceError> {
        self.set_rating_with_context(
            user_id,
            entity_type,
            entity_id,
            rating,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_rating_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        // Checked before the writer gate rather than after the claim: an
        // out-of-range rating is refused on its own terms, without queuing
        // behind a scan for the right to be told so.
        if !(0..=5).contains(&rating) {
            return Err(ServiceError::Invalid);
        }
        let intent = MutationIntent::new(
            "set-rating",
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "rating": rating }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "rating")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if rating == 0 {
            sqlx::query(
                "DELETE FROM user_rating WHERE user_id=? AND entity_type=? AND entity_id=?",
            )
            .bind(user_id.to_string())
            .bind(entity_type)
            .bind(entity_id.to_string())
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO UPDATE SET rating=excluded.rating, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(rating).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "rating",
                entity_id,
                if rating == 0 { "delete" } else { "upsert" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "rating": rating,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }
}
