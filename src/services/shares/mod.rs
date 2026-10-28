//! Public shares and their visits.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

/// Whether a share row may still be opened: not expired, below its visit cap,
/// and owned by an account that is not disabled (RFC-023 decision 4) — a
/// disabled account shares nothing, exactly as a missing share does. One
/// spelling for the read that hands out the page and the write that
/// counts a visit, so the two cannot disagree about what "open" means.
macro_rules! share_is_open {
    () => {
        "(s.expires_at IS NULL OR s.expires_at>?) \
         AND (s.max_visits IS NULL OR s.visit_count<s.max_visits) \
         AND EXISTS (SELECT 1 FROM account a \
                     WHERE a.id=s.owner_user_id AND a.disabled=0)"
    };
}

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
        let rows = sqlx::query("SELECT id, description, expires_at, created_at, visit_count, max_visits FROM share WHERE owner_user_id=? ORDER BY created_at DESC")
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
                max_visits: row.try_get("max_visits")?,
                songs: songs_by_share.remove(&id).unwrap_or_default(),
            });
        }
        Ok(shares)
    }

    /// Hands a share its link again, when `user_id` owns it.
    ///
    /// Only the token's hash is stored, but the token itself is derived from
    /// the instance key and the share's id
    /// ([`crate::auth::security::SecretBox::derive_share_token`]), so its
    /// owner can be given it on any read rather than only at creation.
    /// Subsonic's `getShares` needs it there: `url` is required on a `share`,
    /// and DSub crashes on a share without one. The owner already reads every
    /// track the link plays, so the link grants them nothing new. Anyone else
    /// is never handed it. A key rotation re-addresses every share under the
    /// new key, so what this derives is always the address that opens it.
    pub fn reveal_share_link(&self, user_id: Uuid, share: &mut ShareItem) {
        if share.owner_id == user_id {
            share.url_token = Some(self.secret_box.derive_share_token(share.id));
        }
    }

    /// Creates a share from identifiers that may name tracks or albums
    /// (Subsonic `createShare`).
    ///
    /// The specification takes the id "of a song, album or video"; a share
    /// holds tracks only, so an album is expanded here into its available
    /// tracks, in the album's order, and the share is then created from those
    /// tracks by the one implementation every surface uses. An id that is
    /// neither a track nor an album the caller can see answers
    /// [`ServiceError::NotFound`]; more than [`MAX_SHARE_TRACKS`] tracks once
    /// expanded answers [`ServiceError::Invalid`], as too many tracks do.
    pub async fn create_share_of_items(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<ShareItem, ServiceError> {
        if ids.is_empty() || ids.len() > MAX_SHARE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let track_ids = self.share_track_ids(user_id, ids).await?;
        self.create_share(user_id, &track_ids, description, expires_at)
            .await
    }

    /// The tracks `ids` name: a track for itself, an album for its available
    /// tracks in sleeve order — the order [`Self::album`] lists them in.
    async fn share_track_ids(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, ServiceError> {
        let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
        let mut connection = self.db.pool().acquire().await?;
        let tracks = sqlx::query_scalar::<_, String>(
            "SELECT t.id FROM track t JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND t.id IN (SELECT value FROM json_each(?))",
        )
        .bind(user_id.to_string())
        .bind(&ids_json)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<std::collections::HashSet<_>, _>>()?;
        if ids.iter().all(|id| tracks.contains(id)) {
            return Ok(ids.to_vec());
        }
        let album_rows = sqlx::query(
            "SELECT t.album_id, t.id FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND t.album_id IN (SELECT value FROM json_each(?)) \
             ORDER BY t.album_id, t.disc_number NULLS LAST, t.track_number NULLS LAST, \
                      t.title COLLATE NOCASE, t.id",
        )
        .bind(user_id.to_string())
        .bind(&ids_json)
        .fetch_all(&mut *connection)
        .await?;
        let mut albums = HashMap::<Uuid, Vec<Uuid>>::new();
        for row in album_rows {
            albums
                .entry(parse_uuid(row.try_get("album_id")?)?)
                .or_default()
                .push(parse_uuid(row.try_get("id")?)?);
        }
        let mut expanded = Vec::new();
        for id in ids {
            if tracks.contains(id) {
                expanded.push(*id);
            } else {
                // An album with no available track is answered like an id
                // that names nothing: there is nothing in it to share.
                expanded.extend(albums.get(id).ok_or(ServiceError::NotFound)?);
            }
            if expanded.len() > MAX_SHARE_TRACKS {
                return Err(ServiceError::Invalid);
            }
        }
        Ok(expanded)
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
            None,
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
        max_visits: Option<i64>,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        let mut intent_payload = serde_json::json!({
            "track_ids": ids,
            "description": description,
            "expires_at": expires_at,
        });
        // Named only when set. The intent is hashed and compared on replay, so
        // a field named unconditionally would change the hash of every share
        // creation a client retries across this upgrade.
        if let Some(max_visits) = max_visits {
            intent_payload["max_visits"] = max_visits.into();
        }
        let intent = MutationIntent::new("create", "share", &intent_payload);
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
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
        validate_max_visits(max_visits)?;
        let songs = self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        let id = Uuid::new_v4();
        let token = self.secret_box.derive_share_token(id);
        let token_hash = security::token_hash(&token);
        let now = now_ms();
        sqlx::query("INSERT INTO share (id, owner_user_id, token_hash, description, expires_at, max_visits, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(token_hash.as_slice()).bind(description).bind(expires_at).bind(max_visits).bind(now).bind(now).execute(&mut *tx).await?;
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
                    "max_visits": max_visits,
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
            max_visits,
            songs,
        })
    }

    /// Opens a share for an anonymous visitor, and counts the visit.
    ///
    /// **This is the one place a visit is counted** (RFC-020 decision 4). The
    /// streams that follow are not visits: they are redeemed against the pass
    /// this opening is handed, through [`Self::shared_track_owner`], which
    /// reads and never writes. Counting every stream request had a single
    /// listen with three seeks count four visits, and queued each of them
    /// behind the writer gate a scan holds.
    ///
    /// The count and the check are one statement. An expired, deleted or
    /// capped share matches no row, so a visit can neither be counted for a
    /// share that cannot be opened nor slip past a cap between a read and a
    /// write — two visitors racing for the last visit are serialised by the
    /// writer gate, and the second finds the cap reached.
    pub async fn public_share(&self, token: &str) -> Result<ShareItem, ServiceError> {
        let hash = security::token_hash(token);
        let _writer = self.db.writer_guard().await;
        let visited_at = now_ms();
        let row = sqlx::query(concat!(
            "UPDATE share AS s SET visit_count=visit_count+1, last_visited_at=? \
             WHERE s.token_hash=? AND ",
            share_is_open!(),
            " RETURNING id, owner_user_id, description, expires_at, created_at, \
                        visit_count, max_visits"
        ))
        .bind(visited_at)
        .bind(hash.as_slice())
        .bind(visited_at)
        .fetch_optional(self.db.pool())
        .await?;
        drop(_writer);
        let row = row.ok_or(ServiceError::NotFound)?;
        let id = parse_uuid(row.try_get("id")?)?;
        let owner = parse_uuid(row.try_get("owner_user_id")?)?;
        let mut connection = self.db.pool().acquire().await?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM share_track WHERE share_id=? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(ShareItem {
            id,
            owner_id: owner,
            url_token: None,
            description: row.try_get("description")?,
            expires_at: row.try_get("expires_at")?,
            created_at: row.try_get("created_at")?,
            visit_count: row.try_get("visit_count")?,
            max_visits: row.try_get("max_visits")?,
            // Lenient like `shares_on`, and for a stronger reason: the visit is
            // already counted above. A track gone unavailable since the share
            // was created would otherwise answer 404 to the visitor while the
            // owner still sees the share, and the counter would climb anyway.
            songs: self
                .songs_by_ids_lenient_on(&mut connection, owner, &ids)
                .await?,
        })
    }

    /// Whether a share can be opened, without opening it.
    ///
    /// What the share page is served on (RFC-020 decision 3): handing a
    /// browser the player is not a visit — a link preview fetching the page to
    /// draw a card has listened to nothing — so this reads and counts nothing.
    pub async fn share_is_open(&self, token: &str) -> Result<bool, ServiceError> {
        let hash = security::token_hash(token);
        let open = sqlx::query_scalar::<_, i64>(concat!(
            "SELECT 1 FROM share s WHERE s.token_hash=? AND ",
            share_is_open!()
        ))
        .bind(hash.as_slice())
        .bind(now_ms())
        .fetch_optional(self.db.pool())
        .await?;
        Ok(open.is_some())
    }

    /// The account a share's track is streamed as, for one stream request of
    /// a visit already counted.
    ///
    /// **Read-only, and outside the writer gate** (RFC-020 decision 4): a seek
    /// is a range request, and an anonymous listener must not queue behind a
    /// scan to move the playhead. The pass the visit carries names the share;
    /// this re-reads the share on every request, so deleting it, letting it
    /// expire or disabling its owner stops the music at once whatever the pass
    /// says. The cap is not asked again — the visit that reached it was
    /// counted, and keeps its music.
    pub async fn shared_track_owner(
        &self,
        token: &str,
        share_id: Uuid,
        track_id: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let hash = security::token_hash(token);
        let owner = sqlx::query_scalar::<_, String>(
            "SELECT s.owner_user_id FROM share s \
             WHERE s.token_hash=? AND s.id=? AND (s.expires_at IS NULL OR s.expires_at>?) \
               AND EXISTS (SELECT 1 FROM account a \
                           WHERE a.id=s.owner_user_id AND a.disabled=0) \
               AND EXISTS (SELECT 1 FROM share_track st \
                           WHERE st.share_id=s.id AND st.track_id=?)",
        )
        .bind(hash.as_slice())
        .bind(share_id.to_string())
        .bind(now_ms())
        .bind(track_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(parse_uuid(owner)?)
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
            None,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        max_visits: Option<i64>,
        clear: ShareClear,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        // Clearing must be part of the intent: "set expiry to X" and "remove the
        // expiry" are different mutations, and an operation id replayed across
        // both has to be rejected rather than silently treated as the same.
        let mut intent_payload = serde_json::json!({
            "description": description,
            "expires_at": expires_at,
            "clear_description": clear.description,
            "clear_expires_at": clear.expires_at,
        });
        // Named only when set, so the hash of every update made before the cap
        // existed is the hash it had.
        if let Some(max_visits) = max_visits {
            intent_payload["max_visits"] = max_visits.into();
        }
        if clear.max_visits {
            intent_payload["clear_max_visits"] = true.into();
        }
        let intent = MutationIntent::new("update", &format!("share:{id}"), &intent_payload);
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
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
        validate_max_visits(max_visits)?;
        let persisted = sqlx::query(
            "UPDATE share SET \
               description=CASE WHEN ? THEN NULL ELSE COALESCE(?, description) END, \
               expires_at=CASE WHEN ? THEN NULL ELSE COALESCE(?, expires_at) END, \
               max_visits=CASE WHEN ? THEN NULL ELSE COALESCE(?, max_visits) END, \
               updated_at=? \
             WHERE id=? AND owner_user_id=? RETURNING description, expires_at, max_visits",
        )
        .bind(clear.description)
        .bind(description)
        .bind(clear.expires_at)
        .bind(expires_at)
        .bind(clear.max_visits)
        .bind(max_visits)
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
        let persisted_max_visits: Option<i64> = persisted.try_get("max_visits")?;
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
                    "max_visits": persisted_max_visits,
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
        let mut tx = self.db.begin_write(&_writer).await?;
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

/// A visit cap is a positive count. Zero would be a share nobody may open,
/// which is what deleting it is for.
fn validate_max_visits(max_visits: Option<i64>) -> Result<(), ServiceError> {
    match max_visits {
        Some(cap) if cap < 1 => Err(ServiceError::Invalid),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
