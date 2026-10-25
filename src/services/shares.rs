//! Public shares and their visits.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

/// Whether a share row may still be opened: not expired, and below its visit
/// cap. One spelling for the read that hands out the page and the write that
/// counts a visit, so the two cannot disagree about what "open" means.
macro_rules! share_is_open {
    () => {
        "(s.expires_at IS NULL OR s.expires_at>?) \
         AND (s.max_visits IS NULL OR s.visit_count<s.max_visits)"
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
    /// this re-reads the share on every request, so deleting it or letting it
    /// expire stops the music at once whatever the pass says. The cap is not
    /// asked again — the visit that reached it was counted, and keeps its
    /// music.
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

/// A visit cap is a positive count. Zero would be a share nobody may open,
/// which is what deleting it is for.
fn validate_max_visits(max_visits: Option<i64>) -> Result<(), ServiceError> {
    match max_visits {
        Some(cap) if cap < 1 => Err(ServiceError::Invalid),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    /// A share holding one catalogue row, written straight into the tables:
    /// what this test needs is a share the services can read, not a scan.
    async fn a_share(state: &crate::AppState) -> (String, Uuid, Uuid) {
        let pool = state.db.pool();
        let (account, library, track, share) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        // A placeholder hash, never verified: putting a value through Argon2
        // in a test is what CodeQL reports as a hard-coded credential.
        for (statement, binds) in [
            (
                "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
                 VALUES (?, 'gate-owner', 'placeholder', 'user', 1, 1)",
                vec![account.to_string()],
            ),
            (
                "INSERT INTO library (id, owner_user_id, name, root_path, visibility, \
                                      created_at, updated_at) \
                 VALUES (?, ?, 'Gate', '/music/gate', 'private', 1, 1)",
                vec![library.to_string(), account.to_string()],
            ),
            (
                "INSERT INTO library_member (library_id, user_id, role, created_at) \
                 VALUES (?, ?, 'owner', 1)",
                vec![library.to_string(), account.to_string()],
            ),
            (
                "INSERT INTO track (id, library_id, relative_path, file_size, file_modified_at, \
                                    quick_hash, full_hash, title, duration_ms, is_available, \
                                    created_at, updated_at) \
                 VALUES (?, ?, 'gate.flac', 1, 1, ?, ?, 'Gate', 1000, 1, 1, 1)",
                vec![
                    track.to_string(),
                    library.to_string(),
                    "3".repeat(64),
                    "4".repeat(64),
                ],
            ),
        ] {
            let mut query = sqlx::query(statement);
            for bind in binds {
                query = query.bind(bind);
            }
            query.execute(pool).await.expect(statement);
        }
        let token = state.secret_box.derive_share_token(share);
        sqlx::query(
            "INSERT INTO share (id, owner_user_id, token_hash, created_at, updated_at) \
             VALUES (?, ?, ?, 1, 1)",
        )
        .bind(share.to_string())
        .bind(account.to_string())
        .bind(crate::security::token_hash(&token).as_slice())
        .execute(pool)
        .await
        .expect("share");
        sqlx::query("INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 0)")
            .bind(share.to_string())
            .bind(track.to_string())
            .execute(pool)
            .await
            .expect("share track");
        (token, share, track)
    }

    /// A stream request on a share does not wait for the writer gate
    /// (RFC-020 decision 4).
    ///
    /// The gate is held here the way a scan holds it. The opening of the
    /// share, which counts a visit, is asserted to wait for it — which is
    /// what proves this test can see the gate at all — and the stream lookup
    /// is asserted not to.
    #[tokio::test]
    async fn a_stream_request_on_a_share_does_not_wait_for_the_writer() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let config = crate::Config::for_data_dir(temp.path().join("data"));
        let state = crate::initialize(&config).await.expect("initialize");
        let (token, share, track) = a_share(&state).await;
        let owner = state
            .services
            .public_share(&token)
            .await
            .expect("the share opens")
            .owner_id;

        let held = state.db.writer_guard().await;
        let wait = Duration::from_millis(500);
        assert!(
            tokio::time::timeout(wait, state.services.public_share(&token))
                .await
                .is_err(),
            "opening a share counts a visit, so it must wait for the writer"
        );
        let streamed = tokio::time::timeout(
            wait,
            state.services.shared_track_owner(&token, share, track),
        )
        .await
        .expect("a stream request must not wait for the writer gate")
        .expect("the shared track resolves");
        assert_eq!(streamed, owner);
        drop(held);
    }
}
