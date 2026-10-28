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
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let Some(receipt) = self
            .scrobble_on(
                &_writer, &mut tx, user_id, track_id, submission, time, context,
            )
            .await?
        else {
            tx.rollback().await?;
            return Ok(());
        };
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    /// Several listens in one request — a Subsonic `scrobble` names as many
    /// tracks as it likes — recorded together or not at all.
    ///
    /// One transaction under one hold of the writer gate. Applied one listen at
    /// a time, a refusal of the third left the first two recorded and the whole
    /// request reported as failed, so a client retrying it recorded them
    /// again; and checking every track first was not enough, because a scan
    /// could take the gate between the check and the last write and make a
    /// track unavailable in that gap.
    pub async fn scrobble_all(
        &self,
        user_id: Uuid,
        plays: &[(Uuid, Option<i64>)],
        submission: bool,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let mut receipts = Vec::with_capacity(plays.len());
        for (track_id, time) in plays {
            if let Some(receipt) = self
                .scrobble_on(
                    &_writer,
                    &mut tx,
                    user_id,
                    *track_id,
                    submission,
                    *time,
                    MutationContext::server_generated(),
                )
                .await?
            {
                receipts.push(receipt);
            }
        }
        tx.commit().await?;
        for receipt in receipts {
            self.sync.publish(user_id, receipt);
        }
        Ok(())
    }

    /// One listen, recorded in `tx` under `writer`, answering the receipt to
    /// publish once `tx` commits — or `None` for a replay of an operation
    /// already applied, which writes nothing.
    #[allow(clippy::too_many_arguments)]
    async fn scrobble_on(
        &self,
        writer: &tokio::sync::OwnedMutexGuard<()>,
        tx: &mut SqliteConnection,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
        context: MutationContext,
    ) -> Result<Option<MutationReceipt>, ServiceError> {
        let intent = MutationIntent::new(
            if submission {
                "scrobble"
            } else {
                "now-playing"
            },
            &format!("track:{track_id}"),
            &serde_json::json!({ "submission": submission, "time": time }),
        );
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(writer, &mut *tx, user_id, context, intent)
            .await?
        {
            validate_replay_type(&receipt, "scrobble")?;
            return Ok(None);
        }
        self.authorize_entity_on(&mut *tx, user_id, "track", track_id)
            .await?;
        let now = play_time(time)?;
        let recorded = sqlx::query(
            "INSERT INTO play_event (user_id, track_id, submission, played_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(i64::from(submission))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if submission {
            // In this transaction and under this writer gate, which is the
            // whole of RFC-010 decision 1: a listen cannot be recorded without
            // being queued for the destinations this account has linked, nor
            // queued without being recorded. A now-playing is deliberately not
            // queued — decision 3 — because delivering it late would announce a
            // track the listener left long ago.
            self.enqueue_scrobble_on(
                &mut *tx,
                user_id,
                recorded.last_insert_rowid(),
                track_id,
                now,
            )
            .await?;
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
                &mut *tx,
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
        Ok(Some(receipt))
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
        // One lookup for every listener at once. Resolving them one by one cost
        // a query per row, and every row of this table is a different person
        // playing something right now — the list is short but the shape was
        // wrong.
        let mut track_ids = rows
            .iter()
            .map(|row| parse_uuid(row.try_get("track_id")?))
            .collect::<Result<Vec<_>, _>>()?;
        track_ids.sort_unstable();
        track_ids.dedup();
        let mut connection = self.db.pool().acquire().await?;
        let visible = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &track_ids)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut result = Vec::new();
        for row in rows {
            let id = parse_uuid(row.try_get("track_id")?)?;
            // A track this account cannot see is skipped, as it was when the
            // strict lookup answered `NotFound` for it.
            let Some(song) = visible.get(&id).cloned() else {
                continue;
            };
            result.push((row.try_get("username")?, song, row.try_get("started_at")?));
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
        let mut tx = self.db.begin_write(&_writer).await?;
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
        // A track whose file went is left out rather than refused: the client
        // keeps it in the queue it holds, and refusing the whole queue left it
        // unable to save anything until the queue was cleared by hand (DSub,
        // the 2026-10-28 campaign). A current track that went takes its
        // position with it, which measured a place in that track.
        let lookup_ids = current.iter().chain(ids).copied().collect::<Vec<_>>();
        let available = available_on(&mut tx, user_id, &lookup_ids).await?;
        let ids = ids
            .iter()
            .copied()
            .filter(|id| available.contains(id))
            .collect::<Vec<_>>();
        let (current, position_ms) = match current {
            Some(id) if !available.contains(&id) => (None, 0),
            current => (current, position_ms),
        };
        let ids = ids.as_slice();
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

/// When a scrobble says it was played: `time` if it gave one, now otherwise,
/// refused as invalid when it is negative or more than five minutes ahead.
fn play_time(time: Option<i64>) -> Result<i64, ServiceError> {
    const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
    let current_time = now_ms();
    let played = time.unwrap_or(current_time);
    if played < 0 || played > current_time.saturating_add(MAX_FUTURE_SKEW_MS) {
        return Err(ServiceError::Invalid);
    }
    Ok(played)
}

/// Which tracks of `ids` this account can read now, with every id required to
/// be a track of one of its libraries: an id it cannot see, or that never was
/// a track, is refused as not found. One whose file went is a track of the
/// library still, and is answered by being left out of the set.
///
/// Read in one statement rather than a song per id: the queue holds as many
/// tracks as a playlist, and hydrating each one under the writer gate to learn
/// that it exists is what kept the ceiling at four hundred.
async fn available_on(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    ids: &[Uuid],
) -> Result<std::collections::HashSet<Uuid>, ServiceError> {
    if ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let distinct: std::collections::HashSet<&Uuid> = ids.iter().collect();
    let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
    let rows = sqlx::query(
        "SELECT t.id, t.is_available FROM track t JOIN library_member m ON m.library_id=t.library_id \
         WHERE m.user_id=? AND t.id IN (SELECT DISTINCT value FROM json_each(?))",
    )
    .bind(user_id.to_string())
    .bind(ids_json)
    .fetch_all(&mut *connection)
    .await?;
    if rows.len() != distinct.len() {
        return Err(ServiceError::NotFound);
    }
    let mut available = std::collections::HashSet::new();
    for row in rows {
        if row.try_get::<i64, _>("is_available")? == 1 {
            available.insert(parse_uuid(row.try_get("id")?)?);
        }
    }
    Ok(available)
}
