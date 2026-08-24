//! Scrobbles, listening history and the saved queue.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn scrobble(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
    ) -> Result<(), ServiceError> {
        self.scrobble_with_context(
            user_id,
            track_id,
            submission,
            time,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn scrobble_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if submission {
                "scrobble"
            } else {
                "now-playing"
            },
            &format!("track:{track_id}"),
            &serde_json::json!({ "submission": submission, "time": time }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "scrobble")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        let current_time = now_ms();
        let now = time.unwrap_or(current_time);
        const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
        if now < 0 || now > current_time.saturating_add(MAX_FUTURE_SKEW_MS) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "INSERT INTO play_event (user_id, track_id, submission, played_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(i64::from(submission))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if submission {
            sqlx::query("DELETE FROM now_playing WHERE user_id=?")
                .bind(user_id.to_string())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO now_playing (user_id, track_id, started_at, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET track_id=excluded.track_id, started_at=excluded.started_at, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(track_id.to_string()).bind(now).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "scrobble",
                track_id,
                if submission { "append" } else { "upsert" },
                &serde_json::json!({
                    "track_id": track_id,
                    "submission": submission,
                    "played_at": now,
                }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn now_playing(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, SongItem, i64)>, ServiceError> {
        let rows = sqlx::query(
            "SELECT a.username, n.track_id, n.started_at FROM now_playing n \
             JOIN account a ON a.id=n.user_id WHERE a.disabled=0 ORDER BY n.started_at DESC",
        )
        .fetch_all(self.db.pool())
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let id = parse_uuid(row.try_get("track_id")?)?;
            match self.songs_by_ids(user_id, &[id]).await {
                Ok(mut songs) => {
                    if let Some(song) = songs.pop() {
                        result.push((row.try_get("username")?, song, row.try_get("started_at")?));
                    }
                }
                Err(ServiceError::NotFound) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }

    pub async fn history(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.history_on(&mut connection, user_id, limit).await
    }

    pub(super) async fn history_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        if !(0..=MAX_HISTORY_LIMIT).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "SELECT p.track_id, p.submission, p.played_at FROM play_event p \
             JOIN track t ON t.id=p.track_id JOIN library_member m ON m.library_id=t.library_id \
             WHERE p.user_id=? AND m.user_id=? ORDER BY p.played_at DESC, p.id DESC LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(HistoryItem {
                track_id: parse_uuid(row.try_get("track_id")?)?,
                submission: row.try_get::<i64, _>("submission")? != 0,
                played_at: row.try_get("played_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn save_queue(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
    ) -> Result<(), ServiceError> {
        self.save_queue_with_context(
            user_id,
            ids,
            current,
            position_ms,
            client,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_queue_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "save",
            &format!("queue:{user_id}"),
            &serde_json::json!({
                "track_ids": ids,
                "current": current,
                "position_ms": position_ms,
                "client": client,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "queue")?;
            return Ok(());
        }
        if ids.len() > MAX_QUEUE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        if position_ms < 0 {
            return Err(ServiceError::Invalid);
        }
        self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        if let Some(current) = current {
            self.songs_by_ids_on(&mut tx, user_id, &[current]).await?;
        }
        sqlx::query("INSERT INTO play_queue (user_id, current_track_id, position_ms, changed_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET current_track_id=excluded.current_track_id, position_ms=excluded.position_ms, changed_by=excluded.changed_by, updated_at=excluded.updated_at")
            .bind(user_id.to_string()).bind(current.map(|id| id.to_string())).bind(position_ms).bind(client).bind(now_ms()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM play_queue_track WHERE user_id=?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        if !ids.is_empty() {
            let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
            sqlx::query(
                "INSERT INTO play_queue_track (user_id, track_id, position) \
                 SELECT ?, value, CAST(key AS INTEGER) FROM json_each(?)",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .execute(&mut *tx)
            .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "queue",
                user_id,
                "upsert",
                &serde_json::json!({
                    "track_ids": ids,
                    "current": current,
                    "position_ms": position_ms,
                    "client": client,
                }),
                Some(user_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn queue(&self, user_id: Uuid) -> Result<Option<QueueItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.queue_on(&mut connection, user_id).await
    }

    pub(super) async fn queue_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Option<QueueItem>, ServiceError> {
        let row = sqlx::query("SELECT current_track_id, position_ms, changed_by, updated_at FROM play_queue WHERE user_id=?")
            .bind(user_id.to_string()).fetch_optional(&mut *connection).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM play_queue_track WHERE user_id=? ORDER BY position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        let current = row
            .try_get::<Option<String>, _>("current_track_id")?
            .map(parse_uuid)
            .transpose()?;
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, &ids)
            .await?;
        Ok(Some(QueueItem {
            // The lenient resolution above drops a track that went unavailable
            // since the queue was saved. Keeping `current` on it would name a
            // song the client was never handed.
            current: current.filter(|id| songs.iter().any(|song| song.id == *id)),
            position_ms: row.try_get("position_ms")?,
            changed_by: row.try_get("changed_by")?,
            updated_at: row.try_get("updated_at")?,
            songs,
        }))
    }
}
