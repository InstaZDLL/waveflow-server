//! Public shares and their visits.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    pub async fn shares(&self, user_id: Uuid) -> Result<Vec<ShareItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.shares_on(&mut connection, user_id).await
    }

    pub(super) async fn shares_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<ShareItem>, ServiceError> {
        let rows = sqlx::query("SELECT id, description, expires_at, created_at, visit_count FROM share WHERE owner_user_id=? ORDER BY created_at DESC")
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?;
        let track_rows = sqlx::query(
            "SELECT st.share_id, st.track_id FROM share_track st \
             JOIN share s ON s.id=st.share_id WHERE s.owner_user_id=? \
             ORDER BY st.share_id, st.position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut track_owners = Vec::with_capacity(track_rows.len());
        let mut track_ids = Vec::with_capacity(track_rows.len());
        for track_row in track_rows {
            track_owners.push(parse_uuid(track_row.try_get("share_id")?)?);
            track_ids.push(parse_uuid(track_row.try_get("track_id")?)?);
        }
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, &track_ids)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut songs_by_share = HashMap::<Uuid, Vec<SongItem>>::new();
        for (share_id, track_id) in track_owners.into_iter().zip(track_ids) {
            if let Some(song) = songs.get(&track_id) {
                songs_by_share
                    .entry(share_id)
                    .or_default()
                    .push(song.clone());
            }
        }

        let mut shares = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            shares.push(ShareItem {
                id,
                owner_id: user_id,
                url_token: None,
                description: row.try_get("description")?,
                expires_at: row.try_get("expires_at")?,
                created_at: row.try_get("created_at")?,
                visit_count: row.try_get("visit_count")?,
                songs: songs_by_share.remove(&id).unwrap_or_default(),
            });
        }
        Ok(shares)
    }

    pub async fn create_share(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<ShareItem, ServiceError> {
        self.create_share_with_context(
            user_id,
            ids,
            description,
            expires_at,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_share_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "share",
            &serde_json::json!({
                "track_ids": ids,
                "description": description,
                "expires_at": expires_at,
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
            validate_replay_type(&receipt, "share")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            let mut share = self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound)?;
            share.url_token = Some(self.secret_box.derive_share_token(id));
            return Ok(share);
        }
        if ids.is_empty() || ids.len() > MAX_SHARE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let songs = self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        let id = Uuid::new_v4();
        let token = self.secret_box.derive_share_token(id);
        let token_hash = security::token_hash(&token);
        let now = now_ms();
        sqlx::query("INSERT INTO share (id, owner_user_id, token_hash, description, expires_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(token_hash.as_slice()).bind(description).bind(expires_at).bind(now).bind(now).execute(&mut *tx).await?;
        for (position, track) in ids.iter().enumerate() {
            sqlx::query("INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, ?)")
                .bind(id.to_string())
                .bind(track.to_string())
                .bind(position as i64)
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "track_ids": ids,
                    "description": description,
                    "expires_at": expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        Ok(ShareItem {
            id,
            owner_id: user_id,
            url_token: Some(token),
            description: description.map(str::to_owned),
            expires_at,
            created_at: now,
            visit_count: 0,
            songs,
        })
    }

    pub async fn public_share(&self, token: &str) -> Result<ShareItem, ServiceError> {
        let hash = security::token_hash(token);
        let row = sqlx::query("SELECT id, owner_user_id, description, expires_at, created_at, visit_count FROM share WHERE token_hash=? AND (expires_at IS NULL OR expires_at>?)")
            .bind(hash.as_slice()).bind(now_ms()).fetch_optional(self.db.pool()).await?.ok_or(ServiceError::NotFound)?;
        let id = parse_uuid(row.try_get("id")?)?;
        let owner = parse_uuid(row.try_get("owner_user_id")?)?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM share_track WHERE share_id=? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        // The rows above were read outside the writer gate, and acquiring it can
        // block behind a scan. Re-check both revocation and expiry at write
        // time: no affected row means the share died during that wait, and a
        // visitor must not see what an owner deleted or let expire.
        let _writer = self.db.writer_guard().await;
        let visited_at = now_ms();
        let visited = sqlx::query(
            "UPDATE share SET visit_count=visit_count+1, last_visited_at=? \
             WHERE id=? AND (expires_at IS NULL OR expires_at>?)",
        )
        .bind(visited_at)
        .bind(id.to_string())
        .bind(visited_at)
        .execute(self.db.pool())
        .await?
        .rows_affected();
        drop(_writer);
        if visited == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(ShareItem {
            id,
            owner_id: owner,
            url_token: None,
            description: row.try_get("description")?,
            expires_at: row.try_get("expires_at")?,
            created_at: row.try_get("created_at")?,
            visit_count: row.try_get::<i64, _>("visit_count")? + 1,
            songs: self.songs_by_ids(owner, &ids).await?,
        })
    }

    pub async fn update_share(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
    ) -> Result<ShareItem, ServiceError> {
        self.update_share_with_context(
            user_id,
            id,
            description,
            expires_at,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn update_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        // Clearing must be part of the intent: "set expiry to X" and "remove the
        // expiry" are different mutations, and an operation id replayed across
        // both has to be rejected rather than silently treated as the same.
        let intent = MutationIntent::new(
            "update",
            &format!("share:{id}"),
            &serde_json::json!({
                "description": description,
                "expires_at": expires_at,
                "clear_description": clear.description,
                "clear_expires_at": clear.expires_at,
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
            validate_replay_type(&receipt, "share")?;
            drop(_writer);
            return self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound);
        }
        let persisted = sqlx::query(
            "UPDATE share SET \
               description=CASE WHEN ? THEN NULL ELSE COALESCE(?, description) END, \
               expires_at=CASE WHEN ? THEN NULL ELSE COALESCE(?, expires_at) END, \
               updated_at=? \
             WHERE id=? AND owner_user_id=? RETURNING description, expires_at",
        )
        .bind(clear.description)
        .bind(description)
        .bind(clear.expires_at)
        .bind(expires_at)
        .bind(now_ms())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(persisted) = persisted else {
            tx.rollback().await?;
            return Err(ServiceError::NotFound);
        };
        let persisted_description: Option<String> = persisted.try_get("description")?;
        let persisted_expires_at: Option<i64> = persisted.try_get("expires_at")?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "description": persisted_description,
                    "expires_at": persisted_expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.shares(user_id)
            .await?
            .into_iter()
            .find(|share| share.id == id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_share(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_share_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new("delete", &format!("share:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM share WHERE id=? AND owner_user_id=?")
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
                    "share",
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
