//! Playlists and their track lists.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn playlists(&self, user_id: Uuid) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlists_on(&mut connection, user_id).await
    }

    pub(super) async fn playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE owner_user_id=? ORDER BY updated_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        // One query for every playlist's track list and one for the songs they
        // name, rather than two per playlist: an account with fifty playlists
        // was costing a hundred round trips to answer `getPlaylists`.
        let playlist_ids = rows
            .iter()
            .map(|row| parse_uuid(row.try_get("id")?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ordered: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        if !playlist_ids.is_empty() {
            let ids_json =
                serde_json::to_string(&playlist_ids).map_err(|_| ServiceError::Invalid)?;
            for row in sqlx::query(
                "SELECT pt.playlist_id, pt.track_id FROM playlist_track pt                  JOIN playlist p ON p.id=pt.playlist_id                  WHERE p.owner_user_id=? AND p.id IN (SELECT value FROM json_each(?))                  ORDER BY pt.playlist_id, pt.position",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .fetch_all(&mut *connection)
            .await?
            {
                ordered
                    .entry(parse_uuid(row.try_get("playlist_id")?)?)
                    .or_default()
                    .push(parse_uuid(row.try_get("track_id")?)?);
            }
        }
        let mut union: Vec<Uuid> = ordered.values().flatten().copied().collect();
        union.sort_unstable();
        union.dedup();
        // Resolved once for every playlist at once. The per-playlist order and
        // the dropping of a track this account cannot see are reapplied below,
        // exactly as `songs_by_ids_lenient_on` would have applied them.
        let visible = self
            .songs_by_ids_lenient_on(connection, user_id, &union)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut result = Vec::with_capacity(rows.len());
        for (row, id) in rows.into_iter().zip(playlist_ids) {
            result.push(PlaylistItem {
                id,
                name: row.try_get("name")?,
                comment: row.try_get("comment")?,
                public: row.try_get::<i64, _>("public")? != 0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                songs: ordered
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .filter_map(|track| visible.get(track).cloned())
                    .collect(),
            });
        }
        Ok(result)
    }

    pub async fn playlist(&self, user_id: Uuid, id: Uuid) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlist_on(&mut connection, user_id, id).await
    }

    async fn playlist_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PlaylistItem, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE id=? AND owner_user_id=?",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(PlaylistItem {
            id,
            name: row.try_get("name")?,
            comment: row.try_get("comment")?,
            public: row.try_get::<i64, _>("public")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            songs: self.playlist_songs_on(connection, user_id, id).await?,
        })
    }

    async fn playlist_songs_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let ids = self
            .playlist_track_ids_on(connection, user_id, playlist_id)
            .await?;
        self.songs_by_ids_lenient_on(connection, user_id, &ids)
            .await
    }

    async fn playlist_track_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<Uuid>, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT pt.track_id FROM playlist_track pt JOIN playlist p ON p.id=pt.playlist_id \
             WHERE p.id=? AND p.owner_user_id=? ORDER BY pt.position",
        )
        .bind(playlist_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub async fn create_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
    ) -> Result<PlaylistItem, ServiceError> {
        self.create_playlist_with_context(
            user_id,
            name,
            track_ids,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_playlist_with_context(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "playlist",
            &serde_json::json!({ "name": name.trim(), "track_ids": track_ids }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        // Checked after the replay branch and not before it. A replay owes the
        // caller the outcome the original call had, and this ceiling is a
        // policy number rather than a fact of the domain: lower it in a later
        // release and an operation that was valid when it ran would start
        // answering an error to its own retry. Still ahead of every write.
        if track_ids.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        validate_name(name)?;
        self.songs_by_ids_on(&mut tx, user_id, track_ids).await?;
        let id = Uuid::new_v4();
        let now = now_ms();
        sqlx::query("INSERT INTO playlist (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(name.trim()).bind(now).bind(now)
            .execute(&mut *tx).await?;
        replace_playlist_tracks(&mut tx, id, track_ids, now).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.trim(),
                    "track_ids": track_ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
    ) -> Result<PlaylistItem, ServiceError> {
        self.update_playlist_with_context(
            user_id,
            id,
            name,
            comment,
            public,
            add,
            remove_indexes,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut removes = remove_indexes.to_vec();
        removes.sort_unstable_by(|a, b| b.cmp(a));
        removes.dedup();
        let mut intent_payload = serde_json::json!({
            "name": name.map(str::trim),
            "comment": comment,
            "public": public,
            "add": add,
            "remove_indexes": &removes,
            "clear_comment": clear.comment,
        });
        // Added to the payload only when set. The intent is hashed and compared
        // on replay, so naming a new field unconditionally would change the
        // hash of every update this server version ever saw before, and turn a
        // client's retry across an upgrade into a conflict.
        if clear.tracks {
            intent_payload["clear_tracks"] = serde_json::Value::Bool(true);
        }
        let intent = MutationIntent::new("update", &format!("playlist:{id}"), &intent_payload);
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        // A request naming more tracks than a playlist may hold can never be
        // valid, whatever this playlist currently holds, so it is refused
        // before anything is read. After the replay branch for the same reason
        // the create path is: a retry is owed its original outcome.
        if add.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let current = self.playlist_on(&mut tx, user_id, id).await?;
        if let Some(name) = name {
            validate_name(name)?;
        }
        self.songs_by_ids_on(&mut tx, user_id, add).await?;
        let mut ids = if clear.tracks {
            Vec::new()
        } else {
            self.playlist_track_ids_on(&mut tx, user_id, id).await?
        };
        for index in removes {
            if index >= ids.len() {
                return Err(ServiceError::Invalid);
            }
            ids.remove(index);
        }
        ids.extend_from_slice(add);
        // Checked on the result rather than on `add`: an update that adds one
        // track to a playlist already at the ceiling is what has to be refused,
        // and the request that gets there is small.
        if ids.len() > MAX_PLAYLIST_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let changed_at = now_ms();
        sqlx::query(
            "UPDATE playlist SET name=COALESCE(?, name), \
             comment=CASE WHEN ? THEN NULL ELSE COALESCE(?, comment) END, \
             public=COALESCE(?, public), updated_at=? WHERE id=? AND owner_user_id=?",
        )
        .bind(name.map(str::trim))
        .bind(clear.comment)
        .bind(comment)
        .bind(public.map(i64::from))
        .bind(changed_at)
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM playlist_track WHERE playlist_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        replace_playlist_tracks(&mut tx, id, &ids, changed_at).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.map(str::trim).unwrap_or(&current.name),
                    "comment": comment.or(current.comment.as_deref()),
                    "public": public.unwrap_or(current.public),
                    "track_ids": ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    pub async fn delete_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_playlist_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent =
            MutationIntent::new("delete", &format!("playlist:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM playlist WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "playlist",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }
}
